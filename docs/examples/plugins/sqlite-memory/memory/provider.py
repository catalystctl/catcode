#!/usr/bin/env python3
"""SQLite memory_provider for Catalyst Code.

Actions: inject, save, append, list, forget, compact_append.
DB path: ~/.config/catalyst-code/sqlite-memory/<workspace-hash>.db
"""
import hashlib
import json
import os
import re
import sqlite3
import sys
import time


def _emit(obj):
    sys.stdout.write(json.dumps(obj))
    sys.stdout.write("\n")


def _ws_hash(workspace: str) -> str:
    return hashlib.sha256(os.path.realpath(workspace or "").encode()).hexdigest()[:16]


def _db_path(workspace: str) -> str:
    home = os.path.expanduser("~")
    base = os.path.join(home, ".config", "catalyst-code", "sqlite-memory")
    os.makedirs(base, exist_ok=True)
    return os.path.join(base, f"{_ws_hash(workspace)}.db")


def _conn(workspace: str):
    path = _db_path(workspace)
    c = sqlite3.connect(path)
    c.execute(
        """
        CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            content TEXT NOT NULL,
            type TEXT DEFAULT 'note',
            description TEXT DEFAULT '',
            scope TEXT DEFAULT 'workspace',
            updated_at INTEGER NOT NULL
        )
        """
    )
    return c


def _slug(name: str) -> str:
    s = re.sub(r"[^a-zA-Z0-9_-]+", "-", name.strip().lower()).strip("-")
    return s or f"mem-{int(time.time())}"


def main():
    try:
        ctx = json.loads(sys.stdin.read() or "{}")
    except Exception as e:
        _emit({"ok": False, "output": f"bad json: {e}"})
        return

    action = ctx.get("action") or ""
    args = ctx.get("args") or {}
    workspace = ctx.get("workspace") or ""

    try:
        db = _conn(workspace)
    except Exception as e:
        _emit({"ok": False, "output": f"db open failed: {e}"})
        return

    try:
        if action == "inject":
            rows = db.execute(
                "SELECT name, type, description, content FROM memories ORDER BY updated_at DESC LIMIT 80"
            ).fetchall()
            if not rows:
                _emit({"ok": True, "injection": ""})
                return
            lines = ["[MEMORY — sqlite provider]"]
            for name, typ, desc, content in rows:
                blurb = (desc or content or "")[:120].replace("\n", " ")
                lines.append(f"- **{name}** ({typ or 'note'}): {blurb}")
            _emit({"ok": True, "injection": "\n".join(lines)})
            return

        if action in ("save", "append"):
            name = (args.get("name") or "").strip()
            content = args.get("content") or ""
            if not name or not content:
                _emit({"ok": False, "output": "name and content required"})
                return
            mid = _slug(name)
            typ = args.get("type") or "note"
            desc = args.get("description") or ""
            scope = args.get("scope") or "workspace"
            now = int(time.time())
            existing = db.execute(
                "SELECT content FROM memories WHERE id=?", (mid,)
            ).fetchone()
            if action == "append" and existing:
                content = (existing[0] or "") + "\n" + content
            db.execute(
                """
                INSERT INTO memories(id, name, content, type, description, scope, updated_at)
                VALUES(?,?,?,?,?,?,?)
                ON CONFLICT(id) DO UPDATE SET
                  content=excluded.content,
                  type=excluded.type,
                  description=excluded.description,
                  scope=excluded.scope,
                  updated_at=excluded.updated_at
                """,
                (mid, name, content, typ, desc, scope, now),
            )
            db.commit()
            _emit({"ok": True, "output": f"saved {mid}", "id": mid})
            return

        if action == "list":
            rows = db.execute(
                "SELECT id, name, type, description, scope FROM memories ORDER BY name"
            ).fetchall()
            entries = [
                {
                    "id": r[0],
                    "name": r[1],
                    "type": r[2],
                    "description": r[3],
                    "scope": r[4],
                }
                for r in rows
            ]
            lines = [f"{e['id']}: {e['name']} ({e['type']})" for e in entries]
            _emit(
                {
                    "ok": True,
                    "output": "\n".join(lines) if lines else "(empty)",
                    "entries": entries,
                }
            )
            return

        if action == "forget":
            mid = (args.get("id") or "").strip()
            if not mid:
                _emit({"ok": False, "output": "id required"})
                return
            db.execute("DELETE FROM memories WHERE id=?", (mid,))
            db.commit()
            _emit({"ok": True, "output": f"forgot {mid}"})
            return

        if action == "compact_append":
            content = args.get("content") or ""
            name = args.get("name") or "compact-extract"
            mid = _slug(name)
            now = int(time.time())
            existing = db.execute(
                "SELECT content FROM memories WHERE id=?", (mid,)
            ).fetchone()
            body = ((existing[0] + "\n") if existing else "") + content
            cap = int(args.get("cap_bytes") or 32000)
            if len(body.encode()) > cap:
                body = body[-cap:]
            db.execute(
                """
                INSERT INTO memories(id, name, content, type, description, scope, updated_at)
                VALUES(?,?,?,?,?,?,?)
                ON CONFLICT(id) DO UPDATE SET content=excluded.content, updated_at=excluded.updated_at
                """,
                (mid, name, body, "note", "compaction extract", "workspace", now),
            )
            db.commit()
            _emit({"ok": True, "output": f"compact_append {mid}"})
            return

        _emit({"ok": False, "output": f"unknown action: {action}"})
    except Exception as e:
        _emit({"ok": False, "output": str(e)})
    finally:
        db.close()


if __name__ == "__main__":
    main()
