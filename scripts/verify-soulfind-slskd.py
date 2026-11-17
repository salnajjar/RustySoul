#!/usr/bin/env python3
"""Isolated actual Soulfind/slskd/RustySoul containers, without public networking."""
import argparse
import hashlib
import http.client
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from urllib.parse import quote, urlencode

LABEL = 'io.rustysoul.verification=soulfind-slskd'


def run(*command, **kwargs):
    return subprocess.run(command, check=True, capture_output=True, text=True, **kwargs).stdout.strip()


def load_helpers():
    path = Path(__file__).with_name('verify-nicotine-interop.py')
    spec = importlib.util.spec_from_file_location('interop_helpers', path)
    helpers = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helpers)
    return helpers


class ServerRelay:
    """Relay real Soulfind messages, controlling only reachability for each case."""
    def __init__(self, reverse, published):
        self.helpers = load_helpers()
        self.reverse, self.published = reverse, published
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 2243))
        self.listener.listen()
        self.listener.settimeout(.2)
        self.stop = threading.Event()
        self.sockets, self.threads, self.errors = [], [], []
        self.addresses, self.brokered, self.withheld = 0, [], 0
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def copy(self, source, target, outbound):
        exact = self.helpers.exact
        try:
            while not self.stop.is_set():
                header = exact(source, 4)
                length = struct.unpack('<I', header)[0]
                assert 4 <= length <= 16 * 1024 * 1024
                frame = bytearray(exact(source, length))
                code = struct.unpack_from('<I', frame)[0]
                if not outbound and code == 18 and not self.reverse:
                    self.withheld += 1
                    continue
                if outbound and code in (3, 18):
                    username, offset = self.helpers.read_string(frame, 4)
                    if code == 18:
                        kind, offset = self.helpers.read_string(frame, offset)
                        self.brokered.append(kind)
                    else:
                        self.addresses += 1
                    position = offset + 4  # IPv4 precedes port.
                    if code == 3 and self.reverse:
                        struct.pack_into('<I', frame, position, 0)
                    elif username in self.published:
                        struct.pack_into('<I', frame, position, self.published[username])
                target.sendall(header + frame)
        except (OSError, EOFError):
            pass
        except Exception as error:
            self.errors.append(repr(error))
        finally:
            for sock in (source, target):
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    def accept(self):
        while not self.stop.is_set():
            try:
                incoming, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            outgoing = socket.create_connection(('127.0.0.1', 2242), timeout=5)
            outgoing.settimeout(None)
            self.sockets.extend((incoming, outgoing))
            for source, target, outbound in ((incoming, outgoing, False), (outgoing, incoming, True)):
                worker = threading.Thread(target=self.copy, args=(source, target, outbound), daemon=True)
                self.threads.append(worker)
                worker.start()

    def close(self):
        self.stop.set()
        self.listener.close()
        for sock in self.sockets:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            sock.close()
        self.thread.join(2)
        for worker in self.threads:
            worker.join(2)


def inside(args):
    root = Path('/fixture')
    helpers = load_helpers()
    proxies = []
    published = {}
    if args.fragment:
        # Both real clients receive fragmented handshakes; no reference-client
        # coalescing exception is applied to slskd.
        proxies = [helpers.FragmentingProxy(2234), helpers.FragmentingProxy(50300)]
        published = dict(zip(('interop-rust', 'interop-slskd'), (proxy.port for proxy in proxies)))
    relay = ServerRelay(args.mode == 'reverse', published)
    token = None
    checks = []

    def request(client, path, body=None, method=None, retry_until=None):
        connection = http.client.HTTPConnection('127.0.0.1', 8484 if client == 'rust' else 5030, timeout=15)
        headers = {'Content-Type': 'application/json'}
        if client == 'rust' and token:
            headers['Authorization'] = 'Bearer ' + token
        elif client == 'slskd':
            headers['X-API-Key'] = 'rustysoul-isolated-interop-test-key'
        connection.request(method or ('GET' if body is None else 'POST'),
                           ('/api/v1/' if client == 'rust' else '/api/v0/') + path,
                           None if body is None else json.dumps(body), headers)
        response = connection.getresponse()
        data = response.read()
        connection.close()
        if response.status == 429 and body is not None:
            retry_until = retry_until or time.monotonic() + 20
            assert time.monotonic() < retry_until
            time.sleep(.2)
            return request(client, path, body, method, retry_until)
        assert response.status in (200, 201, 202, 204), (client, path, response.status, data[:1500])
        return json.loads(data) if data else None

    def command(kind, **fields):
        return request('rust', 'commands', {'type': kind, **fields})

    def state():
        return request('rust', 'status')

    def wait(predicate, label, seconds=45):
        deadline = time.monotonic() + seconds
        while True:
            assert not relay.errors, relay.errors
            if predicate():
                return
            assert time.monotonic() < deadline, label
            time.sleep(.05)

    def ready():
        try:
            return request('slskd', 'application')['server']['isLoggedIn']
        except (OSError, AssertionError):
            return False

    def network(**fields):
        settings = state()['network']
        settings.update(fields)
        command('update_network', settings=settings)
        wait(lambda: state()['network'] == settings, 'network settings')

    def rust_done(path, status='completed'):
        return any(row['path'] == path and row['status'] == status for row in state()['downloads'])

    def slsk_transfers(kind='downloads'):
        users = request('slskd', 'transfers/' + kind)
        return [file for user in users for directory in user['directories'] for file in directory['files']]

    def slsk_download(path):
        rows = [row for row in slsk_transfers() if row['filename'] == path]
        return rows[-1] if rows else None

    def slsk_queue(path, size):
        return request('slskd', 'transfers/downloads/interop-rust', [{'filename': path, 'size': size}])

    def completed_slsk(path):
        row = slsk_download(path)
        return row and 'Succeeded' in row['state']

    def record(message):
        checks.append(message)
        print(message, flush=True)

    try:
        wait(ready, 'slskd actual Soulfind login', 90)
        token = request('rust', 'auth/login', {'username': 'admin', 'password': 'change-me-now'})['token']
        token = request('rust', 'auth/account', {'current_password': 'change-me-now',
                    'username': 'interop-admin', 'password': 'eightpwd'})['token']
        command('update_storage_reserve', minimum_free_space_mib=0)
        command('update_retry_policy', settings={'enabled': False, 'max_attempts': 3,
                'base_delay_seconds': 15, 'max_delay_seconds': 300})
        command('rescan_shares')
        wait(lambda: state()['shares']['files'] == 4, 'RustySoul library')
        command('update_credentials', username='interop-rust', password='eightpwd')
        wait(lambda: state()['connection'] == 'connected', 'RustySoul actual Soulfind login')
        command('browse', username='interop-slskd')
        wait(lambda: state()['browse'].get('interop-slskd', {}).get('total', 0) >= 3, 'slskd shares')
        rust_listing = request('rust', 'browse/files?' + urlencode({'username': 'interop-slskd'}))['files']
        slsk_listing = request('slskd', 'users/interop-rust/browse')
        if isinstance(slsk_listing, dict):
            slsk_listing = slsk_listing['directories']
        rust_paths = {file['filename']: directory['name'] + '\\' + file['filename']
                      for directory in slsk_listing for file in directory['files']}
        slsk_paths = {row['path'].rsplit('\\', 1)[-1]: row['path'] for row in rust_listing}
        assert set(rust_paths) == {'Café 音楽.bin', 'empty.bin', 'resume.bin', 'queued.bin'}, rust_paths
        assert {'Café 音楽.bin', 'resume.bin', 'queued.bin'} <= set(slsk_paths), slsk_paths
        record('Both real clients logged into Soulfind and browsed Unicode-named shares')

        def exact_download(directory, name):
            files = [path for path in directory.rglob(name) if path.is_file()]
            assert len(files) == 1, files
            expected = (root / 'RustShare' / name).read_bytes()
            assert files[0].read_bytes() == expected

        supported = ['Café 音楽.bin'] + (['empty.bin'] if 'empty.bin' in slsk_paths else [])
        for name in supported:
            size = (root / 'RustShare' / name).stat().st_size
            slsk_queue(rust_paths[name], size)
            command('download', username='interop-slskd', path=slsk_paths[name], size=size)
            wait(lambda: completed_slsk(rust_paths[name]) and rust_done(slsk_paths[name]), 'bidirectional ' + name)
            exact_download(root / 'slsk-downloads', name)
            exact_download(root / 'rust-downloads', name)
        record('Byte-identical bidirectional transfers: ' + ', '.join(supported))

        network(upload_slots=0)
        path = rust_paths['queued.bin']
        slsk_queue(path, (root / 'RustShare/queued.bin').stat().st_size)
        wait(lambda: any(row['path'] == path for row in state()['uploads']), 'RustySoul upload queue')
        position = request('slskd', 'transfers/downloads/interop-rust/' + slsk_download(path)['id'] + '/position')
        assert position > 0, position
        network(upload_slots=1)
        wait(lambda: completed_slsk(path), 'slskd queued completion')
        exact_download(root / 'slsk-downloads', 'queued.bin')
        record('slskd received a real upload queue position and completed after reopening a slot')

        # Save the actual partial length and prove the next negotiation preserves
        # it with byte-identical completion through the independent sender.
        network(download_limit_kib=64)
        path = slsk_paths['resume.bin']
        size = (root / 'SlskShare/resume.bin').stat().st_size
        partials = root / 'rust-downloads/.incomplete'
        command('download', username='interop-slskd', path=path, size=size)
        wait(lambda: any(0 < item.stat().st_size < size for item in partials.rglob('resume.bin.part')),
             'RustySoul partial from slskd')
        command('transfer', username='interop-slskd', path=path, action='pause')
        wait(lambda: rust_done(path, 'paused'), 'RustySoul paused')
        time.sleep(.7)
        prefix = next(partials.rglob('resume.bin.part')).stat().st_size
        command('disconnect')
        wait(lambda: state()['connection'] == 'disconnected', 'RustySoul disconnected')
        # slskd 0.26.2 publishes its terminal transfer state before releasing its
        # per-file task lock. A retry during that interval can be left queued by
        # slskd. Observe the real task's completion before requesting a new one.
        def remote_upload_released():
            log = root / 'slskd-live.log'
            if not log.exists():
                return False
            marker = 'Task for upload of ' + path + ' to interop-rust '
            return any(marker in line and ('completed successfully' in line or
                       'did not complete successfully' in line) for line in log.read_text(errors='replace').splitlines())
        wait(remote_upload_released, 'slskd previous upload task released', 60)
        command('connect')
        wait(lambda: state()['connection'] == 'connected', 'RustySoul reconnect')
        network(download_limit_kib=0)
        command('transfer', username='interop-slskd', path=path, action='resume')
        wait(lambda: rust_done(path), 'RustySoul resumed slskd file')
        exact_download(root / 'rust-downloads', 'resume.bin')
        record('RustySoul pause/reconnect/resume retained a ' + str(prefix) + '-byte prefix from slskd')

        network(upload_limit_kib=64)
        path = rust_paths['resume.bin']
        slsk_queue(path, size)
        wait(lambda: slsk_download(path) and slsk_download(path).get('bytesTransferred', 0) > 0,
             'slskd partial from RustySoul')
        transfer = slsk_download(path)
        request('slskd', 'transfers/downloads/interop-rust/' + transfer['id'], method='DELETE')
        wait(lambda: 'Cancelled' in slsk_download(path)['state'], 'slskd cancellation')
        wait(lambda: any(row['path'] == path and row['status'] in ('failed', 'cancelled', 'completed')
                         for row in state()['uploads']), 'RustySoul previous upload released')
        network(upload_limit_kib=0)
        slsk_queue(path, size)
        wait(lambda: completed_slsk(path), 'slskd resumed completion')
        exact_download(root / 'slsk-downloads', 'resume.bin')
        record('slskd active cancellation/requeue completed with identical bytes')

        if args.mode == 'reverse':
            assert 'P' in relay.brokered and 'F' in relay.brokered, relay.brokered
        else:
            assert not relay.brokered, relay.brokered
        report = {'mode': args.mode, 'fragmented': args.fragment, 'checks': checks,
                  'fragmented_prefix_writes': sum(proxy.fragmented for proxy in proxies),
                  'actual_server_address_replies': relay.addresses, 'brokered_types': relay.brokered,
                  'slskd_version': request('slskd', 'application')['version'],
                  'empty_share_supported_by_slskd': 'empty.bin' in slsk_paths}
        (root / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2), flush=True)
    except Exception:
        if token:
            print('RustySoul status: ' + json.dumps(state()), flush=True)
        try:
            print('slskd downloads: ' + json.dumps(slsk_transfers()), flush=True)
        except Exception:
            pass
        raise
    finally:
        relay.close()
        for proxy in proxies:
            proxy.close()


def host(args):
    assert os.name == 'posix' and os.geteuid() == 0, 'Run on the Linux container host with sudo'
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    image_metadata = json.loads(run('podman', 'image', 'inspect', args.rust_image, args.slskd_image,
                                   args.soulfind_image, args.runner_image, '--format=json'))
    (output / 'images.json').write_text(json.dumps(image_metadata, indent=2))
    cases = [(args.mode or 'direct', args.fragment)] if args.mode or args.sdk else [
        ('direct', False), ('reverse', False), ('direct', True), ('reverse', True)]
    for mode, fragmented in cases:
        name = 'rustysoul-interop-' + uuid.uuid4().hex[:12]
        containers = []
        followers = []
        network_created = pod_created = False
        with tempfile.TemporaryDirectory(prefix=name + '-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            os.chown(root, 10001, 10001)
            def directory(name):
                result = root / name
                result.mkdir(mode=0o700)
                os.chown(result, 10001, 10001)
                return result
            for child in ['soulfind', 'rust-data', 'RustShare', 'SlskShare', 'rust-downloads',
                          'slsk-downloads', 'slsk-incomplete', 'slskd-app']:
                directory(child)
            for share in ['RustShare', 'SlskShare']:
                for filename, content in [('Café 音楽.bin', bytes(range(251)) * 2048),
                       ('empty.bin', b''), ('resume.bin', bytes(range(251)) * 16384),
                       ('queued.bin', bytes(range(251)) * 1024)]:
                    (root / share / filename).write_bytes(content)
            shutil.copyfile(__file__, root / 'verify-soulfind-slskd.py')
            shutil.copyfile(Path(__file__).with_name('verify-nicotine-interop.py'), root / 'verify-nicotine-interop.py')
            (root / 'rustysoul.toml').write_text('bind="127.0.0.1:8484"\ndata_dir="/var/lib/rustysoul"\n'
                'download_dir="/downloads"\nlocation_roots=["/downloads","/RustShare"]\n'
                '[soulseek]\nserver="127.0.0.1"\nport=2243\nlisten_port=2234\n'
                'shares=["/RustShare"]\nauto_connect=false\n')
            options = {'flags': {'no_version_check': True}, 'remote_configuration': False,
                'directories': {'incomplete': '/incomplete', 'downloads': '/downloads'},
                'shares': {'directories': ['/SlskShare'], 'cache': {'workers': 1}},
                'web': {'port': 5030, 'ip_address': '127.0.0.1', 'https': {'disabled': True},
                        'authentication': {'api_keys': {'fixture': {'key': 'rustysoul-isolated-interop-test-key',
                        'role': 'administrator', 'cidr': '127.0.0.1/32'}}}},
                'soulseek': {'address': '127.0.0.1', 'port': 2243, 'username': 'interop-slskd',
                    'password': 'eightpwd', 'listen_ip_address': '127.0.0.1', 'listen_port': 50300,
                    'distributed_network': {'disabled': True}},
                'transfers': {'upload': {'slots': 1, 'speed_limit': 100000},
                              'download': {'speed_limit': 100000}}}
            (root / 'slskd-app/slskd.yml').write_text(json.dumps(options, indent=2))
            case = ('sdk-' + args.sdk_test if args.sdk_test else 'sdk') if args.sdk else mode + ('-fragmented' if fragmented else '')
            log = (output / (case + '.log')).open('w')
            try:
                run('podman', 'network', 'create', '--internal', '--label', LABEL, name)
                network_created = True
                run('podman', 'pod', 'create', '--name', name, '--label', LABEL, '--network', name)
                pod_created = True
                def container(role, image, mounts, command=(), entrypoint=None):
                    container_name = name + '-' + role
                    arguments = ['podman', 'run', '-d', '--name', container_name, '--pod', name,
                        '--label', LABEL, '--user', '10001:10001', '--cap-drop', 'ALL',
                        '--security-opt', 'no-new-privileges', '--read-only', '--memory', '512m',
                        '--pids-limit', '256', '--tmpfs', '/tmp:rw,size=128m,mode=1777']
                    for mount in mounts:
                        arguments.extend(['-v', mount])
                    if entrypoint:
                        arguments.extend(['--entrypoint', entrypoint])
                    if role == 'slskd':
                        arguments.extend(['--env', 'DOTNET_BUNDLE_EXTRACT_BASE_DIR=/tmp/dotnet'])
                    if role == 'runner' and args.sdk:
                        # The host's container AppArmor profile denies the SDK
                        # harness killing/reaping its disposable Soulfind child.
                        # Only this isolated test runner needs subprocess signals.
                        arguments.extend(['--security-opt', 'apparmor=unconfined'])
                    run(*arguments, image, *command)
                    containers.append(container_name)
                    return container_name
                controller = container('runner', args.runner_image, [f'{root}:/fixture'],
                        ('-c', 'import time; time.sleep(86400)'), 'python3')
                infra = run('podman', 'pod', 'inspect', name, '--format={{.InfraContainerID}}')
                namespace_pid = run('podman', 'inspect', infra, '--format={{.State.Pid}}')
                # Soulfind's address probe is routed to loopback only. No default
                # route is added to the internal test network.
                assert int(namespace_pid) > 1
                run('nsenter', '-t', namespace_pid, '-n', 'ip', 'route', 'add',
                    '10.255.255.255/32', 'dev', 'lo', 'src', '127.0.0.1')
                assert not run('nsenter', '-t', namespace_pid, '-n', 'ip', 'route', 'show', 'default')
                container('soulfind', args.soulfind_image, [f'{root / "soulfind"}:/data'])
                if args.sdk:
                    assert args.sdk.is_file()
                    shutil.copyfile(args.sdk, root / 'sdk-e2e')
                    (root / 'sdk-e2e').chmod(0o755)
                    run('podman', 'cp', containers[-1] + ':/bin/soulfind', str(root / 'soulfind-bin'))
                    (root / 'soulfind-bin').chmod(0o755)
                    command = ['podman', 'exec', '-e', 'SOULFIND_BIN=/fixture/soulfind-bin',
                        '-e', 'SOULSEEK_E2E_REQUIRED=1', controller, '/fixture/sdk-e2e', '--nocapture', '--test-threads=1']
                    if args.sdk_test:
                        command.extend(['--exact', args.sdk_test])
                else:
                    container('rust', args.rust_image, [f'{root / "rust-data"}:/var/lib/rustysoul',
                        f'{root / "RustShare"}:/RustShare:ro', f'{root / "rust-downloads"}:/downloads',
                        f'{root / "rustysoul.toml"}:/etc/rustysoul/rustysoul.toml:ro'])
                    slskd = container('slskd', args.slskd_image, [f'{root / "slskd-app"}:/app',
                        f'{root / "SlskShare"}:/SlskShare:ro', f'{root / "slsk-downloads"}:/downloads',
                        f'{root / "slsk-incomplete"}:/incomplete'])
                    stream = (root / 'slskd-live.log').open('w')
                    follower = subprocess.Popen(['podman', 'logs', '--follow', slskd], stdout=stream, stderr=subprocess.STDOUT)
                    followers.append((follower, stream))
                    command = ['podman', 'exec', controller, 'python3', '/fixture/verify-soulfind-slskd.py',
                               '--inside', '--mode', mode, *(['--fragment'] if fragmented else [])]
                result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=1800 if args.sdk else 300)
                log.flush()
                assert result.returncode == 0, f'{case} failed; see {log.name}'
                if args.sdk:
                    text = Path(log.name).read_text()
                    expected = '1 passed; 0 failed' if args.sdk_test else '87 passed; 0 failed'
                    assert 'e2e skipped' not in text and expected in text
                else:
                    shutil.copyfile(root / 'result.json', output / (case + '.json'))
                print('Passed ' + case, flush=True)
            finally:
                for follower, stream in followers:
                    follower.terminate()
                    try:
                        follower.wait(5)
                    except subprocess.TimeoutExpired:
                        follower.kill()
                        follower.wait(5)
                    stream.close()
                for container_name in containers:
                    logs = subprocess.run(['podman', 'logs', container_name], capture_output=True, text=True)
                    (output / (case + '-' + container_name.rsplit('-', 1)[-1] + '.log')).write_text(logs.stdout + logs.stderr)
                if pod_created:
                    run('podman', 'pod', 'rm', '--force', name)
                if network_created:
                    run('podman', 'network', 'rm', name)
                log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('--mode', choices=['direct', 'reverse'])
    parser.add_argument('--fragment', action='store_true')
    parser.add_argument('--sdk', type=Path, help='Run required-server SDK tests with Soulfind from the official image')
    parser.add_argument('--sdk-test', help='Run one exact SDK server case instead of the full suite')
    parser.add_argument('--rust-image', default='ghcr.io/salnajjar/rustysoul:beta')
    parser.add_argument('--slskd-image', default='docker.io/slskd/slskd:latest')
    parser.add_argument('--soulfind-image', default='ghcr.io/soulfind-dev/soulfind:latest')
    parser.add_argument('--runner-image', default='localhost/rustysoul:interop-runner')
    parser.add_argument('--output', type=Path, default=Path('soulfind-slskd-results'))
    args = parser.parse_args()
    inside(args) if args.inside else host(args)


if __name__ == '__main__':
    main()
