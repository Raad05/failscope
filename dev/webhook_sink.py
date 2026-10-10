#!/usr/bin/env python3
"""Tiny webhook receiver for local alert testing: appends each POSTed JSON
body as one line to the given file and answers 204.

    dev/webhook_sink.py 9911 .local/webhooks.jsonl
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT, OUT = int(sys.argv[1]), sys.argv[2]


class Sink(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        with open(OUT, "a") as f:
            f.write(json.dumps(json.loads(body)) + "\n")
        self.send_response(204)
        self.end_headers()

    def log_message(self, *args):
        pass


HTTPServer(("127.0.0.1", PORT), Sink).serve_forever()
