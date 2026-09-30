"""Loopback-only HTTP fixtures for the real, compiled NSIS installer tests."""
import http.server
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1]).resolve()


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(root), **kwargs)

    def log_message(self, fmt, *args):
        with (root / "requests.log").open("a", encoding="utf-8") as log:
            log.write((fmt % args) + "\n")


with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
    (root / "server.json").write_text(
        json.dumps({"url": f"http://127.0.0.1:{server.server_port}"}), encoding="utf-8"
    )
    server.serve_forever()
