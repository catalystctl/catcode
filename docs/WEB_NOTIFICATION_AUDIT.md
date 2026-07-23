# CatCode Web Frontend Notification Audit

## Executive summary

The web frontend has a **single-session notification model** layered on top of a **multi-session runtime**, and the two do not match. The runtime (`HarnessBridge` + `LiveSession`) already keeps many sessions alive concurrently — one core per session, surviving tab close, running across projects — but the notification surface only ever reflects the **one session the client is currently viewing**. Everything that happens in a *background* session is invisible until you manually switch back to it.

On top of that, there is **no desktop-notification channel at all** (zero Web Notifications API usage) and **no tab-level signaling** (no `document.title`/favicon badge), so even the browser tab itself gives no signal when a turn finishes or the agent blocks waiting for you.

This audit catalogs every notification-worthy moment in the frontend, prioritizes them, and specifies the two missing channels — a persistent **in-app notification center** (distinct from the existing transient toasts) and **opt-in desktop notifications** — with concrete wiring points. The headline idea (notify across sessions/projects that another session finished or needs attention) is validated and is the single highest-value gap; it is also the natural anchor for the rest of the system.

| Priority | Count | Channel gap |
| --- | ---: | --- |
| P0 | 2 | Cross-session attention (agent blocked, invisible) |
| P1 | 6 | Cross-session completion + unfocused-tab attention |
| P2 | 4 | Subagent/goal completion, dead terminal, errors |
| P3 | 4 | Resource/compact/connection informational |

## Current state (what exists today)

### Transient toasts — `web/src/components/toasts.tsx`
- A small stack of `info` / `error` / `success` toasts, **auto-dismiss after 5s** (`TTL = 5000`), capped at the last 6 (`reducer.ts` `pushToast`).
- `aria-live="polite"`; docked (inside `.chat-panel`) or viewport-fixed.
- Fired by the reducer on: `ready` (model-mismatch warning), `approval_expired`, `ask_request`, `sudo_request`, `compacting`, `compacted`, `http_retry`, `usage` (not-available / summary).
- **Shape mismatch:** 5s auto-dismiss is fine for ephemeral status ("compacting…") but wrong for "the agent is blocked waiting for your approval" — if you are in another session, the toast fires on a stream you are not subscribed to and you never see it at all; even on the viewed session it vanishes before you return.

### Attention UI — `web/src/components/chat.tsx`
- `Approval`, `AskFlyout`, `SudoPrompt`, `IntercomPrompt`, `OauthPromptBanner` render from the **single viewed session's** `state.pendingApproval / pendingAsk / pendingSudo / pendingIntercom / pendingOauth` (`chat.tsx:749–784`).
- `streamingSessionFile = state.streaming ? state.currentSessionFile : null` (`chat.tsx:670`) — the sidebar streaming indicator marks **only the session you are viewing**.
- Net: every attention surface is scoped to the active session. There is no concept of "another session needs you."

### Multi-session runtime — `web/src/server/live-session.ts` + `web/src/server/core-bridge.ts`
- `HarnessBridge` holds a pool of `LiveSession`s keyed by session-file path. Each has its own `CoreProcess`, `AgentState`, and SSE subscribers; switching projects **never tears down other live sessions**.
- Each `LiveSession` already exposes `.ready`, `.streaming`, `.sinkCount`, and its `state.pendingApproval/Ask/Sudo/Intercom/Oauth`. `HarnessBridge.liveSessions()` returns `{ workspace, sessionFile, running, streaming, viewers }`.
- Idle sessions are reaped after 2h (`IDLE_GC_MS`), but **mid-turn sessions are never reaped** — so a background session can genuinely sit blocked on approval for an hour and you would not know.
- **Cross-session broadcast is metadata-only.** `broadcastSessions` fans the `sessions` CoreEvent to siblings, but that event is pure disk metadata (`SessionEntry`: name, mtime, size, title, path, messages, current, pinned). It carries **no live status** — no `streaming`, no `needsAttention`, no `lastMessagePreview`. Attention/completion events (`approval_request`, `ask_request`, `sudo_request`, `done`, `aborted`, `goal_completion_summary`, intercom escalation, `error`, core death via `onDead`) are fanned **only to that session's own subscribers** (`LiveSession.fanout → sinks`).

### What does not exist
- `new Notification` / `Notification.requestPermission` / service-worker push / audio — **none** (grep returned empty).
- `document.title` mutation or favicon badge on streaming / done / needs-attention — **none** (only a middleware `favicon.ico` route exclusion and a preview-proxy `<title>`).
- Any persistent, dismissible notification feed or unread badge.

## The core gap (diagnosis)

> The server already *knows* which sessions are streaming, finished, or blocked on a human gate. It just does not *tell the client* — and the client has no channel to receive it except for the single session it is viewing.

Three independent deficits compound:

1. **Wire gap** — attention/completion events are session-scoped. A client viewing session A is not subscribed to session B's stream, so B's `approval_request`/`done`/etc. never reach it. `broadcastSessions` only relays the disk-metadata `sessions` list, never live status.
2. **Model gap** — `SessionEntry` has no status fields, so even if events were relayed the sidebar has nowhere to render "blocked / finished / unread."
3. **Channel gap** — no desktop notifications and no tab badge, so when the tab is hidden or you are in another project, there is no signal at all.

The user's headline idea closes all three at once for the completion case; doing it properly for the attention case is strictly more valuable because there the agent is *blocked*.

## Notification-worthy moments — catalog

Channels: **C** = persistent in-app notification center (new), **T** = transient toast (existing), **D** = desktop notification (new, opt-in), **B** = browser tab badge / `document.title` (new).

| ID | Moment | Source event / state | Fires when | Channels | Pri |
| --- | --- | --- | --- | --- | --- |
| **N1** | Background session needs tool approval | `approval_request` on a non-viewed session | agent blocked, awaiting yes/no/always | C, D, B | **P0** |
| **N2** | Background session asked a question | `ask_request` on a non-viewed session | agent blocked on user answers | C, D, B | **P0** |
| **N3** | Background session needs sudo | `sudo_request` on a non-viewed session | agent blocked on approve/decline | C, D, B | P1 |
| **N4** | Background subagent escalated to supervisor | `pendingIntercom` / `need_decision` | subagent blocked on a decision | C, D, B | P1 |
| **N5** | Background session needs OAuth paste-code | `pendingOauth` | agent blocked on pasteable code | C, D, B | P1 |
| **N6** | Background goal plan ready for review | `goal_plan` / `approveGoalPlan` | goal mode paused for plan approval | C, D | P1 |
| **N7** | **Background session turn finished** *(the headline idea)* | `done` on a non-viewed session | turn complete, may want to return | C, D, B | P1 |
| **N8** | Background session aborted / errored / core died | `aborted`, `error`, `onDead` | run ended unexpectedly | C, D, B | P1 |
| **N9** | Background goal/mission completed | `goal_completion_summary` | long-running goal finished | C, D | P2 |
| **N10** | Background subagent run completed | `subagentRuns` → `completed` | a delegated task finished (opt-in) | C | P2 |
| **N11** | Active session needs attention while tab unfocused | any of N1–N6 on the viewed session, but `document.hidden` or different project | you stepped away mid-block | D, B | P1 |
| **N12** | Active session turn finished while tab unfocused | `done` on viewed session + tab hidden | "your turn is done" when away | D, B | P2 |
| **N13** | Stream disconnect / 502 hammer | `lastStreamErrToastRef` path | connection lost | D (when hidden) | P3 |
| **N14** | Provider usage / quota hit | `usage` (not available) | rate-limited / out of quota | T, D (hidden) | P3 |
| **N15** | Context compacted | `compacted` | informational, already toasts | T | P3 |
| **N16** | Sandbox prepare finished (long-running) | sandbox status → ready | image/runtime download done | C, D | P3 |
| **N17** | Self-update available | self-update check | a newer version exists | C | P3 |
| **N18** | Terminal died/disconnected silently | terminal WS `terminated`/close (see `WEB-003`) | PTY lost but tab looks alive | C, T, B | P2 |

### Notes on the catalog
- **P0 (N1–N2)** are blocking: the agent cannot proceed until a human acts, and today that human gets no signal if they are in another session. This is worse than the completion case (N7) because a finished turn simply waits, while a blocked turn wastes a live core and stalls work.
- **N7 (the headline idea)** is the cleanest entry point: `done` already triggers a `list_sessions`/`stats` refresh on the origin session, so the bridge already has the transition — it just needs to fan a *status* event, not only a *metadata* list.
- **N10** and **N16** can be noisy; gate behind per-category opt-in toggles.
- **N18** overlaps with the existing `WEB-003` reliability finding; a notification is the user-facing half of that fix.

## Recommended architecture

### Two distinct in-app channels (do not overload toasts)
1. **Transient toasts (keep as-is)** — ephemeral status only: compacting, retrying, usage summary, "session core exited." 5s auto-dismiss.
2. **Notification center / feed (new)** — persistent, dismissible, unread-badge, click-to-navigate. For everything in the catalog above `T`. Lives as a bell icon in the header with an unread count; items show session title + project + reason + timestamp and, on click, call `agent.loadSession(path)` (cross-project) or `switchWorkspace`. This is the persistent half the user's idea needs — a finished/ blocked background session must survive longer than 5s.

### Desktop channel (new, opt-in)
- `Notification.requestPermission()` behind an explicit Settings toggle ("Desktop notifications"). Persist the granted/denied state; if `denied`, show guidance and stop prompting.
- Fire for P0/P1 attention + completion items. Gate on **focus state**: emit a desktop notification only when `document.visibilityState === "hidden"` **or** the originating session is not the viewed one (the "in another session/project" case). Never fire a desktop notification for the session you are actively looking at.
- Dedup with `tag` per session (e.g. `session:<file>:attention`) so repeated approvals coalesce into "Session X: 3 approvals waiting" rather than spamming.
- `requireInteraction: true` for P0 blocking items so they do not auto-vanish; plain auto-dismiss for completion.

### Tab badge (new, no permission needed)
- `document.title` = `"(2) CatCode"` with a small favicon dot when unread attention exists; clear on focus. Cheap, high-signal, works even if desktop notifications are denied.
- Distinguish "blocked" (e.g. red dot / `(⚠)`) from "finished" (neutral dot) so the user knows whether to switch back urgently.

### The enabling change: cross-session status broadcast
The bridge already holds every session's status. The minimal, reconnect-safe change (matching the existing `inject`/`broadcastSessions` pattern) is to have the bridge **synthesize a lightweight `session_status` event and fan it to every session's subscribers** whenever a session's status *changes*:

- Add live-status fields to the session model — either extend `SessionEntry` or add a parallel `LiveSessionSummary`: `streaming: boolean`, `needsAttention: boolean` (derived from `pendingApproval || pendingAsk || pendingSudo || pendingIntercom || pendingOauth`), `attentionKind`, `lastEventAt`, `lastMessagePreview`, `running: boolean`.
- Add `SessionCallbacks.onStatusChange(workspace, summary)` to `LiveSession`'s callbacks; call it on the relevant transitions: `done`/`aborted` (completion), `approval_request`/`ask_request`/`sudo_request`/intercom/oauth (attention entered), and the matching clears (approval given, etc.), plus `onDead`.
- `HarnessBridge` fans the resulting `session_status` event to **all** live sessions (all workspaces), not just siblings — so viewing project A's session surfaces project B's blocked session too. (Today `broadcastSessions` is workspace-scoped; cross-project status needs a global fan, like `broadcastProjects`.)
- Client side: a new `useNotifications` hook (or extend `useAgent`) keeps a `Map<sessionFile, LiveSessionSummary>` from these broadcasts and derives (a) sidebar live badges per session, (b) notification-center items, (c) the tab badge, (d) desktop-notification triggers — all from status *transitions* (e.g. `needsAttention` false→true).

This is the single change that makes the user's idea — and N1–N10 — possible, because today no client ever learns about a non-viewed session's lifecycle.

## Concrete wiring points

| Concern | File / symbol |
| --- | --- |
| Toast system to keep as the ephemeral channel | `web/src/components/toasts.tsx` (`Toasts`, `ToastItem`, `TTL`) |
| `pushToast` + existing fire sites | `web/src/lib/reducer.ts` (`pushToast` ~L107; cases `approval_expired`, `ask_request`, `sudo_request`, `compacting`, `compacted`, `http_retry`, `usage`, `ready`) |
| Attention modals scoped to active session | `web/src/components/chat.tsx:749–784` (`Approval`, `IntercomPrompt`, `AskFlyout`, `SudoPrompt`, `OauthPromptBanner`) |
| Sidebar only marks viewed session streaming | `web/src/components/chat.tsx:670` (`streamingSessionFile`); `web/src/components/sidebar.tsx:281` (`matchesSession`) |
| Session model lacking live status | `web/src/lib/types.ts` (`SessionEntry`) — add `streaming`/`needsAttention`/`attentionKind`/`lastEventAt`/`lastMessagePreview` |
| Per-session state already has attention flags | `web/src/lib/types.ts` (`AgentState.pendingApproval/Ask/Sudo/Intercom/Oauth`) |
| Runtime already exposes per-session status | `web/src/server/live-session.ts` (`.ready`, `.streaming`, `.sinkCount`, `state`); `web/src/server/core-bridge.ts` (`liveSessions()`) |
| Cross-session fan-out to extend | `web/src/server/core-bridge.ts` (`broadcastSessions` — workspace-scoped, metadata-only; add global `session_status` fan like `broadcastProjects`) |
| Transition hooks to emit status | `web/src/server/live-session.ts` `onCoreEvent` (already branches `sessions`, `done`, `aborted`, `ready`); add `SessionCallbacks.onStatusChange` |
| Client hook to extend / add `useNotifications` | `web/src/lib/use-agent.ts` (holds one session's `AgentState`; add cross-session summary map from broadcasts) |
| Desktop permission + tab badge (new) | new module, e.g. `web/src/lib/notifications.ts` (`Notification.requestPermission`, `document.title`, favicon dot); Settings toggle in `web/src/components/settings.tsx` |
| Design tokens to reuse (do not reinvent) | `toasts.tsx` palette — `border-l-danger/success/accent`, `bg-ink-900`, `text-ink-200`, `shadow-elev-2`, `animate-fade-in` |

## Suggested rollout order

1. **Tab badge (N7/N1 unfocused)** — smallest, no permission, immediate value: mutate `document.title` + favicon on `done`/attention. Pure client change.
2. **Cross-session `session_status` broadcast + sidebar live badges** — the enabling server change; surfaces N1/N7 in the sidebar for every session, no new UI surface yet.
3. **In-app notification center (persistent feed)** — the bell + unread list with click-to-navigate; absorbs N1–N10. Replaces the "toast that vanishes before you return" failure mode.
4. **Desktop notifications (opt-in)** — layer the `Notification` API on the same transitions, gated by focus state and per-category toggles; full realization of the headline idea.

Steps 1–3 deliver the user's idea entirely in-app; step 4 adds the OS-level channel "when allowed."
