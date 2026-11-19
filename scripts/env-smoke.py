#!/usr/bin/env python3
"""Check native environment-only startup without credentials or external networking."""
import argparse
import http.client
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
args = parser.parse_args()
binary = args.binary.resolve()
with tempfile.TemporaryDirectory(prefix='rustysoul-env-smoke-') as temporary:
    root = Path(temporary)
    (root / 'library').mkdir()
    (root / 'downloads').mkdir()
    with socket.socket() as reservation:
        reservation.bind(('127.0.0.1', 0))
        port = reservation.getsockname()[1]
    env = {k:v for k,v in os.environ.items() if not k.startswith(('RUSTYSOUL_', 'SOULSEEK_'))}
    env.update(RUSTYSOUL_BIND=f'127.0.0.1:{port}', RUSTYSOUL_DATA_DIR=str(root/'state'), RUSTYSOUL_DOWNLOAD_DIR=str(root/'downloads'), RUSTYSOUL_LIBRARY_DIRS=str(root/'library'), RUSTYSOUL_LOCATION_ROOTS=str(root), RUSTYSOUL_AUTO_CONNECT='false')
    subprocess.run([binary, '--check'], cwd=root, env=env, check=True)
    assert not (root/'rustysoul.toml').exists()
    missing = subprocess.run([binary, '--config', root/'missing.toml', '--check'], cwd=root, env=env, capture_output=True)
    assert missing.returncode != 0
    invalid = subprocess.run([binary, '--check'], cwd=root, env={**env,'RUSTYSOUL_ALLOW_REMOTE':'invalid'}, capture_output=True)
    assert invalid.returncode != 0 and b'RUSTYSOUL_ALLOW_REMOTE' in invalid.stderr
    with (root/'daemon.log').open('w+') as log:
        for _ in range(2):
            process = subprocess.Popen([binary], cwd=root, env=env, stdout=log, stderr=log)
            try:
                deadline = time.monotonic()+20
                while True:
                    assert process.poll() is None, 'Daemon exited; inspect the environment-only startup'
                    try:
                        connection = http.client.HTTPConnection('127.0.0.1',port,timeout=1)
                        connection.request('GET','/health')
                        response=connection.getresponse()
                        assert response.status==200
                        response.read();connection.close()
                        break
                    except OSError:
                        assert time.monotonic()<deadline
                        time.sleep(.1)
                subprocess.run([binary,'--healthcheck'],cwd=root,env=env,check=True)
            finally:
                process.terminate()
                process.wait(timeout=20)
                assert process.returncode==0
    with sqlite3.connect(root/'state/rustysoul.sqlite3') as db:
        assert db.execute('PRAGMA user_version').fetchone()[0]==16
        assert db.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
    print('Native environment-only validation, HTTP binding, health, private storage and restart passed without a TOML file.')
