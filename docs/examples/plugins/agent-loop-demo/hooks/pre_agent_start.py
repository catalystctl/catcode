#!/usr/bin/env python3
"""Append a short per-turn system note (demo)."""
import json, sys
_ = json.load(sys.stdin)
print(json.dumps({
  "allow": True,
  "modify": {"append_system_prompt": "[agent-loop-demo] Turn started."}
}))
