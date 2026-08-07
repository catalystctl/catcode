# Frontend testing

The hub frontend (chat + git) has a fast unit suite. The authenticated browser
regression script still targets the pre-chat terminal hub and needs a rewrite
before it can gate CI again.

## Fast suite

```bash
cd web
bun test
npm run typecheck
npm run build
```

`bun test` covers:

- agent reducer (core event coverage, goal/CEO lifecycle, undo, models);
- hub layout store v2 (chat sessions per project, multi-device round-trip);
- hub layout pure helpers (legacy split-tree — still unit-tested);
- project name / clone URL validation;
- terminal protocol / mouse helpers (server WS still present for compat);
- catcode binary resolution;
- Node/Bun runtime requirement detection.

## Authenticated hub regression (stale)

```bash
AUDIT_BASE=http://localhost:3000 npm run test:e2e:hub
```

`web/scripts/hub-regression.mjs` still asserts terminal panes / presets / PTY
reattach. It will fail against the chat hub until rewritten to cover:

- login / first-run setup;
- browse-add a workspace;
- multi-session chat (new session, switch session, live SSE);
- git sidebar;
- leave/return and sign-out/sign-in session reattach;
- mobile viewport (drawer git panel).
