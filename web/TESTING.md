# Frontend testing

The frontend has two complementary suites. Fast deterministic tests run in CI;
authenticated browser regressions exercise Monaco, Ghostty, persistence, and
cross-project behavior against a running application.

## Fast suite

```bash
cd web
bun test
npm run typecheck
npm run lint
```

`bun test` covers:

- core-event reducer completeness and state transitions;
- preview URL/content rewriting;
- preview file-type support;
- persisted IDE layout schema recovery, viewport clamping, dock repair, and terminal restoration;
- editor save snapshot reconciliation and overlapping-save serialization;
- dirty project-switch decisions;
- terminal workspace identity and protocol envelopes;
- Node/Bun runtime requirement detection.

## Authenticated browser regressions

Start the application in development or production, then provide a dedicated
test account through `AUDIT_EMAIL` and `AUDIT_PASSWORD` (the scripts also read
the gitignored `web/.env.local`):

```bash
AUDIT_BASE=http://localhost:3000 npm run test:e2e
```

The browser suite runs every repaired reliability regression twice:

- delayed saves and edits made while a save is in flight;
- dirty project-switch cancel and explicit discard;
- persisted-layout recovery;
- resize interruption cleanup;
- every movable panel across every dock;
- terminal unavailable-state rendering;
- workspace-scoped same-ID terminal termination;
- termination acknowledgement and WebSocket closure.

Artifacts are written under `web/.frontend-audit/runtime/`. File fixtures and
PTYs created by the suite are removed during cleanup.

Run a subset with:

```bash
npm run test:e2e:frontend
npm run test:e2e:terminal
AUDIT_ONLY=save npm run test:e2e:frontend
AUDIT_ONLY=project npm run test:e2e:frontend
AUDIT_ONLY=layout npm run test:e2e:frontend
```

## Responsive smoke test

With a server running:

```bash
AUDIT_BASE=http://localhost:3000 npm run audit:mobile
```

Without credentials it checks the public login/setup surfaces. With the audit
credentials it additionally checks the IDE at phone, tablet, and desktop
viewports, including bottom navigation and horizontal overflow.

## Remaining environment-dependent coverage

The automated suite does not claim exhaustive browser/platform coverage.
Before high-risk releases, retain manual or hosted-matrix checks for:

- Safari and Firefox, especially native drag-and-drop and pointer capture;
- physical touch/pen input and virtual keyboards;
- real reverse proxies, idle timeouts, and process restarts;
- screen-panel backends and interactive alternate-screen terminal programs;
- long-duration memory/resource growth and very large repositories.
