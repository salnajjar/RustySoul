#!/usr/bin/env python3
"""Run the unmodified Nicotine+ core against a compiled RustySoul on loopback.

Only the central rendezvous server is synthetic. Peer framing, browse, transfer
negotiation and file I/O use each client's actual implementation. The external source tree
is imported for verification only; no Nicotine+ code is shipped in RustySoul.
"""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlencode


def exact(sock, count):
    result = bytearray()
    while len(result) < count:
        block = sock.recv(count - len(result))
        if not block:
            raise EOFError()
        result.extend(block)
    return bytes(result)


def string(value):
    encoded = value.encode()
    return struct.pack('<I', len(encoded)) + encoded


def read_string(body, offset=0):
    count = struct.unpack_from('<I', body, offset)[0]
    end = offset + 4 + count
    assert end <= len(body)
    return body[offset + 4:end].decode(), end


def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class Rendezvous:
    """Two local synthetic accounts; never forwards to an external address."""
    def __init__(self, reverse):
        self.reverse = reverse
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen()
        self.listener.settimeout(.2)
        self.address = self.listener.getsockname()
        self.clients = {}
        self.ports = {}
        self.published_ports = {}
        self.counts = {}
        self.sockets = []
        self.threads = []
        self.errors = []
        self.brokered = []
        self.address_requests = []
        self.lock = threading.RLock()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def send(self, sock, code, body=b''):
        with self.lock:
            sock.sendall(struct.pack('<II', len(body) + 4, code) + body)

    def accept(self):
        while not self.stop.is_set():
            try:
                sock, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            self.sockets.append(sock)
            thread = threading.Thread(target=self.client, args=(sock,), daemon=True)
            self.threads.append(thread)
            thread.start()

    def client(self, sock):
        username = None
        try:
            while not self.stop.is_set():
                count = struct.unpack('<I', exact(sock, 4))[0]
                assert 4 <= count <= 65536
                frame = exact(sock, count)
                code, body = struct.unpack_from('<I', frame)[0], frame[4:]
                if code == 1:
                    username, _ = read_string(body)
                    assert username in ('interop-rust', 'interop-nicotine')
                    with self.lock:
                        self.clients[username] = sock
                    self.send(sock, 1, b'\x01' + string('Local interoperability fixture')
                              + bytes([1, 0, 0, 127]) + string('fixture') + b'\x00')
                elif code == 2:
                    self.ports[username] = struct.unpack_from('<I', body)[0]
                elif code == 3:
                    target, _ = read_string(body)
                    self.address_requests.append((username, target))
                    target_port = 0 if self.reverse else self.published_ports.get(target, self.ports.get(target, 0))
                    self.send(sock, 3, string(target) + bytes([1, 0, 0, 127])
                              + struct.pack('<IIH', target_port, 0, 0))
                elif code == 18:
                    token = struct.unpack_from('<I', body)[0]
                    target, offset = read_string(body, 4)
                    kind, _ = read_string(body, offset)
                    assert kind in ('P', 'F', 'D')
                    self.brokered.append((username, target, kind))
                    if not self.reverse:
                        # Nicotine+ races direct and brokered requests. Withhold
                        # the latter here to prove the direct path on its own.
                        continue
                    peer = self.clients.get(target)
                    if peer is None:
                        self.send(sock, 1001, struct.pack('<I', token) + string(target))
                        continue
                    self.send(peer, 18, string(username) + string(kind) + bytes([1, 0, 0, 127])
                              + struct.pack('<II', self.published_ports.get(username, self.ports[username]), token) + b'\x00'
                              + struct.pack('<II', 0, 0))
                elif code == 35:
                    self.counts[username] = struct.unpack_from('<II', body)
                elif code == 5:
                    target, _ = read_string(body)
                    folders, files = self.counts.get(target, (1, 4))
                    self.send(sock, 5, string(target) + b'\x01'
                              + struct.pack('<IIIIII', 2, 1024 * 1024, 0, 0, files, folders)
                              + string(''))
                elif code == 7:
                    target, _ = read_string(body)
                    self.send(sock, 7, string(target) + struct.pack('<I', 2) + b'\x00')
                elif code == 36:
                    target, _ = read_string(body)
                    folders, files = self.counts.get(target, (1, 4))
                    self.send(sock, 36, string(target)
                              + struct.pack('<IIIII', 1024 * 1024, 0, 0, files, folders))
                elif code == 92:
                    self.send(sock, 92, struct.pack('<I', 0))
                elif code == 64:
                    self.send(sock, 64, bytes(16))
        except (EOFError, ConnectionError, OSError):
            pass
        except Exception as error:
            self.errors.append(repr(error))
        finally:
            with self.lock:
                if self.clients.get(username) is sock:
                    self.clients.pop(username, None)
            sock.close()

    def close(self):
        self.stop.set()
        self.listener.close()
        for sock in self.sockets:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(2)
        for thread in self.threads:
            thread.join(2)


class FragmentingProxy:
    """Delay/fragment each real peer stream's prefix, retaining half-close behavior."""
    def __init__(self, destination, preserve_incoming_init=False):
        self.destination = destination
        self.preserve_incoming_init = preserve_incoming_init
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen()
        self.listener.settimeout(.2)
        self.port = self.listener.getsockname()[1]
        self.stop = threading.Event()
        self.sockets = []
        self.workers = []
        self.fragmented = 0
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def copy(self, source, destination, preserve_init=False):
        prefix = 512
        try:
            if preserve_init:
                # The supplied Nicotine+ snapshot closes an incomplete PeerInit
                # in _process_peer_init_input. Keep just that frame together
                # toward Nicotine+; RustySoul's incoming init remains fragmented.
                header = exact(source, 4)
                count = struct.unpack('<I', header)[0]
                assert 1 <= count <= 16384
                destination.sendall(header + exact(source, count))
            while not self.stop.is_set():
                block = source.recv(65536)
                if not block:
                    break
                fragmented = min(prefix, len(block))
                for offset in range(0, fragmented, 7):
                    destination.sendall(block[offset:min(offset + 7, fragmented)])
                    self.fragmented += 1
                    time.sleep(.001)
                destination.sendall(block[fragmented:])
                prefix -= fragmented
        except (OSError, EOFError):
            # A failed forward is a broken transport, not merely a half-close.
            # Close both legs so a real uploader promptly observes cancellation
            # instead of filling a proxy buffer and waiting for its stall timer.
            for sock in (source, destination):
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
        finally:
            try:
                destination.shutdown(socket.SHUT_WR)
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
            try:
                outgoing = socket.create_connection(('127.0.0.1', self.destination), timeout=5)
                outgoing.settimeout(None)
            except OSError:
                incoming.close()
                continue
            incoming.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            outgoing.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.sockets.extend((incoming, outgoing))
            for source, destination in ((incoming, outgoing), (outgoing, incoming)):
                worker = threading.Thread(target=self.copy,
                        args=(source, destination, self.preserve_incoming_init and source is incoming), daemon=True)
                self.workers.append(worker)
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
        for worker in self.workers:
            worker.join(2)


def run_case(args):
    sys.path.insert(0, str(args.nicotine.resolve()))
    from pynicotine import __version__ as nicotine_version
    from pynicotine.config import config
    from pynicotine.core import core
    from pynicotine.events import events
    from pynicotine.i18n import apply_translations
    from pynicotine.logfacility import log
    from pynicotine.slskmessages import FileOffset, PlaceInQueueRequest
    from pynicotine.transfers import TransferStatus

    apply_translations()
    # Fail closed if the imported client ever tries contacting a public service.
    original_connect_ex = socket.socket.connect_ex
    def local_connect_ex(sock, address):
        assert address[0] in ('127.0.0.1', '::1'), address
        return original_connect_ex(sock, address)
    socket.socket.connect_ex = local_connect_ex

    central = Rendezvous(args.mode == 'reverse')
    daemon = None
    token = None
    checks = []
    upload_offsets = []
    download_offsets = []
    nic_started = False
    proxies = []
    logs = []
    def log_message(_timestamp, message, *_rest):
        logs.append(message)
    events.connect('log-message', log_message)

    with tempfile.TemporaryDirectory(prefix='rustysoul-nicotine-interop-') as temporary:
        root = Path(temporary)
        rust_share, nic_share = root / 'RustShare', root / 'NicShare'
        rust_downloads, nic_downloads = root / 'rust-downloads', root / 'nic-downloads'
        for directory in (rust_share, nic_share, rust_downloads, nic_downloads, root / 'rust-data', root / 'nic-data'):
            directory.mkdir(mode=0o700)
        small = b'fLaC' + bytes(range(251)) * 2048
        large = bytes(range(251)) * 16384
        files = {'CafÃ© éŸ³æ¥½.bin': small, 'empty.bin': b'', 'resume.bin': large, 'queued.bin': small}
        for directory in (rust_share, nic_share):
            for name, payload in files.items():
                (directory / name).write_bytes(payload)
        http_port, rust_port, nic_port = port(), port(), port()
        if args.fragment:
            proxies = [FragmentingProxy(rust_port), FragmentingProxy(nic_port, preserve_incoming_init=True)]
            central.published_ports = dict(zip(('interop-rust', 'interop-nicotine'), (p.port for p in proxies)))
        conf = root / 'rustysoul.toml'
        conf.write_text(f'bind="127.0.0.1:{http_port}"\ndata_dir={json.dumps(str(root / "rust-data"))}\n'
                        f'download_dir={json.dumps(str(rust_downloads))}\nlocation_roots=[{json.dumps(str(root))}]\n'
                        f'[soulseek]\nserver="127.0.0.1"\nport={central.address[1]}\nlisten_port={rust_port}\n'
                        f'shares=[{json.dumps(str(rust_share))}]\nauto_connect=false\n')

        def request(path, body=None, retry_until=None):
            connection = http.client.HTTPConnection('127.0.0.1', http_port, timeout=5)
            headers = {'Content-Type': 'application/json'}
            if token:
                headers['Authorization'] = 'Bearer ' + token
            connection.request('GET' if body is None else 'POST', path if path.startswith('/') else '/api/v1/' + path,
                               None if body is None else json.dumps(body), headers)
            response = connection.getresponse()
            data = json.loads(response.read())
            connection.close()
            if response.status == 429 and body is not None:
                retry_until = retry_until or time.monotonic() + 15
                assert time.monotonic() < retry_until, (path, 'command rate limit did not clear')
                # Respect application admission rather than disabling its limit
                # for an artificially fast scripted administrator.
                events.process_thread_events()
                time.sleep(.15)
                return request(path, body, retry_until)
            assert response.status in (200, 202), (path, response.status, data)
            return data

        def wait(predicate, label, seconds=20):
            deadline = time.monotonic() + seconds
            while True:
                events.process_thread_events()
                assert not central.errors, central.errors
                if daemon is not None:
                    assert daemon.poll() is None, 'RustySoul exited'
                if predicate():
                    return
                assert time.monotonic() < deadline, label
                time.sleep(.02)

        def command(kind, **fields):
            return request('commands', {'type': kind, **fields})

        def state():
            return request('status')

        def rust_transfer(path, status):
            return any(row['path'] == path and row['status'] == status for row in state()['downloads'])

        def nic_transfer(path):
            return core.downloads.transfers['interop-rust' + path]

        def network(**fields):
            settings = state()['network']
            settings.update(fields)
            command('update_network', settings=settings)
            wait(lambda: state()['network'] == settings, 'network settings')

        output = (root / 'daemon.log').open('w+')
        try:
            environment = {key: value for key, value in os.environ.items() if not key.startswith('RUSTYSOUL_')}
            environment.update(RUSTYSOUL_DEFAULT_USERNAME='admin', RUSTYSOUL_DEFAULT_PASSWORD='change-me-now')
            daemon = subprocess.Popen([str(args.binary.resolve()), '--config', str(conf)],
                                      stdout=output, stderr=subprocess.STDOUT, env=environment)
            def started():
                try:
                    request('/health')
                    return True
                except (OSError, AssertionError):
                    return False
            wait(started, 'HTTP startup')
            token = request('auth/login', {'username': 'admin', 'password': 'change-me-now'})['token']
            token = request('auth/account', {'current_password': 'change-me-now',
                                             'username': 'interop-admin', 'password': 'eightpwd'})['token']
            command('update_storage_reserve', minimum_free_space_mib=0)
            command('update_retry_policy', settings={'enabled': False, 'max_attempts': 3,
                    'base_delay_seconds': 15, 'max_delay_seconds': 300})
            command('rescan_shares')
            wait(lambda: state()['shares']['files'] == len(files), 'RustySoul index')

            config.set_data_folder(str(root / 'nic-data'))
            config.set_config_file(str(root / 'nic-data' / 'config'))
            core.init_components(enabled_components={'network_thread', 'shares', 'users', 'network_filter',
                    'statistics', 'downloads', 'uploads', 'userbrowse', 'userinfo', 'buddies', 'pluginhandler',
                    'notifications'},
                    isolated_mode=True)
            config.sections['server'].update(server=central.address, login='interop-nicotine', passw='eightpwd',
                    portrange=(nic_port, nic_port), upnp=False, auto_connect_startup=False)
            config.sections['transfers'].update(shared=[('NicShare', str(nic_share))],
                    downloaddir=str(nic_downloads), incompletedir=str(root / 'nic-incomplete'),
                    rescanonstartup=True, rescan_shares_daily=False, uploadslots=1)
            config.sections['plugins']['enable'] = False
            config.sections['words']['commands'] = False
            config.sections['logging']['debugmodes'] = ['upload', 'download', 'transfer', 'connection']
            log.init_log_levels()
            def observe_upload(**fields):
                if fields.get('offset') is not None:
                    upload_offsets.append(fields['offset'])
            def observe_message(message):
                if isinstance(message, FileOffset):
                    download_offsets.append(message.offset)
            events.connect('file-upload-progress', observe_upload)
            events.connect('queue-network-message', observe_message)
            core.cli_interface_address = '127.0.0.1'
            core.start()
            nic_started = True
            wait(lambda: core.shares.initialized and not core.shares.rescanning, 'Nicotine+ index', 60)
            core.connect()
            command('update_credentials', username='interop-rust', password='eightpwd')
            wait(lambda: state()['connection'] == 'connected' and core.users.login_username == 'interop-nicotine'
                 and len(central.ports) == 2, 'two local network logins')

            command('browse', username='interop-nicotine')
            core.userbrowse.browse_user('interop-rust')
            wait(lambda: state()['browse'].get('interop-nicotine', {}).get('total') == len(files)
                 and core.userbrowse.users['interop-rust'].num_files == len(files), 'bidirectional browse', 40)
            rust_listing = request('browse/files?' + urlencode({'username': 'interop-nicotine'}))['files']
            assert {row['path']: row['size'] for row in rust_listing} == {
                'NicShare\\' + name: len(payload) for name, payload in files.items()}
            nic_listing = core.userbrowse.users['interop-rust'].public_folders
            rust_paths = {entry.path: folder + '\\' + entry.path
                          for folder, entries in nic_listing.items() for entry in entries}
            assert set(rust_paths) == set(files)
            checks.append('bidirectional real-client browse, UTF-8 names and zero-byte files')

            for name in ('CafÃ© éŸ³æ¥½.bin', 'empty.bin'):
                core.downloads.enqueue_download('interop-rust', rust_paths[name], size=len(files[name]))
                command('download', username='interop-nicotine', path='NicShare\\' + name, size=len(files[name]))
                wait(lambda: nic_transfer(rust_paths[name]).status == TransferStatus.FINISHED
                     and rust_transfer('NicShare\\' + name, 'completed'), 'bidirectional download ' + name, 40)
                assert (nic_downloads / name).read_bytes() == files[name]
                assert (rust_downloads / 'interop-nicotine' / 'NicShare' / name).read_bytes() == files[name]
            wait(lambda: any(row['path'] == rust_paths['empty.bin'] and row['status'] == 'completed'
                             for row in state()['uploads']), 'empty upload recorded completed')
            checks.append('bidirectional byte-identical transfers and empty-file completion')

            # Pause upload admission, inspect a real queue reply, then reopen slots.
            network(upload_slots=0)
            queued = rust_paths['queued.bin']
            core.downloads.enqueue_download('interop-rust', queued, size=len(small))
            wait(lambda: any(row['path'] == queued for row in state()['uploads']), 'RustySoul upload queue')
            core.send_message_to_peer('interop-rust', PlaceInQueueRequest(queued))
            wait(lambda: nic_transfer(queued).queue_position > 0, 'Nicotine+ queue position')
            network(upload_slots=1)
            wait(lambda: nic_transfer(queued).status == TransferStatus.FINISHED, 'queued upload resumes')
            assert (nic_downloads / 'queued.bin').read_bytes() == small
            checks.append('real queue-position response and slot reopening')

            # Pause a real receiver after file I/O, reconnect, then resume at
            # exactly the retained prefix observed by the independent sender.
            network(download_limit_kib=64)
            resume_path = 'NicShare\\resume.bin'
            digest = hashlib.sha256(('interop-nicotine\0' + resume_path).encode()).hexdigest()
            partial = rust_downloads / '.incomplete' / digest / 'resume.bin.part'
            command('download', username='interop-nicotine', path=resume_path, size=len(large))
            wait(lambda: partial.exists() and 0 < partial.stat().st_size < len(large), 'partial RustySoul download')
            command('transfer', username='interop-nicotine', path=resume_path, action='pause')
            wait(lambda: rust_transfer(resume_path, 'paused'), 'download paused')
            # Let the worker release its locked file before recording the prefix.
            time.sleep(.7)
            prefix = partial.stat().st_size
            assert 0 < prefix < len(large)
            command('disconnect')
            wait(lambda: state()['connection'] == 'disconnected', 'RustySoul disconnect')
            command('connect')
            wait(lambda: state()['connection'] == 'connected', 'RustySoul reconnect')
            network(download_limit_kib=0)
            first_offset = len(upload_offsets)
            command('transfer', username='interop-nicotine', path=resume_path, action='resume')
            wait(lambda: rust_transfer(resume_path, 'completed'), 'RustySoul resumed completion', 40)
            assert prefix in upload_offsets[first_offset:], (prefix, upload_offsets[first_offset:])
            assert (rust_downloads / 'interop-nicotine' / 'NicShare' / 'resume.bin').read_bytes() == large
            checks.append('RustySoul pause/reconnect/resume with exact Nicotine+ observed offset')

            # Exercise the opposite resume direction through Nicotine+'s actual
            # pause/retry controls and outgoing FileOffset message.
            network(upload_limit_kib=64)
            nic_path = rust_paths['resume.bin']
            core.downloads.enqueue_download('interop-rust', nic_path, size=len(large))
            nic_partial = Path(core.downloads.get_incomplete_download_file_path('interop-rust', nic_path))
            wait(lambda: nic_partial.exists() and 0 < nic_partial.stat().st_size < len(large),
                 'partial Nicotine+ download')
            nic_resume = nic_transfer(nic_path)
            core.downloads.abort_downloads({nic_resume: None})
            wait(lambda: nic_resume.status == TransferStatus.PAUSED, 'Nicotine+ paused')
            prefix = nic_partial.stat().st_size
            assert 0 < prefix < len(large)
            network(upload_limit_kib=0)
            first_offset = len(download_offsets)
            core.downloads.retry_download(nic_resume)
            wait(lambda: nic_resume.status == TransferStatus.FINISHED, 'Nicotine+ resumed completion', 40)
            assert prefix in download_offsets[first_offset:], (prefix, download_offsets[first_offset:])
            assert (nic_downloads / 'resume.bin').read_bytes() == large
            checks.append('Nicotine+ pause/retry with exact outgoing resume offset')

            # Repeated cancellation/retry at the real queued-request boundary.
            # Zero download slots holds admission without a timing-dependent
            # large payload; only our synthetic transfer is cancelled.
            cancel_path = 'NicShare\\queued.bin'
            network(download_slots=0)
            command('download', username='interop-nicotine', path=cancel_path, size=len(small))
            for _ in range(3):
                wait(lambda: rust_transfer(cancel_path, 'waiting'), 'queued retry')
                command('cancel_downloads', username='interop-nicotine', paths=[cancel_path])
                wait(lambda: rust_transfer(cancel_path, 'cancelled'), 'cancelled queued download')
                command('transfer', username='interop-nicotine', path=cancel_path, action='resume')
            network(download_slots=1)
            wait(lambda: rust_transfer(cancel_path, 'completed'), 'completion after repeated cancellation', 40)
            assert (rust_downloads / 'interop-nicotine' / 'NicShare' / 'queued.bin').read_bytes() == small
            checks.append('three real queued cancellation/retry cycles and final exact completion')

            assert central.address_requests
            if args.mode == 'reverse':
                assert any(kind == 'P' for _, _, kind in central.brokered), central.brokered
                assert any(kind == 'F' for _, _, kind in central.brokered), central.brokered
            report = {'mode': args.mode, 'nicotine_version': nicotine_version, 'checks': checks,
                      'fragmented_prefix_writes': sum(proxy.fragmented for proxy in proxies),
                      'brokered_requests': central.brokered,
                      'brokered_requests_forwarded': len(central.brokered) if central.reverse else 0,
                      'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest()}
            print(json.dumps(report, indent=2))
            return report
        except Exception:
            if token and daemon is not None and daemon.poll() is None:
                print('RustySoul transfer state: ' + json.dumps(state()['downloads']), file=sys.stderr)
            if core.downloads is not None:
                print('Nicotine+ download state: ' + repr([(t.virtual_path, t.status)
                      for t in core.downloads.transfers.values()]), file=sys.stderr)
            if core.uploads is not None:
                print('Nicotine+ upload state: ' + repr([(t.virtual_path, t.status)
                      for t in core.uploads.transfers.values()]), file=sys.stderr)
            output.flush()
            output.seek(0)
            print('RustySoul fixture log:\n' + output.read()[-12000:], file=sys.stderr)
            print('Nicotine+ fixture log:\n' + '\n'.join(str(line) for line in logs[-40:]), file=sys.stderr)
            raise
        finally:
            if nic_started:
                core.quit()
                deadline = time.monotonic() + 5
                while core._network_thread.is_alive() and time.monotonic() < deadline:
                    events.process_thread_events()
                    time.sleep(.02)
                events.process_thread_events()
                core._network_thread.join(2)
            if daemon is not None:
                daemon.terminate()
                try:
                    daemon.wait(10)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait(5)
            central.close()
            for proxy in proxies:
                proxy.close()
            output.close()
            socket.socket.connect_ex = original_connect_ex


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--nicotine', type=Path, default=Path('../nicotine-plus'))
    parser.add_argument('--mode', choices=['direct', 'reverse'])
    parser.add_argument('--fragment', action='store_true', help='Delay and fragment peer-stream prefixes through loopback proxies')
    args = parser.parse_args()
    assert args.binary.is_file()
    assert (args.nicotine / 'pynicotine/slskproto.py').is_file()
    if args.mode:
        run_case(args)
    else:
        for mode in ('direct', 'reverse'):
            subprocess.run([sys.executable, __file__, '--binary', str(args.binary),
                            '--nicotine', str(args.nicotine), '--mode', mode,
                            *(['--fragment'] if args.fragment else [])], check=True, timeout=180)


if __name__ == '__main__':
    main()
