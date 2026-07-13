#!/usr/bin/env bash
# Demo bash override: block commands that mention sensitive filenames, else run.
set -euo pipefail
input=$(cat)
cmd=$(echo "$input" | python3 -c "import sys,json; print(json.load(sys.stdin).get('args',{}).get('command',''))" 2>/dev/null || true)
ws=$(echo "$input" | python3 -c "import sys,json; print(json.load(sys.stdin).get('workspace','.'))" 2>/dev/null || echo ".")

lower=$(printf '%s' "$cmd" | tr '[:upper:]' '[:lower:]')
for bad in '.env' 'id_rsa' 'id_ed25519' 'credentials.json' '.pem'; do
  if echo "$lower" | grep -q -- "$bad"; then
    python3 -c "import json; print(json.dumps({'ok': False, 'output': 'sandbox-deny-env blocked command mentioning $bad', 'notify': 'blocked secret-touching bash'}))"
    exit 0
  fi
done

# Run under the workspace (same as core bash cwd).
out=$(cd "$ws" && bash -c "$cmd" 2>&1) || {
  code=$?
  python3 -c "import json,sys; print(json.dumps({'ok': False, 'output': sys.argv[1]}))" "$out"
  exit 0
}
python3 -c "import json,sys; print(json.dumps({'ok': True, 'output': sys.argv[1]}))" "$out"
