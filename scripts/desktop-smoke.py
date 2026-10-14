"""Real helper startup/handshake/shutdown on a synthetic desktop; missing synthetic NSS database, no real token/PIN access."""
import base64
import json
import os
from pathlib import Path
import re
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

binary = str(Path(sys.argv[1]).resolve())

def receive(stream):
    header = stream.read(2)
    assert len(header) == 2
    length = header[1] & 127
    if length == 126:
        length = struct.unpack('!H', stream.read(2))[0]
    elif length == 127:
        length = struct.unpack('!Q', stream.read(8))[0]
    return json.loads(stream.read(length))

def send(sock, value):
    body = json.dumps(value).encode()
    mask = os.urandom(4)
    prefix = bytes([0x81, 0x80 | len(body)]) if len(body) < 126 else bytes([0x81, 0xfe]) + struct.pack('!H', len(body))
    sock.sendall(prefix + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(body)))

for run in range(3):
    with tempfile.TemporaryDirectory(prefix='epoxy-desktop-smoke-') as directory:
        env = dict(os.environ, XDG_RUNTIME_DIR=directory)
        logfile = Path(directory) / 'stderr.log'
        with logfile.open('wb') as output:
            os.chmod(logfile, 0o600)
            start = time.monotonic()
            process = subprocess.Popen([binary, '--config-path', str(Path(directory) / 'absent.toml'),
                '--selection-path', str(Path(directory) / 'app/selection.json'),
                '--nssdb-path', str(Path(directory) / 'absent-synthetic-nss-db'), '--endpoint', '127.0.0.1:0',
                '--pin-prompt', 'ui', '--log', 'debug'] + (['--show-window'] if run else []),
                env=env, stdout=subprocess.DEVNULL, stderr=output)
            observed = []
            def observe_window():
                while process.poll() is None:
                    if 'desktop window presented' in logfile.read_text():
                        observed.append((time.monotonic() - start) * 1000)
                        return
                    time.sleep(.005)
            observer = threading.Thread(target=observe_window)
            observer.start()
            try:
                deadline = start + 8
                address = None
                while time.monotonic() < deadline:
                    match = re.search(r'listening on (127\.0\.0\.1):(\d+)', logfile.read_text())
                    if match:
                        address = (match[1], int(match[2]))
                        break
                    assert process.poll() is None, 'helper exited during startup'
                    time.sleep(.005)
                assert address, 'listener did not become ready'
                ready_ms = (time.monotonic() - start) * 1000
                def control(*args):
                    completed = subprocess.run([binary, 'diagnostics', *args], env=env, capture_output=True, text=True, timeout=3, check=True)
                    return json.loads(completed.stdout)
                # Exercise the actual application's guard before its admitted protocol flow.
                with socket.create_connection(address, timeout=2) as denied:
                    key = base64.b64encode(os.urandom(16)).decode()
                    denied.sendall(f'GET / HTTP/1.1\r\nHost: {address[0]}:{address[1]}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nOrigin: https://synthetic-origin-secret.evil.example\r\nSec-WebSocket-Key: {key}\r\n\r\n'.encode())
                    data = b''
                    while True:
                        block = denied.recv(4096)
                        if not block:
                            break
                        data += block
                    assert data.startswith(b'HTTP/1.1 403'), data
                    assert b'ON_OPEN_EVENT' not in data and b'synthetic-origin-secret' not in data
                with socket.create_connection(address, timeout=2) as sock:
                    stream = sock.makefile('rb')
                    key = base64.b64encode(os.urandom(16)).decode()
                    sock.sendall(f'GET / HTTP/1.1\r\nHost: {address[0]}:{address[1]}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nOrigin: https://test.purs.gov.rs\r\nSec-WebSocket-Key: {key}\r\n\r\n'.encode())
                    assert b'101' in stream.readline()
                    while stream.readline() != b'\r\n':
                        pass
                    assert receive(stream)['operation'] == 'ON_OPEN_EVENT'
                    info = {'operation': 'GET_INFO', 'input': {'sbSession': 'synthetic-secret', 'host': 'synthetic-host', 'language': 'sr', 'pin': 'synthetic-pin'}}
                    send(sock, info)
                    assert receive(stream)['status'] == 0
                    handshake_ms = (time.monotonic() - start) * 1000
                    # This must complete before the missing-tray fallback interval.
                    assert handshake_ms < 2000, 'handshake waited for tray/UI'
                    assert not control('status')['requests_masked']
                    assert control('enable', '--level', 'debug', '--requests', 'masked', '--ttl', '1')['requests_masked']
                    send(sock, info)
                    assert receive(stream)['status'] == 0
                    time.sleep(1.2)
                    assert not control('status')['requests_masked']
                    while not observed and time.monotonic() < deadline:
                        time.sleep(.02)
                    assert observed, 'no GTK control window/fallback presentation'
                    window_ms = observed[0]
                    # Leave connection open: service shutdown must still close it and clean controls.
                    process.terminate()
                    process.wait(timeout=4)
                    stream.close()
                assert process.returncode == 0, logfile.read_text()
                assert not (Path(directory) / 'epoxy/control.sock').exists()
                contents = logfile.read_text()
                for secret in ['synthetic-secret', 'synthetic-host', 'synthetic-pin', 'synthetic-origin-secret']:
                    assert secret not in contents
                assert 'origin_unapproved' in contents
                assert 'NSS initialized' not in contents
                scans = [line for line in contents.splitlines()
                         if 'certificate control received' in line and 'certificate_refresh' in line]
                assert len(scans) == 1, 'desktop startup must discover certificates once without a user/portal request'
                assert contents.index('listening on') < contents.index('certificate control received')
                assert 'service stopped' in contents
                print(f'PASS run {run + 1}: readiness={ready_ms:.1f}ms handshake={handshake_ms:.1f}ms; first presentation request={window_ms:.1f}ms; clean SIGTERM; automatic discovery attempted; no real token/PIN access')
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=3)
                observer.join(timeout=1)
