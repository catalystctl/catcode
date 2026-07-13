#!/usr/bin/env python3
"""Deny writes to sensitive paths (.env, keys, credential stores)."""
import json
import os
import sys

BLOCK_NAMES = {
    ".env",
    ".env.local",
    ".env.production",
    "credentials.json",
    "id_rsa",
    "id_ed25519",
    "secrets.yaml",
    "secrets.yml",
}
BLOCK_SUFFIXES = (".pem", ".p12", ".pfx", ".key")
BLOCK_PARTS = ("/.ssh/", "/.aws/", "/.gnupg/")


def main():
    raw = sys.stdin.read()
    try:
        ctx = json.loads(raw) if raw.strip() else {}
    except Exception:
        print(json.dumps({"allow": True}))
        return
    path = ((ctx.get("args") or {}).get("path") or "").replace("\\", "/")
    base = os.path.basename(path)
    lower = path.lower()
    blocked = (
        base in BLOCK_NAMES
        or any(base.endswith(s) for s in BLOCK_SUFFIXES)
        or any(p in lower for p in BLOCK_PARTS)
    )
    if blocked:
        print(
            json.dumps(
                {
                    "allow": False,
                    "reason": f"path-guard blocked write to sensitive path: {path}",
                    "notify": f"blocked write to {base}",
                }
            )
        )
        return
    print(json.dumps({"allow": True}))


if __name__ == "__main__":
    main()
