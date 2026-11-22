#!/usr/bin/env python3
"""Isolated functional installation checks; never uses production data or Soulseek."""
import argparse
import http.client
import json
import os
from pathlib import Path
import socket
import sqlite3
import struct
import subprocess
import tempfile
import threading
import time
import uuid
from urllib.parse import urlencode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', choices=['docker', 'podman'], required=True)
    parser.add_argument('--image', required=True)
    parser.add_argument('--network', choices=['host', 'bridge'], required=True)
    parser.add_argument('--storage', choices=['named', 'bind'], default='named')
    parser.add_argument('--quadlet', type=Path)
    parser.add_argument('--bind-data',action='store_true',help='Also bind the private database directory (POSIX filesystems only)')
    parser.add_argument('--daemon-bind',action='store_true',help='Create isolated Linux bind sources in a daemon-owned fixture volume')
    parser.add_argument('--env-only',action='store_true',help='Configure with environment variables only; mount no TOML files')
    args = parser.parse_args()
    if args.bind_data:
        assert args.storage=='bind'
    if args.quadlet:
        assert args.engine == 'podman' and args.network == 'host' and args.storage == 'named'
        assert not args.env_only
    label = 'io.rustysoul.verification=container-matrix'
    name = 'rustysoul-matrix-' + uuid.uuid4().hex[:12]
    volumes = []
    unit = None
    token = None
    bind_parent=None

    def run(*command, input=None, check=True):
        return subprocess.run(command, input=input, capture_output=True, text=True, check=check).stdout.strip()

    def engine(*command, **kwargs):
        return run(args.engine, *command, **kwargs)

    rootless = args.engine == 'podman' and json.loads(engine('info', '--format', 'json'))['host']['security']['rootless']
    assert not args.quadlet or rootless

    def port():
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            return sock.getsockname()[1]

    http_port, peer_port = port(), port()
    listener = socket.socket()
    public_mock = args.network == 'bridge' and args.engine == 'docker' and os.name != 'nt'
    listener.bind(('0.0.0.0' if public_mock else '127.0.0.1', 0))
    listener.listen()
    listener.settimeout(.2)
    stop = threading.Event()
    sockets = []

    def exact(sock, length):
        data = bytearray()
        while len(data) < length:
            block = sock.recv(length - len(data))
            if not block:
                raise EOFError()
            data.extend(block)
        return bytes(data)

    def string(text):
        encoded = text.encode()
        return struct.pack('<I', len(encoded)) + encoded

    def send(sock, code, body=b''):
        sock.sendall(struct.pack('<II', len(body) + 4, code) + body)

    def receive(sock, expected):
        while True:
            length = struct.unpack('<I', exact(sock, 4))[0]
            assert 4 <= length <= 65536
            frame = exact(sock, length)
            if struct.unpack('<I', frame[:4])[0] == expected:
                return frame[4:]

    def serve_connection(sock):
        sockets.append(sock)
        try:
            sock.settimeout(30)
            while not stop.is_set():
                length = struct.unpack('<I', exact(sock, 4))[0]
                assert 4 <= length <= 8192
                frame = exact(sock, length)
                code = struct.unpack('<I', frame[:4])[0]
                if code == 1:
                    send(sock, 1, b'\x01' + string('Local matrix fixture') + bytes([1, 0, 0, 127]) + string('checksum') + b'\x00')
                elif code == 92:
                    send(sock, 92, struct.pack('<I', 0))
        except (OSError, EOFError):
            pass
        finally:
            sock.close()

    def serve():
        while not stop.is_set():
            try:
                sock, _ = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            threading.Thread(target=serve_connection, args=(sock,), daemon=True).start()

    threading.Thread(target=serve, daemon=True).start()

    def request(path, body=None, expected=(200, 202)):
        connection = http.client.HTTPConnection('127.0.0.1', http_port, timeout=10)
        headers = {'Content-Type': 'application/json'}
        if token:
            headers['Authorization'] = 'Bearer ' + token
        connection.request('GET' if body is None else 'POST', '/api/v1/' + path, None if body is None else json.dumps(body), headers)
        response = connection.getresponse()
        result = json.loads(response.read())
        connection.close()
        assert response.status in expected, (path, response.status, result)
        return result

    def wait(predicate, seconds=20):
        end = time.monotonic() + seconds
        while True:
            try:
                state = request('status')
                if predicate(state):
                    return state
            except OSError:
                pass
            assert time.monotonic() < end, 'Container state did not become ready'
            time.sleep(.1)

    def init_peer(kind):
        sock = socket.create_connection(('127.0.0.1', peer_port), timeout=5)
        sockets.append(sock)
        body = b'\x01' + string('fixture-peer') + string(kind) + struct.pack('<I', 0)
        sock.sendall(struct.pack('<I', len(body)) + body)
        return sock

    with tempfile.TemporaryDirectory(prefix='rustysoul-container-matrix-') as temporary:
        root = Path(temporary)
        try:
            config = root / 'rustysoul.toml'
            server = '127.0.0.1' if args.network == 'host' else '10.0.2.2' if args.engine == 'podman' else 'host.docker.internal'
            if public_mock:
                network=json.loads(engine('network','inspect','bridge'))[0]
                server=next(c['Gateway'] for c in network['IPAM']['Config'] if ':' not in c.get('Gateway',''))
            internal_http = http_port if args.network == 'host' else 8484
            config.write_text(f'bind="{"127.0.0.1" if args.network == "host" else "0.0.0.0"}:{internal_http}"\nallow_remote=true\ndata_dir="/var/lib/rustysoul"\ndownload_dir="/downloads"\nlocation_roots=["/downloads","/library"]\n[soulseek]\nserver="{server}"\nport={listener.getsockname()[1]}\nlisten_port={peer_port}\nauto_connect=false\n', encoding='utf-8')
            sources = {}
            bind_parent=None
            if args.daemon_bind:
                assert args.engine=='docker' and args.storage=='bind'
                bind_parent=name+'-bind-root';engine('volume','create','--label',label,bind_parent);volumes.append(bind_parent)
                engine('run','--rm','--label',label,'--user','0:0','--entrypoint','sh','-v',bind_parent+':/fixture',args.image,'-c','mkdir /fixture/data /fixture/downloads /fixture/library; chown 10001:10001 /fixture/data /fixture/downloads /fixture/library; chmod 700 /fixture/data /fixture/downloads /fixture/library')
                bind_point=json.loads(engine('volume','inspect',bind_parent))[0]['Mountpoint']
                assert Path(bind_point).is_absolute() and '..' not in Path(bind_point).parts
            for purpose in ['data', 'downloads', 'library'] + ([] if args.env_only else ['config']):
                if args.storage == 'bind' and (purpose in ['downloads','library'] or (args.bind_data and purpose=='data')):
                    if bind_parent:
                        sources[purpose]=str(Path(bind_point)/purpose)
                    else:
                        directory = root / purpose
                        directory.mkdir()
                        if rootless:
                            engine('unshare', 'chown', '10001:10001', str(directory))
                            engine('unshare','chmod','700',str(directory))
                        else:
                            engine('run','--rm','--label',label,'--user','0:0','--entrypoint','sh','-v',str(directory)+':/prepare',args.image,'-c','chown 10001:10001 /prepare; chmod 700 /prepare')
                        sources[purpose] = str(directory)
                else:
                    volume = name + '-' + purpose
                    engine('volume', 'create', '--label', label, volume)
                    volumes.append(volume)
                    sources[purpose] = volume
            # Install-time config contains only the loopback fixture address.
            if not args.env_only:
                engine('run', '--rm', '-i', '--label', label, '--user', '0:0', '--entrypoint', 'sh', '-v', sources['config'] + ':/config', args.image, '-c', 'cat > /config/rustysoul.toml; chmod 644 /config/rustysoul.toml', input=config.read_text())
            engine('run', '--rm', '--label', label, '--entrypoint', 'sh', '-v', sources['library'] + ':/library', args.image, '-c', 'printf fLaC > /library/matrix.flac; : > /library/empty.txt')
            mounts = [sources['data'] + ':/var/lib/rustysoul', sources['downloads'] + ':/downloads', sources['library'] + ':/library:ro']
            if not args.env_only:
                mounts.append(sources['config'] + ':/etc/rustysoul:ro')
            extra_env = ['RUSTYSOUL_DEFAULT_USERNAME=matrix-bootstrap', 'RUSTYSOUL_DEFAULT_PASSWORD=eightpwd']
            if args.env_only:
                extra_env += [f'RUSTYSOUL_BIND={"127.0.0.1" if args.network=="host" else "0.0.0.0"}:{internal_http}', 'RUSTYSOUL_ALLOW_REMOTE=true', 'RUSTYSOUL_DATA_DIR=/var/lib/rustysoul', 'RUSTYSOUL_DOWNLOAD_DIR=/downloads', 'RUSTYSOUL_LIBRARY_DIRS=[]', 'RUSTYSOUL_LOCATION_ROOTS=["/downloads","/library"]', f'RUSTYSOUL_SOULSEEK_SERVER={server}',f'RUSTYSOUL_SOULSEEK_PORT={listener.getsockname()[1]}',f'RUSTYSOUL_TRANSFER_PORT={peer_port}','RUSTYSOUL_AUTO_CONNECT=false']
            if args.network == 'bridge':
                extra_env.append(f'RUSTYSOUL_PUBLISHED_TRANSFER_PORT={peer_port}')
            if args.quadlet:
                unit_dir = Path.home() / '.config/containers/systemd'
                unit_dir.mkdir(parents=True, exist_ok=True)
                unit = unit_dir / (name + '.container')
                assert not unit.exists()
                env = root / 'daemon.env'
                env.write_text('\n'.join(extra_env) + '\n')
                env.chmod(0o600)
                text = args.quadlet.read_text()
                text = text.replace('Image=localhost/rustysoul:local', 'Image=' + args.image).replace('ContainerName=rustysoul', 'ContainerName=' + name)
                for purpose in ['data', 'downloads', 'library']:
                    text = text.replace('rustysoul-' + purpose + ':', sources[purpose] + ':')
                text = text.replace('%h/.config/rustysoul/rustysoul.toml', str(config)).replace('%h/.config/rustysoul/daemon.env', str(env))
                text = text.replace('--pids-limit=256 --stop-timeout=90', '--pids-limit=128 --memory=512m --stop-timeout=20')
                unit.write_text(text)
                run('systemctl', '--user', 'daemon-reload')
                run('systemctl', '--user', 'start', name + '.service')
            else:
                command = ['run', '-d', '--name', name, '--label', label, '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--pids-limit', '128', '--memory', '512m', '--stop-timeout', '20']
                if args.network == 'host':
                    command += ['--network', 'host']
                else:
                    command += ['--network', 'slirp4netns:allow_host_loopback=true' if args.engine == 'podman' else 'bridge', '-p', f'127.0.0.1:{http_port}:8484', '-p', f'127.0.0.1:{peer_port}:{peer_port}']
                    if args.engine == 'docker' and os.name != 'nt':
                        command += ['--add-host', 'host.docker.internal:host-gateway']
                for mount in mounts:
                    command += ['-v', mount]
                for value in extra_env:
                    command += ['-e', value]
                command += [args.image]
                engine(*command)
            end = time.monotonic() + 25
            while True:
                try:
                    login = request('auth/login', {'username': 'matrix-bootstrap', 'password': 'eightpwd'})
                    break
                except OSError:
                    assert time.monotonic() < end
                    time.sleep(.1)
            token = login['token']
            assert login['account']['must_change']
            request('status', expected=(403,))
            token = request('auth/account', {'current_password': 'eightpwd', 'username': 'matrix-admin', 'password': 'new-eight-pwd'})['token']
            request('commands', {'type': 'update_locations', 'downloads': '/downloads', 'shares': ['/library']})
            state = wait(lambda state: state['library']['indexed'] == 2 and not state['library']['scanning'])
            assert state['network']['port'] == peer_port
            assert state['port_binding']['fixed_port'] == (peer_port if args.network == 'bridge' else None)
            settings = state['network']
            # Storage protection has dedicated tests. Installation/transport
            # fixtures must not depend on the build runner's disk budget.
            settings['minimum_free_space_mib'] = 0
            settings['connect_on_startup'] = True
            settings['upload_slots'] = 3
            request('commands', {'type': 'update_network', 'settings': settings})
            wait(lambda state: state['network'] == settings)
            request('commands', {'type': 'update_credentials', 'username': 'matrix-user', 'password': 'eightpwd'})
            wait(lambda state: state['connection'] == 'connected')
            peer = init_peer('P')
            send(peer, 15)
            receive(peer, 16)
            remote = r'Album\empty.txt'
            request('commands', {'type': 'download', 'username': 'fixture-peer', 'path': remote, 'size': 0})
            assert receive(peer, 43) == string(remote)
            send(peer, 40, struct.pack('<II', 1, 12345) + string(remote) + struct.pack('<Q', 0))
            assert receive(peer, 41)[:5] == struct.pack('<I', 12345) + b'\x01'
            file = init_peer('F')
            file.sendall(struct.pack('<I', 12345))
            assert struct.unpack('<Q', exact(file, 8))[0] == 0
            assert file.recv(1) == b''
            wait(lambda state: any(t['path'] == remote and t['status'] == 'completed' for t in state['downloads']))
            for sock in [peer, file]:
                sock.close()
            assert engine('exec', name, 'id', '-u') == '10001'
            engine('exec', name, 'sh', '-c', 'test -w /downloads && test ! -w /library')
            inspection = json.loads(engine('inspect', name))[0]
            assert inspection['HostConfig']['ReadonlyRootfs']
            assert inspection['Config'].get('Healthcheck'), 'Runtime image lost its health check'
            if args.env_only:
                assert all(not mount['Destination'].startswith('/etc/rustysoul') for mount in inspection['Mounts'])
            if args.quadlet:
                run('systemctl', '--user', 'restart', name + '.service')
            else:
                engine('restart', name)
            wait(lambda state: state['connection'] == 'connected' and state['library']['indexed'] == 2)
            assert request('auth/me')['username'] == 'matrix-admin'
            assert request('status')['network'] == settings
            connection = http.client.HTTPConnection('127.0.0.1', http_port, timeout=5)
            connection.request('GET', '/api/v1/downloads/file?' + urlencode({'username': 'fixture-peer', 'path': remote}), headers={'Authorization': 'Bearer ' + token})
            response = connection.getresponse()
            assert response.status == 200 and response.read() == b''
            connection.close()
            health=inspection['Config']['Healthcheck']['Test']
            assert health[0] in ['CMD','CMD-SHELL']
            engine('exec',name,*(['sh','-c',health[1]] if health[0]=='CMD-SHELL' else health[1:]))
            if args.engine=='podman':engine('healthcheck','run',name)
            if args.quadlet:
                run('systemctl', '--user', 'stop', name + '.service')
                mountpoint = json.loads(engine('volume', 'inspect', sources['data']))[0]['Mountpoint']
                with (root / 'state.sqlite3').open('wb') as output:
                    subprocess.run([args.engine, 'unshare', 'cat', str(Path(mountpoint) / 'rustysoul.sqlite3')], stdout=output, check=True)
            else:
                engine('stop', name)
                engine('cp', name + ':/var/lib/rustysoul/rustysoul.sqlite3', str(root / 'state.sqlite3'))
            from contextlib import closing
            with closing(sqlite3.connect(root / 'state.sqlite3')) as db:
                assert db.execute('PRAGMA user_version').fetchone()[0] == 18
                assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
                assert not db.execute('PRAGMA foreign_key_check').fetchall()
                for table in ['downloads', 'uploads']:
                    assert {'started_at', 'finished_at'} <= {row[1] for row in db.execute('PRAGMA table_info('+table+')')}
                started, finished = db.execute('SELECT started_at,finished_at FROM downloads WHERE username=? AND path=?', ('fixture-peer', remote)).fetchone()
                assert started is not None and finished is not None and finished >= started
            print(f'{args.engine} rootless={rootless} {args.network}/{args.storage} bind_data={args.bind_data} env_only={args.env_only} quadlet={bool(args.quadlet)}: initial account gate, UID/mount permissions, indexing, incoming peer/file connection, empty download, health and restart/session/credentials/settings/media/database persistence passed.', flush=True)
        except Exception:
            import sys
            log=subprocess.run([args.engine,'logs','--tail','20',name],capture_output=True,text=True);print(log.stdout+log.stderr,file=sys.stderr)
            raise
        finally:
            if unit:
                run('systemctl', '--user', 'stop', name + '.service', check=False)
                unit.unlink(missing_ok=True)
                run('systemctl', '--user', 'daemon-reload')
                run('systemctl', '--user', 'reset-failed', name + '.service', check=False)
            engine('rm', '-f', name, check=False)
            for volume in volumes:
                engine('volume', 'rm', volume)
            stop.set()
            listener.close()
            for sock in sockets:
                try:
                    sock.close()
                except OSError:
                    pass
            if rootless and args.storage == 'bind':
                assert root.resolve().parent == Path(tempfile.gettempdir()).resolve() and root.name.startswith('rustysoul-container-matrix-')
                engine('unshare', 'chown', '-R', '--no-dereference', '0:0', str(root))
            elif args.engine=='docker' and args.storage=='bind' and os.name!='nt' and not bind_parent:
                assert root.resolve().parent==Path(tempfile.gettempdir()).resolve() and root.name.startswith('rustysoul-container-matrix-')
                engine('run','--rm','--label',label,'--user','0:0','--entrypoint','sh','-v',str(root)+':/fixture',args.image,'-c',f'chown -R --no-dereference {os.getuid()}:{os.getgid()} /fixture')


if __name__ == '__main__':
    main()
