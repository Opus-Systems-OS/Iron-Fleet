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
import time
import uuid
from datetime import datetime, timezone
from email.parser import BytesParser
from email.policy import HTTP
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn

REQUIRED = {"x-api-key": None, "anthropic-version": "2023-06-01", "anthropic-beta": "managed-agents-2026-04-01"}
# The Skills API is GA: same key/version, multipart body, and no beta header.
SKILLS_REQUIRED = {"x-api-key": None, "anthropic-version": "2023-06-01"}
STATE = {"agents": {}, "environments": {}, "sessions": {}, "vaults": {}, "credentials": {}, "skills": {}}
# Per-session event history, oldest first — what GET …/events returns.
EVENTS = {}


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _record(sid, event):
    """Persist an event the way the API does: server-assigned id and processed_at."""
    event = {"id": "sevt_mock_" + uuid.uuid4().hex[:10], "processed_at": _now(), **event}
    EVENTS.setdefault(sid, []).append(event)
    return event

# What docs.claude.com/managed-agents/mcp-connector documents as the only
# accepted mcp_servers entry fields — this is what caught the real
# authorization_token mistake, so keep it strict rather than lenient.
MCP_SERVER_FIELDS = {"type", "name", "url"}
STATIC_BEARER_AUTH_FIELDS = {"type", "mcp_server_url", "token"}
# Write-only on the real API; never echoed in responses, and never printed here.
SECRET_KEYS = {"token", "access_token", "refresh_token", "client_secret", "secret_value", "authorization_token"}


def _redact(obj):
    if isinstance(obj, dict):
        return {k: ("<redacted>" if k in SECRET_KEYS else _redact(v)) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_redact(v) for v in obj]
    return obj


class H(BaseHTTPRequestHandler):
    def _check_headers(self, required=REQUIRED):
        for k, want in required.items():
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
        print(f"--> {self.command} {self.path}\n{json.dumps(_redact(obj), indent=2)}", flush=True)
        return obj

    def _skill_upload(self):
        """Parse a multipart `files[]` upload the way the Skills API documents it:
        every part's filename is `<dir>/<relative path>`, all under one top-level
        dir, which must contain SKILL.md. Returns (dir_name, error)."""
        n = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(n) if n else b""
        ctype = self.headers.get("content-type", "")
        if not ctype.startswith("multipart/form-data"):
            return None, f"expected multipart/form-data, got {ctype!r}"
        msg = BytesParser(policy=HTTP).parsebytes(b"content-type: " + ctype.encode() + b"\r\n\r\n" + raw)
        names = []
        for part in msg.iter_parts():
            if part.get_param("name", header="content-disposition") != "files[]":
                return None, "every part must be a files[] entry"
            fn = part.get_filename()
            if not fn or "/" not in fn:
                return None, f"part filename must be <dir>/<path>, got {fn!r} (percent-encoded slash?)"
            names.append(fn)
        print(f"--> {self.command} {self.path}\n  files[]: {names}", flush=True)
        tops = {fn.split("/", 1)[0] for fn in names}
        if len(tops) != 1:
            return None, f"all files must share one top-level directory, got {sorted(tops)}"
        top = tops.pop()
        if f"{top}/SKILL.md" not in names:
            return None, f"{top}/SKILL.md is required"
        return top, None

    def do_POST(self):
        p = self.path.split("?")[0]
        if p == "/v1/skills" or (p.startswith("/v1/skills/") and p.endswith("/versions")):
            if not self._check_headers(SKILLS_REQUIRED):
                return
            top, err = self._skill_upload()
            if err:
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error", "message": err}})
            if p == "/v1/skills":
                sid = "skill_mock_" + uuid.uuid4().hex[:10]
                ver = "skillver_mock_" + uuid.uuid4().hex[:10]
                STATE["skills"][sid] = {"id": sid, "type": "skill", "display_name": top, "name": top,
                                        "latest_version_id": ver, "source": {"type": "custom"}}
                return self._json(200, STATE["skills"][sid])
            sid = p.split("/")[3]
            s = STATE["skills"].get(sid)
            if not s:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such skill"}})
            if s["name"] != top:
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                        "message": f"skill name is immutable: {s['name']!r} != {top!r}"}})
            ver = "skillver_mock_" + uuid.uuid4().hex[:10]
            s["latest_version_id"] = ver
            return self._json(200, {"id": ver, "type": "skill_version", "skill_id": sid, "name": top, "description": ""})

        if not self._check_headers():
            return
        body = self._body()
        if p == "/v1/agents":
            err = self._validate_mcp_shape(body) or self._validate_skills_shape(body)
            if err:
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error", "message": err}})
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
            cfg = body.get("config", {})
            pk = cfg.get("packages", {})
            if set(pk.keys()) - {"apt", "cargo", "gem", "go", "npm", "pip"}:
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                        "message": f"unknown package manager in config.packages: {sorted(pk.keys())}"}})
            if pk and cfg.get("networking", {}).get("type") == "limited" and not cfg["networking"].get("allow_package_managers"):
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                        "message": "packages with limited networking needs allow_package_managers: true"}})
            eid = "env_mock_" + uuid.uuid4().hex[:10]
            STATE["environments"][eid] = {"id": eid, "type": "environment", **body}
            return self._json(200, STATE["environments"][eid])
        if p == "/v1/sessions":
            amt = body.get("budget", {}).get("max_list_cost", {}).get("amount")
            if not isinstance(amt, str) or not amt.isdigit() or amt.startswith("0"):
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                        "message": f"budget.max_list_cost.amount must be a whole-cent string, got {amt!r}"}})
            for r in body.get("resources", []):
                # docs.claude.com/managed-agents/github: exactly this URL form, token required.
                url = r.get("url", "")
                if r.get("type") != "github_repository" or not r.get("authorization_token"):
                    return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                            "message": f"resources entries must be github_repository with authorization_token: {_redact(r)}"}})
                parts = url.removeprefix("https://github.com/").split("/")
                if not url.startswith("https://github.com/") or len(parts) != 2 or not all(parts) or url.endswith(".git"):
                    return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                            "message": f"resources[].url must be https://github.com/<owner>/<repo>, got {url!r}"}})
            for vid in body.get("vault_ids", []):
                if vid not in STATE["vaults"]:
                    return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": f"no such vault {vid}"}})
            sid = "sesn_mock_" + uuid.uuid4().hex[:10]
            STATE["sessions"][sid] = {
                "id": sid, "type": "session", "status": "running" if body.get("initial_events") else "idle",
                "agent": body["agent"], "environment_id": body["environment_id"], "title": body.get("title"),
                "metadata": body.get("metadata", {}), "budget": body["budget"],
                "vault_ids": body.get("vault_ids", []),
                "resources": [{k: v for k, v in r.items() if k != "authorization_token"} for r in body.get("resources", [])],
                "usage": {"input_tokens": 0, "output_tokens": 0, "active_seconds": 0,
                          "list_cost": {"amount": "1", "currency": "USD"}},
            }
            for ev in body.get("initial_events", []):
                _record(sid, ev)
            return self._json(200, STATE["sessions"][sid])
        if p.startswith("/v1/sessions/") and p.endswith("/events"):
            # There is no /interrupt route on the real API: user.interrupt is
            # just another event here, and the turn ends with status_idle.
            sid = p.split("/")[3]
            s = STATE["sessions"].get(sid)
            if not s:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such session"}})
            for ev in body.get("events", []):
                if ev.get("type") not in ("user.message", "user.interrupt"):
                    return self._json(400, {"type": "error", "error": {"type": "invalid_request_error", "message": f"unsupported event type {ev.get('type')!r}"}})
                _record(sid, ev)
                s["status"] = "idle" if ev["type"] == "user.interrupt" else "running"
            return self._json(200, s)
        if p == "/v1/vaults":
            vid = "vlt_mock_" + uuid.uuid4().hex[:10]
            STATE["vaults"][vid] = {"id": vid, "type": "vault", **body}
            return self._json(200, STATE["vaults"][vid])
        if p.startswith("/v1/vaults/") and p.endswith("/credentials"):
            vid = p.split("/")[3]
            if vid not in STATE["vaults"]:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such vault"}})
            auth = body.get("auth", {})
            if auth.get("type") == "static_bearer":
                extra = set(auth.keys()) - STATIC_BEARER_AUTH_FIELDS
                if extra:
                    return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                            "message": f"Failed to parse request body: unknown field {sorted(extra)[0]!r}"}})
                if not auth.get("mcp_server_url") or not auth.get("token"):
                    return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                            "message": "static_bearer auth requires mcp_server_url and token"}})
            elif auth.get("type") == "environment_variable":
                err = self._validate_env_var_auth(auth, creating=True)
                if err:
                    return self._json(400, {"type": "error", "error": {"type": "invalid_request_error", "message": err}})
                # secret_name is unique among a vault's active credentials (409 on a duplicate).
                for c in STATE["credentials"].values():
                    if c["vault_id"] == vid and c["auth"].get("secret_name") == auth["secret_name"]:
                        return self._json(409, {"type": "error", "error": {"type": "conflict_error",
                                                "message": f"secret_name {auth['secret_name']!r} already exists in this vault"}})
            else:
                return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                        "message": f"unsupported auth.type {auth.get('type')!r}"}})
            cid = "vcrd_mock_" + uuid.uuid4().hex[:10]
            # Real credential values are write-only and never echoed back.
            redacted_auth = {k: v for k, v in auth.items() if k not in SECRET_KEYS}
            STATE["credentials"][cid] = {"id": cid, "type": "vault_credential", "vault_id": vid,
                                          "display_name": body.get("display_name"), "auth": redacted_auth}
            return self._json(200, STATE["credentials"][cid])
        if p.startswith("/v1/vaults/") and "/credentials/" in p:
            # Update (rotate). secret_name / mcp_server_url are immutable.
            _, _, _, vid, _, cid = p.split("/")
            c = STATE["credentials"].get(cid)
            if not c or c["vault_id"] != vid:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such credential"}})
            auth = body.get("auth", {})
            if auth:
                if auth.get("type") != c["auth"].get("type"):
                    return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                            "message": "auth.type must match the credential"}})
                if auth["type"] == "environment_variable":
                    err = self._validate_env_var_auth(auth, creating=False)
                    if err:
                        return self._json(400, {"type": "error", "error": {"type": "invalid_request_error", "message": err}})
                    if "networking" in auth:
                        c["auth"]["networking"] = auth["networking"]
                elif auth["type"] == "static_bearer":
                    # Only `token` may change; mcp_server_url is immutable.
                    extra = set(auth.keys()) - {"type", "token"}
                    if extra:
                        return self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                                "message": f"static_bearer update: unknown/immutable field {sorted(extra)[0]!r}"}})
            if body.get("display_name"):
                c["display_name"] = body["display_name"]
            return self._json(200, c)
        self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": p}})

    def _validate_mcp_shape(self, body):
        """Mirrors docs.claude.com/managed-agents/mcp-connector's two rules:
        mcp_servers entries take only type/name/url, and every server needs a
        matching mcp_toolset entry (and vice versa)."""
        servers = body.get("mcp_servers", [])
        server_names = set()
        for s in servers:
            extra = set(s.keys()) - MCP_SERVER_FIELDS
            if extra:
                return f"Failed to parse request body: unknown field {sorted(extra)[0]!r}"
            server_names.add(s.get("name"))
        toolset_names = {t.get("mcp_server_name") for t in body.get("tools", []) if t.get("type") == "mcp_toolset"}
        dangling = toolset_names - server_names
        unreferenced = server_names - toolset_names
        if dangling:
            return f"tools references undeclared mcp server(s): {sorted(dangling)}"
        if unreferenced:
            return f"mcp_servers declared but never referenced by a tools[mcp_toolset]: {sorted(unreferenced)}"
        return None

    def _validate_env_var_auth(self, auth, creating):
        """Mirrors platform.claude.com/docs/managed-agents/vaults, Environment
        variable tab: secret_name + secret_value, networking limited to at most
        16 bare hostnames, injection_location {header, body} with at least one
        enabled. On update secret_name is immutable and everything is optional."""
        allowed = {"type", "secret_name", "secret_value", "networking", "injection_location"}
        extra = set(auth.keys()) - allowed
        if extra:
            return f"Failed to parse request body: unknown field {sorted(extra)[0]!r}"
        if creating:
            if not auth.get("secret_name") or not auth.get("secret_value"):
                return "environment_variable auth requires secret_name and secret_value"
        elif "secret_name" in auth:
            return "secret_name is immutable"
        if "secret_value" in auth and not (1 <= len(auth["secret_value"]) <= 4096):
            return "secret_value must be 1-4096 characters"
        net = auth.get("networking")
        if net is not None:
            if net.get("type") == "limited":
                hosts = net.get("allowed_hosts", [])
                if not hosts or len(hosts) > 16 or any("/" in h or ":" in h for h in hosts):
                    return "networking.allowed_hosts must be 1-16 bare hostnames"
            elif net.get("type") != "unrestricted":
                return f"networking.type must be limited|unrestricted, got {net.get('type')!r}"
        loc = auth.get("injection_location")
        if loc is not None:
            if set(loc.keys()) - {"header", "body"}:
                return "injection_location takes only header/body"
            if creating and not (loc.get("header") or loc.get("body")):
                return "injection_location must enable at least one location"
        return None

    def _validate_skills_shape(self, body):
        """Mirrors platform.claude.com/docs/managed-agents/skills: each entry is
        {type: anthropic|custom, skill_id, version?}. The registry's
        {"type":"custom","skill":"<dir>"} form must have been resolved by sync
        before it gets here — an unresolved one is exactly the bug this catches."""
        for s in body.get("skills", []):
            extra = set(s.keys()) - {"type", "skill_id", "version"}
            if extra:
                return f"Failed to parse request body: unknown field {sorted(extra)[0]!r} in skills"
            if s.get("type") not in ("anthropic", "custom") or not s.get("skill_id"):
                return f"skills entry needs type anthropic|custom and skill_id: {s}"
            if s["type"] == "custom" and s["skill_id"] not in STATE["skills"]:
                return f"no such custom skill {s['skill_id']!r}"
        return None

    def _stream(self, sid):
        """SSE as the docs describe it: `data: {event}` frames, one persisted event
        each, only events emitted after the stream opened. Plays a scripted turn,
        then holds the connection open until the client hangs up."""
        s = STATE["sessions"].get(sid)
        if not s:
            return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such session"}})
        if self.headers.get("accept") != "text/event-stream":
            return self._json(400, {"type": "error", "error": {"type": "invalid_request_error", "message": "accept: text/event-stream required"}})
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.end_headers()
        # The real stream opens with a comment frame; readers must ignore it.
        self.wfile.write(b": connected\n\n")
        self.wfile.flush()
        script = [
            {"type": "session.status_running"},
            {"type": "span.model_request_start"},
            {"type": "agent.message", "content": [{"type": "text", "text": "Mock agent here — I got the task and I am on it."}]},
            {"type": "span.model_request_end"},
            {"type": "agent.tool_use", "name": "bash", "input": {"command": "ls /workspace"}, "evaluated_permission": "allow"},
            {"type": "agent.tool_result", "content": [{"type": "text", "text": "README.md\nsrc\n"}]},
            {"type": "agent.message", "content": [{"type": "text", "text": "Done: two entries in the workspace."}]},
            {"type": "session.usage", "usage": {"input_tokens": 120, "output_tokens": 40, "active_seconds": 2.5, "list_cost": {"amount": "3", "currency": "USD"}}, "budget": s["budget"]},
            {"type": "session.status_idle", "stop_reason": {"type": "end_turn"}},
        ]
        try:
            for ev in script:
                time.sleep(0.4)
                ev = _record(sid, ev)
                if ev["type"] == "session.status_idle":
                    s["status"] = "idle"
                elif ev["type"] == "session.status_running":
                    s["status"] = "running"
                self.wfile.write(f"data: {json.dumps(ev)}\n\n".encode())
                self.wfile.flush()
            while True:  # hold open like the real stream does between turns
                time.sleep(1)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        if not self._check_headers():
            return
        print(f"--> GET {self.path}", flush=True)
        p = self.path.split("?")[0]
        if p == "/v1/sessions":
            return self._json(200, {"data": list(STATE["sessions"].values()), "next_page": None, "prev_page": None})
        if p.startswith("/v1/sessions/") and p.endswith("/events/stream"):
            return self._stream(p.split("/")[3])
        if p.startswith("/v1/sessions/") and p.endswith("/events"):
            sid = p.split("/")[3]
            if sid not in STATE["sessions"]:
                return self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such session"}})
            return self._json(200, {"data": EVENTS.get(sid, []), "next_page": None, "prev_page": None})
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
    class Server(ThreadingMixIn, HTTPServer):
        daemon_threads = True

    Server(("127.0.0.1", port), H).serve_forever()
