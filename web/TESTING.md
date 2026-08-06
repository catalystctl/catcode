# Frontend testing

The hub frontend has a fast unit suite and one authenticated browser
regression. The old IDE/chat e2e scripts were removed with those views.

## Fast suite

```bash
cd web
bun test
npm run typecheck
npm run lint
```

`bun test` covers:

- hub layout presets, split/close/ratio, pane cap, restart id swap;
- project name / clone URL validation;
- terminal protocol envelopes (`launch: "catcode"`);
- catcode binary resolution (PATH walk, override, win32 names);
- Node/Bun runtime requirement detection.

## Authenticated hub regression

Start the application in development or production, then provide a dedicated
test account through `AUDIT_EMAIL` and `AUDIT_PASSWORD` (the script also reads
the gitignored `web/.env.local`):

```bash
AUDIT_BASE=http://localhost:3000 npm run test:e2e:hub
```

`web/scripts/hub-regression.mjs` exercises:

- login / first-run setup;
- browse-add a workspace, auto-launched catcode panes;
- layout presets, close pane, git sidebar;
- leave/return and sign-out/sign-in PTY persistence;
- mobile viewport (drawer git panel, preset select, no horizontal overflow).

`update-web.sh --hub-e2e` runs the same script against the installed service.
