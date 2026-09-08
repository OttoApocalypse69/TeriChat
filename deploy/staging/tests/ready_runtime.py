"""Real local Caddy + synthetic API. Needs an already installed Caddy binary.
No Docker, TLS, external network, or staging configuration/secrets are used.
"""
import http.server
import os
import pathlib
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request


def main():
    binary = os.environ.get("CADDY_BIN") or shutil.which("caddy")
    if not binary:
        print("BLOCKED: Caddy binary missing; set CADDY_BIN to an approved local Caddy 2 binary")
        return 2

    class API(http.server.BaseHTTPRequestHandler):
        ready_status = 503

        def do_GET(self):
            self.send_response(self.ready_status if self.path == "/ready" else 200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"source":"synthetic-api"}')

        def log_message(self, *_args):
            pass

    api = http.server.ThreadingHTTPServer(("127.0.0.1", 0), API)
    thread = threading.Thread(target=api.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="terichat-ready-") as tmp:
            root = pathlib.Path(tmp)
            (root / "index.html").write_text("SYNTHETIC SPA", encoding="utf-8")
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            config = (pathlib.Path(__file__).parents[1] / "Caddyfile").read_text()
            config = config.replace(
                "http://{$STAGING_HOST}, http://chat.unknownchat.xyz {",
                f"http://127.0.0.1:{port} {{",
            ).replace("api:3001", f"127.0.0.1:{api.server_port}")
            config = config.replace("/srv/www", f'"{root.as_posix()}"')
            config = "{\n admin off\n auto_https off\n}\n" + config
            path = root / "Caddyfile"
            path.write_text(config, encoding="utf-8")
            subprocess.run([binary, "validate", "--config", str(path), "--adapter", "caddyfile"], check=True)
            with (root / "caddy.log").open("w") as log:
                proc = subprocess.Popen([binary, "run", "--config", str(path), "--adapter", "caddyfile"], stdout=log, stderr=log)
                try:
                    # Bypass ambient HTTP proxies for these local-only probes.
                    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

                    def fetch(route):
                        try:
                            response = opener.open(f"http://127.0.0.1:{port}{route}", timeout=2)
                        except urllib.error.HTTPError as error:
                            response = error
                        with response:
                            return response.status, response.read()

                    for _ in range(100):
                        try:
                            if fetch("/health")[0] == 200:
                                break
                        except OSError:
                            pass
                        if proc.poll() is not None:
                            raise RuntimeError("Caddy exited before readiness")
                        time.sleep(0.1)
                    else:
                        raise RuntimeError("Caddy did not start within 10 seconds")
                    assert fetch("/ready") == (503, b'{"source":"synthetic-api"}')
                    API.ready_status = 200
                    assert fetch("/ready") == (200, b'{"source":"synthetic-api"}')
                    assert fetch("/v1/fixture") == (200, b'{"source":"synthetic-api"}')
                    assert fetch("/client/route") == (200, b"SYNTHETIC SPA")
                    print("PASS: Caddy preserves API /ready 503 and 200; /v1 proxy and SPA fallback work")
                finally:
                    proc.terminate()
                    try:
                        proc.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait()
    finally:
        api.shutdown()
        api.server_close()
        thread.join()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
