#!/usr/bin/env python3
"""Minimal stand-in for the Managed Agents API, for local control-plane testing.

Asserts the mandatory headers on every request, records created resources in
memory, and prints each request body. Run, then point the control plane at it:

    python3 control-plane/dev/mock-managed-agents.py 9999
    ANTHROPIC_BASE_URL=http://127.0.0.1:9999 ANTHROPIC_API_KEY=test ... cargo run -p control-plane

Nothing here is a substitute for the real API; it only checks the wire shape.
"""
import json
import sys
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer

REQUIRED = {"x-api-key": None, "anthropic-version": "2023-06-01", "anthropic-beta": "managed-agents-2026-04-01"}
STATE = {"agents": {}, "environments": {}, "sessions": {}}


class H(BaseHTTPRequestHandler):
    def _check_headers(self):
        for k, want in REQUIRED.items():
            got = self.headers.get(k)
            if got is None or (want is not None and got != want):
                self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
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
        if p == "/v1/agents":
            aid = "agent_mock_" + uuid.uuid4().hex[:10]
            STATE["agents"][aid] = {"id": aid, "type": "agent", "version": 1, **body}
            return self._json(200, STATE["agents"][aid])
        if p.startswith("/v1/agents/"):
            aid = p.rsplit("/", 1)[1]
            a = STATE["agents"].get(aid)
            if not a:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such agent"}})
            a.update(body); a["version"] += 1
            return self._json(200, a)
        if p == "/v1/environments":
            eid = "env_mock_" + uuid.uuid4().hex[:10]
            STATE["environments"][eid] = {"id": eid, "type": "environment", **body}
            return self._json(200, STATE["environments"][eid])
        if p == "/v1/sessions":
            amt = body.get("budget", {}).get("max_list_cost", {}).get("amount")
            if not isinstance(amt, str) or not amt.isdigit() or amt.startswith("0"):
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                        "message": f"budget.max_list_cost.amount must be a whole-cent string, got {amt!r}"}})
            sid = "sesn_mock_" + uuid.uuid4().hex[:10]
            STATE["sessions"][sid] = {
                "id": sid, "type": "session", "status": "running" if body.get("initial_events") else "idle",
                "agent": body["agent"], "environment_id": body["environment_id"], "title": body.get("title"),
                "metadata": body.get("metadata", {}), "budget": body["budget"],
                "usage": {"input_tokens": 0, "output_tokens": 0, "active_seconds": 0,
                          "list_cost": {"amount": "1", "currency": "USD"}},
            }
            return self._json(200, STATE["sessions"][sid])
        if p.startswith("/v1/sessions/") and p.endswith("/events"):
            sid = p.split("/")[3]
            s = STATE["sessions"].get(sid)
            if not s:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such session"}})
            s["status"] = "running"
            return self._json(200, s)
        if p.startswith("/v1/sessions/") and p.endswith("/interrupt"):
            sid = p.split("/")[3]
            s = STATE["sessions"].get(sid)
            if not s:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such session"}})
            s["status"] = "idle"
            return self._json(200, s)
        self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": p}})

    def do_GET(self):
        if not self._check_headers():
            return
        print(f"--> GET {self.path}", flush=True)
        p = self.path.split("?")[0]
        if p == "/v1/sessions":
            return self._json(200, {"data": list(STATE["sessions"].values()), "next_page": None, "prev_page": None})
        if p.startswith("/v1/sessions/"):
            s = STATE["sessions"].get(p.rsplit("/", 1)[1])
            if s:
                # Pretend the session did some work and crossed its cap.
                s["status"] = "idle"
                s["usage"]["list_cost"]["amount"] = str(int(s["budget"]["max_list_cost"]["amount"]) + 3)
                return self._json(200, s)
            return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such session"}})
        self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": p}})

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9999
    print(f"mock managed agents api on 127.0.0.1:{port}", flush=True)
    HTTPServer(("127.0.0.1", port), H).serve_forever()
