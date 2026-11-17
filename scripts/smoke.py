#!/usr/bin/env python3
"""Exercise the compiled Linux daemon without credentials or public network traffic."""
import argparse
import http.client
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="rustysoul-smoke-") as temp:
        root = Path(temp)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        config = root / "config.toml"
        config.write_text(f'bind = "127.0.0.1:{port}"\ndata_dir = "{root / "data"}"\n')
        env = dict(os.environ)
        env.pop("SOULSEEK_USERNAME", None)
        env.pop("SOULSEEK_PASSWORD", None)
        env.pop("RUSTYSOUL_API_TOKEN", None)
        subprocess.run([binary, "--config", config, "--check"], env=env, check=True)
        with (root / "daemon.log").open("w+") as log:
            process = subprocess.Popen([binary, "--config", config], env=env, stdout=log, stderr=log)
            stream = None
            try:
                deadline = time.monotonic() + 10
                while True:
                    try:
                        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                        conn.request("GET", "/")
                        response = conn.getresponse()
                        assert response.status == 200
                        assert b"RustySoul" in response.read()
                        conn.close()
                        break
                    except (OSError, http.client.HTTPException):
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.05)
                conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                conn.request("GET", "/api/v1/status")
                response = conn.getresponse()
                assert response.status == 401
                response.read()
                def auth_request(path, body, bearer=None):
                    request_headers = {"Content-Type": "application/json"}
                    if bearer:
                        request_headers["Authorization"] = f"Bearer {bearer}"
                    conn.request("POST", "/api/v1/" + path, json.dumps(body), request_headers)
                    result = conn.getresponse()
                    return result.status, json.loads(result.read())
                status_code, session = auth_request("auth/login", {"username": "admin", "password": "change-me-now"})
                assert status_code == 200 and session["account"]["must_change"]
                old_token = session["token"]
                conn.request("GET", "/api/v1/status", headers={"Authorization": f"Bearer {old_token}"})
                response = conn.getresponse()
                assert response.status == 403
                response.read()
                status_code, session = auth_request("auth/account", {"username":"smoke-admin", "current_password":"change-me-now", "password":"smoke-new-password"}, old_token)
                assert status_code == 200 and not session["account"]["must_change"]
                token = session["token"]
                headers = {"Authorization": f"Bearer {token}"}
                conn.request("GET", "/api/v1/status", headers=headers)
                response = conn.getresponse()
                assert response.status == 200
                assert json.loads(response.read())["connection"] == "disconnected"
                subprocess.run([binary, "--config", config, "--healthcheck"], env=env, check=True)
                conn.request("POST", "/api/v1/commands", json.dumps({"type": "connect"}), {**headers, "Content-Type": "application/json"})
                response = conn.getresponse()
                assert response.status == 202
                response.read()
                stream = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
                stream.request("GET", "/api/v1/events", headers=headers)
                events = stream.getresponse()
                assert events.status == 200
                assert b"event: state" in events.readline()
                status = Path(f"/proc/{process.pid}/status").read_text()
                memory = next(line for line in status.splitlines() if line.startswith("VmRSS:"))
                print(f"Disconnected daemon memory snapshot: {memory}")
                started = time.monotonic()
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=10)
                assert process.returncode == 0
                print(f"Clean SIGTERM shutdown with open SSE client: {time.monotonic() - started:.3f}s")
                log.seek(0)
                assert token not in log.read(), "API token leaked into logs"
                assert (root / "data" / "rustysoul.sqlite3").exists()
                print("Compiled-daemon smoke checks passed")
            finally:
                if stream:
                    stream.close()
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    main()
