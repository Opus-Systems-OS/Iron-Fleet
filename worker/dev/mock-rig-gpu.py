#!/usr/bin/env python3
"""Minimal stand-in for the rig-gpu worker's slice of the Managed Agents API,
for exercising the claim/execute/heartbeat/results loop without a live key or
a real GPU. The wire shape here is `worker/src/protocol.rs`'s best guess, not
a confirmed spec — see that file's doc comment.

    python3 worker/dev/mock-rig-gpu.py 9998
    ANTHROPIC_BASE_URL=http://127.0.0.1:9998 RIG_ENVIRONMENT_ID=env_mock \
      RIG_ENVIRONMENT_KEY=test cargo run -p worker -- claim-once

Seeds one claim (an echo and an nvidia-smi probe) the first time a worker
polls; every claim after that is empty (204) until you POST /seed again.
"""
import json
import sys
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer

REQUIRED = {"x-environment-key": None, "anthropic-version": "2023-06-01", "anthropic-beta": "managed-agents-2026-04-01"}
STATE = {"claims": {}, "queue": []}


def seed_claim():
    cid = "claim_mock_" + uuid.uuid4().hex[:10]
    claim = {
        "claim_id": cid,
        "session_id": "sesn_mock_" + uuid.uuid4().hex[:10],
        "lease_seconds": 120,
        "tool_calls": [
            {"id": "toolu_1", "name": "bash", "input": {"command": "echo hello from rig-gpu"}},
            {"id": "toolu_2", "name": "bash", "input": {"command": "nvidia-smi || echo no gpu on this box"}},
        ],
    }
    STATE["claims"][cid] = claim
    STATE["queue"].append(cid)


seed_claim()


class H(BaseHTTPRequestHandler):
    def _check_headers(self):
        for k, want in REQUIRED.items():
            got = self.headers.get(k)
            if got is None or (want is not None and got != want):
                self._json(400, {"error": {"type": "invalid_request_error",
                                 "message": f"missing/invalid header {k}: {got!r}"}})
                return False
        return True

    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("request-id", "req_mock_" + uuid.uuid4().hex[:8])
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _no_content(self):
        self.send_response(204)
        self.send_header("request-id", "req_mock_" + uuid.uuid4().hex[:8])
        self.send_header("content-length", "0")
        self.end_headers()

    def _body(self):
        n = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(n) if n else b""
        obj = json.loads(raw) if raw else {}
        print(f"--> {self.command} {self.path}\n{json.dumps(obj, indent=2)}", flush=True)
        return obj

    def do_POST(self):
        if not self._check_headers():
            return
        body = self._body()
        p = self.path.split("?")[0]
        parts = p.strip("/").split("/")
        # v1 / environments / {id} / claims [/ {claim_id} / heartbeat|results|release]
        if len(parts) == 4 and parts[0:2] == ["v1", "environments"] and parts[3] == "claims":
            if STATE["queue"]:
                cid = STATE["queue"].pop(0)
                return self._json(200, STATE["claims"][cid])
            return self._no_content()
        if len(parts) == 6 and parts[3] == "claims":
            claim_id, action = parts[4], parts[5]
            if claim_id not in STATE["claims"]:
                return self._json(404, {"error": {"type": "not_found_error", "message": "no such claim"}})
            if action == "heartbeat":
                return self._json(200, {"claim_id": claim_id, "status": "extended"})
            if action == "results":
                return self._json(200, {"claim_id": claim_id, "accepted": len(body.get("results", []))})
            if action == "release":
                STATE["queue"].insert(0, claim_id)
                return self._json(200, {"claim_id": claim_id, "status": "released"})
        if p == "/seed":
            seed_claim()
            return self._json(200, {"queued": len(STATE["queue"])})
        self._json(404, {"error": {"type": "not_found_error", "message": p}})

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9998
    print(f"mock rig-gpu worker api on 127.0.0.1:{port} (one claim pre-seeded)", flush=True)
    HTTPServer(("127.0.0.1", port), H).serve_forever()
