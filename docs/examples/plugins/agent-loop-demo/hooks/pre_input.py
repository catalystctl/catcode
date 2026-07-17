#!/usr/bin/env python3
"""Trim leading/trailing whitespace on user input (demo)."""
import json, sys
ctx = json.load(sys.stdin)
text = (ctx.get("args") or {}).get("text") or ""
print(json.dumps({"allow": True, "modify": {"text": text.strip()}}))
