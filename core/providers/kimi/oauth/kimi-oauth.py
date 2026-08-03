#!/usr/bin/env python3
"""Kimi Code (Moonshot) OAuth provider — device-code flow.

Handles the four plugin-OAuth actions the harness dispatches via stdin `action`:

  login    -> POST device authorization; return the verification URL + pending
  complete -> poll the token endpoint (device_code) until approved; write token
  token    -> return a fresh access token (refreshing if near expiry) + headers
  clear    -> delete stored credentials

All token state lives in the file at `token_path` (absolute, harness-provided).
Writes are atomic (temp + rename, 0600). Refresh is serialized with a flock
sidecar so concurrent harness processes (TUI + web + a second TUI) can't clobber
a rotated refresh token.

Stdlib only — no third-party deps (urllib for HTTP; fcntl/msvcrt best-effort
for the cross-process lock).

Kimi OAuth constants mirror the official `kimi` CLI (MoonshotAI/kimi-code) and
the device-code flow documented in the opencode-kimicode-auth reference impl.
"""

import json
import os
import platform
import socket
import sys
import time
import uuid
import urllib.error
import urllib.parse
import urllib.request

CLIENT_ID = "17e5f671-d194-4dfb-9706-5516cb48c098"
SCOPE = "kimi_for_coding"
DEVICE_AUTH_URL = "https://auth.kimi.com/api/oauth/device_authorization"
TOKEN_URL = "https://auth.kimi.com/api/oauth/token"
DEVICE_CODE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
REFRESH_THRESHOLD_S = 300
DEFAULT_VERSION = "1.12.0"


# ---- harness I/O ----
def emit(obj):
    sys.stdout.write(json.dumps(obj))
    sys.stdout.flush()


def die(msg):
    emit({"ok": False, "error": str(msg)})
    sys.exit(0)


def version():
    return os.environ.get("KIMI_CODE_CLI_VERSION") or DEFAULT_VERSION


# ---- HTTP (stdlib urllib, form-encoded) ----
def http_post_form(url, fields):
    data = urllib.parse.urlencode(fields).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        method="POST",
        headers={
            "Content-Type": "application/x-www-form-urlencoded",
            "Accept": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            body = r.read().decode("utf-8", "replace")
            return r.status, (json.loads(body) if body.strip() else {})
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(body)
        except Exception:
            return e.code, {"error": "http_error", "error_description": body}
    except Exception as e:
        return 0, {"error": "request_failed", "error_description": str(e)}


# ---- token file (atomic, 0600) ----
def atomic_write(path, obj):
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(obj, f)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def read_token(path):
    try:
        with open(path) as f:
            return json.load(f)
    except Exception:
        return None


def lock_path(token_path):
    return token_path + ".lock"


def acquire_lock(token_path):
    """Best-effort cross-process lock. Returns an opaque handle to release, or
    None on platforms without flock (Windows best-effort: no serialization)."""
    try:
        import fcntl
    except ImportError:
        return None
    f = open(lock_path(token_path), "w")
    try:
        fcntl.flock(f.fileno(), fcntl.LOCK_EX)
    except Exception:
        pass
    return f


def release_lock(handle):
    if handle:
        try:
            handle.close()
        except Exception:
            pass


# ---- device identity headers (returned by `token` so they're applied per turn) ----
def device_headers(ver, did):
    host = socket.gethostname() or "localhost"
    os_name = platform.system() or "unknown"  # macOS / Linux / Windows
    os_rel = platform.release() or ""
    arch = platform.machine() or "x86_64"
    model = f"{os_name} {os_rel} {arch}".strip()
    return [
        ["User-Agent", f"KimiCLI/{ver}"],
        ["X-Msh-Platform", "kimi_cli"],
        ["X-Msh-Version", ver],
        ["X-Msh-Device-Name", host],
        ["X-Msh-Device-Model", model],
        ["X-Msh-Os-Version", os_rel or os_name],
        ["X-Msh-Device-Id", did],
    ]


def stable_device_id(tok):
    if tok and tok.get("device_id"):
        return tok["device_id"]
    return uuid.uuid4().hex


# ---- actions ----
def do_login(ctx):
    status, data = http_post_form(
        DEVICE_AUTH_URL, {"client_id": CLIENT_ID, "scope": SCOPE}
    )
    if status != 200 or "device_code" not in data:
        die(
            data.get("error_description")
            or data.get("error")
            or f"device authorization failed (HTTP {status})"
        )
    interval = int(data.get("interval", 5) or 5)
    expires_in = int(data.get("expires_in", 1800) or 1800)
    url = data.get("verification_uri_complete") or data.get("verification_uri") or ""
    code = data.get("user_code", "")
    pending = {
        "device_code": data["device_code"],
        "interval": interval,
        "expires_at": int(time.time()) + expires_in,
    }
    msg = (
        "Open the URL below in a browser, enter the code if prompted, and "
        "authorize Kimi Code. When done, return here and run "
        "`/oauth-code kimi` to finish."
    )
    emit(
        {
            "url": url,
            "code": code,
            "message": msg,
            "flow": "manual",
            "pending": pending,
        }
    )


def do_complete(ctx):
    pending = ctx.get("pending") or {}
    device_code = pending.get("device_code")
    if not device_code:
        die("no device_code in pending state — restart /login kimi")
    interval = max(int(pending.get("interval", 5) or 5), 1)
    expires_at = int(pending.get("expires_at", time.time() + 1800))
    while time.time() < expires_at:
        status, data = http_post_form(
            TOKEN_URL,
            {
                "grant_type": DEVICE_CODE_GRANT,
                "device_code": device_code,
                "client_id": CLIENT_ID,
            },
        )
        if status == 200 and data.get("access_token"):
            expires_in = int(data.get("expires_in", 3600) or 3600)
            tok = {
                "access_token": data["access_token"],
                "refresh_token": data.get("refresh_token", ""),
                "expires_in": expires_in,
                "expires_at": int(time.time()) + expires_in,
                "device_id": uuid.uuid4().hex,
            }
            atomic_write(ctx["token_path"], tok)
            emit({"ok": True})
            return
        err = data.get("error", "")
        if err == "expired_token":
            die("device code expired — run /login kimi again")
        if err == "slow_down":
            interval += 5
        # authorization_pending → keep polling
        time.sleep(interval)
    die("timed out waiting for authorization")


def do_token(ctx):
    path = ctx["token_path"]
    tok = read_token(path)
    if not tok or not tok.get("access_token") and not tok.get("refresh_token"):
        emit({"access_token": None})
        return
    did = stable_device_id(tok)
    now = int(time.time())
    expires_at = int(tok.get("expires_at", 0) or 0)
    needs_refresh = (not tok.get("access_token")) or (
        expires_at - now <= REFRESH_THRESHOLD_S
    )
    if needs_refresh and tok.get("refresh_token"):
        handle = acquire_lock(path)
        try:
            # Re-check after acquiring the lock: another process may have
            # already rotated + persisted a fresh token while we waited.
            cur = read_token(path)
            if cur and cur.get("access_token") and (
                int(cur.get("expires_at", 0) or 0) - now > REFRESH_THRESHOLD_S
            ):
                tok = cur
            else:
                status, data = http_post_form(
                    TOKEN_URL,
                    {
                        "grant_type": "refresh_token",
                        "refresh_token": tok["refresh_token"],
                        "client_id": CLIENT_ID,
                    },
                )
                if status == 200 and data.get("access_token"):
                    expires_in = int(data.get("expires_in", 3600) or 3600)
                    tok = {
                        "access_token": data["access_token"],
                        "refresh_token": data.get(
                            "refresh_token", tok.get("refresh_token", "")
                        ),
                        "expires_in": expires_in,
                        "expires_at": now + expires_in,
                        "device_id": did,
                    }
                    atomic_write(path, tok)
                else:
                    # Refresh rejected — surface a re-login prompt rather than
                    # a hard error (the user may just need to /login again).
                    emit({"access_token": None})
                    return
        finally:
            release_lock(handle)
    headers = device_headers(version(), did)
    emit(
        {
            "access_token": tok.get("access_token"),
            "expires_at": int(tok.get("expires_at", 0) or 0),
            "headers": headers,
        }
    )


def do_clear(ctx):
    path = ctx["token_path"]
    for p in (path, lock_path(path), path + ".tmp"):
        try:
            os.remove(p)
        except FileNotFoundError:
            pass
        except OSError:
            pass
    emit({"ok": True})


def main():
    raw = sys.stdin.read()
    try:
        ctx = json.loads(raw) if raw.strip() else {}
    except Exception:
        emit({"ok": False, "error": "invalid JSON on stdin"})
        return
    action = ctx.get("action", "")
    if action == "login":
        do_login(ctx)
    elif action == "complete":
        do_complete(ctx)
    elif action == "token":
        do_token(ctx)
    elif action == "clear":
        do_clear(ctx)
    else:
        die(f"unknown action: {action!r}")


if __name__ == "__main__":
    main()
