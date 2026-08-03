#!/usr/bin/env python3
"""ChatGPT Codex OAuth using the official Codex CLI device-code protocol.

The harness sends one JSON object on stdin with an ``action`` of ``login``,
``complete``, ``token``, or ``clear``. One JSON object is written to stdout.
The ``poll`` login flow tells the harness to invoke ``complete`` immediately,
so authorization never needs a separate ``/oauth-code`` command.

This file deliberately uses only the Python standard library. The endpoint
names, client id, request shapes, redirect URI, refresh grant, and auth.json
layout mirror the open-source OpenAI Codex CLI.
"""

import base64
import json
import os
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"
DEFAULT_AUTH_BASE_URL = "https://auth.openai.com"
REFRESH_THRESHOLD_S = 300
DEVICE_WAIT_S = 900


# ---- harness I/O ---------------------------------------------------------

def emit(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")))
    sys.stdout.flush()


def die(message):
    emit({"ok": False, "error": str(message)})
    raise SystemExit(0)


def now():
    return int(time.time())


# ---- HTTP ----------------------------------------------------------------

def auth_base_url():
    return (os.environ.get("CODEX_AUTH_BASE_URL") or DEFAULT_AUTH_BASE_URL).rstrip("/")


def http_post(url, body, content_type):
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Accept": "application/json",
            "Content-Type": content_type,
            "User-Agent": "codex_cli_rs",
            "originator": "codex_cli_rs",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            raw = response.read().decode("utf-8", "replace")
            return response.status, parse_json(raw)
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode("utf-8", "replace")
        return exc.code, parse_json(raw)
    except Exception as exc:
        return 0, {"error": "request_failed", "error_description": str(exc)}


def parse_json(raw):
    try:
        value = json.loads(raw) if raw.strip() else {}
        return value if isinstance(value, dict) else {}
    except Exception:
        return {"error": "invalid_json", "error_description": raw[:500]}


def post_json(url, payload):
    return http_post(
        url,
        json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        "application/json",
    )


def post_form(url, fields):
    return http_post(
        url,
        urllib.parse.urlencode(fields).encode("utf-8"),
        "application/x-www-form-urlencoded",
    )


def error_text(status, data):
    return (
        data.get("error_description")
        or data.get("error")
        or ("network request failed" if status == 0 else f"HTTP {status}")
    )


# ---- token files ----------------------------------------------------------

def read_json(path):
    try:
        with open(path, encoding="utf-8") as handle:
            value = json.load(handle)
        return value if isinstance(value, dict) else None
    except (OSError, ValueError, TypeError):
        return None


def atomic_write(path, value):
    path = os.path.abspath(path)
    parent = os.path.dirname(path) or "."
    os.makedirs(parent, mode=0o700, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".codex-oauth-", dir=parent)
    try:
        try:
            os.fchmod(fd, 0o600)
        except AttributeError:
            # Windows has no POSIX mode-bit syscall; the file is still created
            # through a private temporary name before the atomic replace.
            pass
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, separators=(",", ":"))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, path)
    except Exception:
        try:
            os.close(fd)
        except OSError:
            pass
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def lock_for(path):
    try:
        import fcntl
    except ImportError:
        return None
    path = os.path.abspath(path)
    os.makedirs(os.path.dirname(path) or ".", mode=0o700, exist_ok=True)
    handle = open(path + ".lock", "a+", encoding="utf-8")
    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
    except OSError:
        pass
    return handle


def unlock(handle):
    if handle is not None:
        try:
            handle.close()
        except OSError:
            pass


def token_path(ctx):
    return os.path.abspath(str(ctx.get("token_path") or "codex.json"))


def codex_auth_path():
    root = os.environ.get("CODEX_HOME") or os.path.join(os.path.expanduser("~"), ".codex")
    return os.path.join(os.path.expanduser(root), "auth.json")


# ---- JWT / Codex CLI auth.json -------------------------------------------

def token_string(value):
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        for key in ("raw", "token", "value"):
            if isinstance(value.get(key), str):
                return value[key]
    return ""


def jwt_payload(raw):
    raw = token_string(raw)
    parts = raw.split(".")
    if len(parts) < 2:
        return {}
    try:
        padded = parts[1] + "=" * (-len(parts[1]) % 4)
        value = json.loads(base64.urlsafe_b64decode(padded.encode("ascii")))
        return value if isinstance(value, dict) else {}
    except (ValueError, TypeError, UnicodeError):
        return {}


def integer(value, default=0):
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def jwt_exp(raw):
    return integer(jwt_payload(raw).get("exp"))


def account_id_from_tokens(id_token, access_token, explicit=""):
    if explicit:
        return str(explicit)
    for raw in (id_token, access_token):
        payload = jwt_payload(raw)
        for key in (
            "https://api.openai.com/auth.chatgpt_account_id",
            "chatgpt_account_id",
            "account_id",
        ):
            value = payload.get(key)
            if value:
                return str(value)
        nested = payload.get("https://api.openai.com/auth")
        if isinstance(nested, dict) and nested.get("chatgpt_account_id"):
            return str(nested["chatgpt_account_id"])
    return ""


def normalize_tokens(value):
    if not isinstance(value, dict):
        return None
    access_token = token_string(value.get("access_token"))
    refresh_token = token_string(value.get("refresh_token"))
    id_token = token_string(value.get("id_token"))
    if not access_token and not refresh_token:
        return None
    expires_at = (
        jwt_exp(access_token)
        or jwt_exp(id_token)
        or integer(value.get("expires_at"))
    )
    return {
        "access_token": access_token,
        "refresh_token": refresh_token,
        "id_token": id_token,
        "account_id": account_id_from_tokens(
            id_token, access_token, value.get("account_id", "")
        ),
        "expires_at": expires_at,
    }


def read_codex_auth():
    document = read_json(codex_auth_path())
    if not document:
        return None
    mode = document.get("auth_mode")
    if mode and mode not in ("chatgpt", "chatgpt_auth"):
        return None
    tokens = document.get("tokens")
    if not isinstance(tokens, dict):
        tokens = document
    return normalize_tokens(tokens)


def external_is_newer(external_path, local_path):
    try:
        return os.path.getmtime(external_path) > os.path.getmtime(local_path)
    except OSError:
        return False


def account_header(token):
    account_id = str(token.get("account_id") or "")
    return [["ChatGPT-Account-ID", account_id]] if account_id else []


# ---- device flow ---------------------------------------------------------

def do_login(ctx):
    existing = read_codex_auth()
    if existing:
        atomic_write(token_path(ctx), existing)
        emit(
            {
                "ok": True,
                "flow": "already_authenticated",
                "message": "Imported the existing Codex CLI ChatGPT login; no new authorization is needed.",
            }
        )
        return

    base = auth_base_url()
    status, data = post_json(
        base + "/api/accounts/deviceauth/usercode", {"client_id": CLIENT_ID}
    )
    device_auth_id = data.get("device_auth_id")
    user_code = data.get("user_code") or data.get("usercode") or ""
    if status != 200 or not device_auth_id or not user_code:
        die("device authorization failed: " + error_text(status, data))

    interval = max(integer(data.get("interval"), 5), 1)
    pending = {
        "device_auth_id": str(device_auth_id),
        "user_code": str(user_code),
        "interval": interval,
        "expires_at": now() + DEVICE_WAIT_S,
    }
    emit(
        {
            "url": base + "/codex/device",
            "code": str(user_code),
            "flow": "poll",
            "auto_complete": True,
            "pending": pending,
            "message": "Open the URL below, enter the device code, and authorize ChatGPT. This login will poll automatically; no /oauth-code command is required.",
        }
    )


def exchange_device_code(data, ctx):
    authorization_code = data.get("authorization_code")
    code_verifier = data.get("code_verifier")
    if not authorization_code or not code_verifier:
        die("device authorization returned an incomplete authorization code")
    base = auth_base_url()
    status, tokens = post_form(
        base + "/oauth/token",
        {
            "grant_type": "authorization_code",
            "code": authorization_code,
            "redirect_uri": base + "/deviceauth/callback",
            "client_id": CLIENT_ID,
            "code_verifier": code_verifier,
        },
    )
    access_token = token_string(tokens.get("access_token"))
    if status != 200 or not access_token:
        die("authorization-code exchange failed: " + error_text(status, tokens))
    expires_at = jwt_exp(access_token) or jwt_exp(tokens.get("id_token"))
    if not expires_at:
        expires_at = now() + max(integer(tokens.get("expires_in"), 3600), 60)
    normalized = normalize_tokens(
        {
            "access_token": access_token,
            "refresh_token": tokens.get("refresh_token", ""),
            "id_token": tokens.get("id_token", ""),
            "expires_at": expires_at,
        }
    )
    if not normalized:
        die("authorization-code exchange returned no usable tokens")
    atomic_write(token_path(ctx), normalized)
    emit({"ok": True})


def do_complete(ctx):
    pending = ctx.get("pending") or {}
    device_auth_id = pending.get("device_auth_id")
    user_code = pending.get("user_code")
    if not device_auth_id or not user_code:
        die("no device authorization state; restart /login codex")
    interval = max(integer(pending.get("interval"), 5), 1)
    deadline = integer(pending.get("expires_at"), now() + DEVICE_WAIT_S)
    endpoint = auth_base_url() + "/api/accounts/deviceauth/token"
    while now() < deadline:
        status, data = post_json(
            endpoint,
            {"device_auth_id": device_auth_id, "user_code": user_code},
        )
        if status == 200 and data.get("authorization_code"):
            exchange_device_code(data, ctx)
            return
        # The Codex CLI retries these two statuses while the user approves.
        if status not in (403, 404):
            die("device authorization polling failed: " + error_text(status, data))
        time.sleep(interval)
    die("timed out waiting for ChatGPT device authorization")


# ---- refresh / action dispatch -------------------------------------------

def refresh_token(path, token):
    status, data = post_json(
        auth_base_url() + "/oauth/token",
        {
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": token.get("refresh_token", ""),
        },
    )
    access_token = token_string(data.get("access_token"))
    if status != 200 or not access_token:
        return None
    expires_at = jwt_exp(access_token) or jwt_exp(data.get("id_token"))
    if not expires_at:
        expires_at = now() + max(integer(data.get("expires_in"), 3600), 60)
    updated = normalize_tokens(
        {
            "access_token": access_token,
            "refresh_token": data.get("refresh_token") or token.get("refresh_token", ""),
            "id_token": data.get("id_token") or token.get("id_token", ""),
            "account_id": data.get("account_id") or token.get("account_id", ""),
            "expires_at": expires_at,
        }
    )
    if updated:
        atomic_write(path, updated)
    return updated


def do_token(ctx):
    path = token_path(ctx)
    local = read_json(path)
    external_path = codex_auth_path()
    external = read_codex_auth()
    token = normalize_tokens(local)
    if external and (not token or external_is_newer(external_path, path)):
        token = external
        atomic_write(path, token)
    if not token:
        emit({"access_token": None})
        return

    current = now()
    expires_at = integer(token.get("expires_at"))
    needs_refresh = (not token.get("access_token")) or (
        expires_at > 0 and expires_at - current <= REFRESH_THRESHOLD_S
    )
    if needs_refresh and token.get("refresh_token"):
        handle = lock_for(path)
        try:
            # Another harness process may have refreshed while this process
            # waited for the lock; always re-read before making a request.
            current_token = normalize_tokens(read_json(path)) or token
            latest_external = read_codex_auth()
            if latest_external and external_is_newer(external_path, path):
                current_token = latest_external
                atomic_write(path, current_token)
            current_exp = integer(current_token.get("expires_at"))
            if current_token.get("access_token") and (
                current_exp == 0 or current_exp - now() > REFRESH_THRESHOLD_S
            ):
                token = current_token
            else:
                token = refresh_token(path, current_token)
                if not token:
                    emit({"access_token": None})
                    return
        finally:
            unlock(handle)

    emit(
        {
            "access_token": token.get("access_token"),
            "expires_at": integer(token.get("expires_at")),
            "headers": account_header(token),
        }
    )


def do_clear(ctx):
    path = token_path(ctx)
    for candidate in (path, path + ".lock", path + ".tmp"):
        try:
            os.remove(candidate)
        except OSError:
            pass
    # Deliberately leave ~/.codex/auth.json intact: that file belongs to the
    # Codex CLI, and clearing this provider must not sign the CLI out.
    emit({"ok": True})


def main():
    try:
        raw = sys.stdin.read()
        ctx = json.loads(raw) if raw.strip() else {}
        if not isinstance(ctx, dict):
            die("OAuth context must be a JSON object")
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
            die("unknown action: %r" % action)
    except SystemExit:
        raise
    except Exception as exc:
        die("Codex OAuth provider error: " + str(exc))


if __name__ == "__main__":
    main()
