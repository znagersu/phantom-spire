"""Mock GitHub Releases HTTP server for Phantom Spire updater E2E verification.
Invoked via `py.exe` (Python stdlib only, zero third-party dependencies).
"""
import argparse
import hashlib
import io
import json
import os
import sys
import threading
import time
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse


def build_mock_release_zip(version: str) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        vst3_payload = (f"PHANTOM_SPIRE_MOCK_VST3_BINARY_V{version}\n" * 64).encode("utf-8")
        exe_payload = (f"PHANTOM_SPIRE_MOCK_EXE_BINARY_V{version}\n" * 64).encode("utf-8")
        zf.writestr("phantom_spire.vst3/Contents/x86_64-win/phantom_spire.vst3", vst3_payload)
        zf.writestr("phantom_spire.exe", exe_payload)
    return buf.getvalue()


class MockGitHubState:
    def __init__(self, version: str):
        self.version = version
        self.zip_bytes = build_mock_release_zip(version)
        self.zip_sha256 = hashlib.sha256(self.zip_bytes).hexdigest()
        self.base_url = ""
        self.requests_seen = []
        self.lock = threading.Lock()


def make_handler(state: MockGitHubState):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            pass

        def _send(self, code: int, body: bytes, content_type: str = "application/json"):
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            parsed = urlparse(self.path)
            path = parsed.path
            with state.lock:
                state.requests_seen.append(self.path)

            zip_url = f"{state.base_url}/PhantomSpire-win64.zip"

            if path == "/health":
                self._send(200, json.dumps({"ok": True, "requests": len(state.requests_seen)}).encode("utf-8"))
            elif path == "/shutdown":
                self._send(200, b'{"shutting_down":true}')
                threading.Thread(target=self.server.shutdown, daemon=True).start()
            elif path == "/PhantomSpire-win64.zip":
                self._send(200, state.zip_bytes, "application/zip")
            elif path == "/latest.json":
                payload = {
                    "version": state.version,
                    "url": zip_url,
                    "zip_url": zip_url,
                    "sha256": state.zip_sha256,
                    "notes": f"Mock Phantom Spire v{state.version}",
                }
                self._send(200, json.dumps(payload).encode("utf-8"))
            elif path == "/latest_bom.json":
                payload = {
                    "version": state.version,
                    "url": zip_url,
                    "zip_url": zip_url,
                    "sha256": state.zip_sha256,
                    "notes": "UTF-8 BOM manifest test",
                }
                self._send(200, b"\xef\xbb\xbf" + json.dumps(payload).encode("utf-8"))
            elif path == "/latest_older.json":
                payload = {
                    "version": "0.0.1",
                    "url": zip_url,
                    "zip_url": zip_url,
                    "sha256": state.zip_sha256,
                }
                self._send(200, json.dumps(payload).encode("utf-8"))
            elif path == "/latest_prerelease.json":
                payload = {
                    "version": f"{state.version}-beta.1",
                    "url": zip_url,
                    "zip_url": zip_url,
                    "sha256": state.zip_sha256,
                }
                self._send(200, json.dumps(payload).encode("utf-8"))
            elif path == "/latest_malformed.json":
                self._send(200, b"{this is definitely not valid json!!!")
            elif path == "/latest_untrusted.json":
                payload = {
                    "version": state.version,
                    "url": "https://untrusted.example.invalid/PhantomSpire-win64.zip",
                    "zip_url": "https://untrusted.example.invalid/PhantomSpire-win64.zip",
                    "sha256": state.zip_sha256,
                }
                self._send(200, json.dumps(payload).encode("utf-8"))
            elif path == "/latest_badsha.json":
                payload = {
                    "version": state.version,
                    "url": zip_url,
                    "zip_url": zip_url,
                    "sha256": "0" * 64,
                }
                self._send(200, json.dumps(payload).encode("utf-8"))
            elif path == "/latest_404.json":
                self._send(404, b'{"error":"not found"}')
            elif path == "/latest_500.json":
                self._send(500, b'{"error":"internal server error"}')
            elif path == "/latest_slow.json":
                time.sleep(15.0)
                self._send(200, b'{"version":"9.9.9"}')
            else:
                self._send(404, b'{"error":"unknown path"}')

    return Handler


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--version", type=str, default="9.9.9")
    parser.add_argument("--ready-file", type=str, required=True)
    args = parser.parse_args()

    state = MockGitHubState(args.version)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(state))
    actual_port = server.server_address[1]
    state.base_url = f"http://127.0.0.1:{actual_port}"

    info = {
        "port": actual_port,
        "base_url": state.base_url,
        "version": state.version,
        "zip_url": f"{state.base_url}/PhantomSpire-win64.zip",
        "zip_sha256": state.zip_sha256,
        "zip_size": len(state.zip_bytes),
    }
    tmp_ready = args.ready_file + ".tmp"
    with open(tmp_ready, "w", encoding="utf-8") as f:
        json.dump(info, f)
    os.replace(tmp_ready, args.ready_file)

    try:
        server.serve_forever(poll_interval=0.2)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
