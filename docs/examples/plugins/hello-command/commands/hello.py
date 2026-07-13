#!/usr/bin/env python3
"""Example /hello plugin command."""
import json
import sys


def main():
    raw = sys.stdin.read()
    try:
        ctx = json.loads(raw) if raw.strip() else {}
    except Exception as e:
        print(json.dumps({"ok": False, "output": f"bad json: {e}"}))
        return
    name = (ctx.get("args") or "").strip() or "world"
    print(
        json.dumps(
            {
                "ok": True,
                "output": f"Hello, {name}!",
                "notify": f"greeted {name}",
                "status": f"hello → {name}",
            }
        )
    )


if __name__ == "__main__":
    main()
