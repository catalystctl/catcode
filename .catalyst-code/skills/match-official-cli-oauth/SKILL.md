---
name: match-official-cli-oauth
description: When a task asks to make the harness's OAuth for a provider EXACTLY match an official CLI (gemini CLI, Claude Code, Codex), port the reference flow byte-for-byte rather than inventing constants.
---

# Match an official CLI's OAuth exactly

Use when a task says "make our <provider> OAuth match how the <official CLI> does
it" (e.g. "match gemini cli", "match claude code", "match codex"). The goal is
that a token obtained by the official CLI works here and vice-versa, and that the
consent screen / scopes / client identity are indistinguishable to the provider.

The reference CLI source is the single source of truth — do NOT guess constants.
Fetch the real values and port them. This shape has already applied to Claude
(`~/.claude` client_id, PKCE loopback) and Gemini (gemini-cli's installed-app
client). A future Codex/Responses-API task is the same shape.

## Steps

1. **Locate the reference CLI's auth source.** The repo is usually a TS/Node
   monorepo; auth logic is rarely in an obvious `auth/` dir.
   - gemini-cli (`google-gemini/gemini-cli`): `packages/core/src/code_assist/oauth2.ts`
     + `google-auth-library` (`src/auth/oauth2client.ts`, `credentials.ts`).
   - Find it via the GitHub contents API
     (`https://api.github.com/repos/<org>/<repo>/contents/<path>`) and the
     recursive tree (`git/trees/main?recursive=1`), grepping for
     `oauth|credential|login|client_id`. Delegate the source-walking to the
     `researcher` agent — it iterates fetches and returns a cited brief.
2. **Extract these EXACT values (copy raw strings, cite file+line):**
   - `client_id` / `client_secret` (the CLI's OWN client — NOT a shared SDK's
     unless the CLI uses one. gemini-cli uses `681255809395-...`, NOT gcloud's
     `32555940559...`).
   - scopes (gemini-cli = 3: `cloud-platform` + `userinfo.email` +
     `userinfo.profile`, space-joined). Missing scopes is the #1 subtle bug.
   - authorize / token / revoke / userinfo endpoints.
   - redirect_uri (loopback `http://127.0.0.1:<port>/<path>` — gemini-cli uses a
     RANDOM port + `/oauth2callback`; Claude uses a fixed port + `/callback`).
   - PKCE: present or NOT? gemini-cli's **web flow uses NO PKCE** (loopback +
     state only); its no-browser/manual-code flow DOES (S256, 128-char verifier
     with `+→~ =→_ /→-`). Don't add PKCE "for safety" if the CLI omits it — that
     isn't "exactly match."
   - `state` shape (gemini-cli = `hex(rand 32 bytes)` = 64 chars).
   - extra auth params (`access_type=offline`; is `prompt` set? `include_granted_scopes`?).
3. **Mirror the on-disk token format for interchange.** Find where the CLI stores
   tokens (gemini-cli: `~/.gemini/oauth_creds.json`; Claude: `~/.claude/.credentials.json`)
   and the EXACT JSON shape (gemini-cli = google-auth-library `Credentials`:
   `{access_token, refresh_token, token_type, expiry_date, scope, id_token?}`,
   **`expiry_date` in MILLISECONDS**). Read AND write that shape so tokens are
   interchangeable. Convert ms↔s at the boundary (our in-memory struct uses
   seconds). Keep the legacy path as a read-only fallback.
4. **Mirror token exchange + refresh form fields exactly.** e.g. gemini-cli:
   `client_secret` in the BODY (not Basic), no `code_verifier` in the web flow,
   no `x-goog-user-project` header on refresh, and PRESERVE the old
   `refresh_token` (Google doesn't always return a new one).
5. **Support the account-type variants the CLI does.** gemini-cli's "Google
   Cloud" path = same OAuth client + `gcloud auth application-default login`
   (ADC) + a `GOOGLE_APPLICATION_CREDENTIALS` service-account key (JWT-bearer
   grant, RS256). If the CLI reads ADC / service-account JSON, port that too —
   don't only support the interactive `/login`.
6. **Implement the no-browser flow for SSH/headless.** The web flow (loopback
   redirect) is USELESS over SSH — the remote machine can't open a browser the
   user can see, and the user's local browser can't reach the remote
   `127.0.0.1:<port>`. Port the CLI's manual-code flow (gemini-cli's
   `authWithUserCode`: redirect to an OOB page like
   `https://codeassist.google.com/authcode` that DISPLAYS the code + PKCE).
   Auto-select it via a headless detector (`SSH_CONNECTION`/`SSH_TTY` set, or
   non-macOS Unix with empty `DISPLAY`+`WAYLAND_DISPLAY`, or an explicit
   `*_NO_BROWSER=1` env; `=0` forces web for port-forwarded setups).
7. **Suspend/resume the manual flow via a command, NOT a blocking read.** The
   core's command loop reads stdin itself, so the manual flow CANNOT block on a
   readline (it would deadlock — no command could deliver the code). Pattern:
   `login` returns an enum `Done | AwaitingCode{pending}`; `AwaitingCode`
   stashes the PKCE verifier in `State` and returns immediately; a NEW command
   (`oauth_code{code}`) retrieves the stashed verifier and finishes the
   exchange. Restore the stashed state on exchange failure so the user retries.
   Accept EITHER a bare code OR a pasted redirect URL (extract + percent-decode
   the `code=` param — users paste the whole `?code=…&scope=…`).
   **Make the authorize URL copyable over SSH:** it's ~450 chars and will be
   CLIPPED by the TUI viewport if rendered raw (and copy grabs only the
   truncated prefix → broken OAuth, sometimes masquerading as "response_type
   missing"). Two fixes: (a) copy it to the LOCAL clipboard via **OSC 52** —
   write `\x1b]52;c;<base64(url)>\x07` to stdout; over SSH the sequence passes
   through to the user's local terminal which writes its clipboard, so the user
   just pastes into their local browser (iTerm2/kitty/WezTerm/Windows
   Terminal/gnome-terminal/alacritty; macOS Terminal.app ignores it; the
   sequence is invisible so it's safe from a Bubble Tea Update handler); (b)
   hard-wrap the URL by RUNES (not words — URLs have no spaces; char/rune-wrap,
   reusing the existing `wrapRunes`) for display as the fallback when OSC 52 is
   unavailable. The `AwaitingCode` variant need NOT carry `url` — the
   `oauth_prompt` event already emitted it.
8. **Guard the constants with a test** (`oauth::tests::<provider>_constants_match_<cli>`)
   that asserts the exact raw strings — so a future edit can't silently drift.
9. **Verify**: `cargo check --all-targets`, `cargo clippy --all-targets`
   (watch `clippy::doc_lazy_continuation` on multi-line doc comments — use real
   `-`/`*` markdown list markers, not `•`; add a blank `///` line before a
   paragraph that follows a list), `cargo fmt --check` (run
   `rustfmt --edition 2021 <file>` on just your file), `cargo test --locked`,
   `cargo build --release`. TUI side: `go vet ./... && go build ./... &&
   gofmt -l`.

## Gotchas

- **Transport ≠ OAuth identity.** The CLI may route subscription turns through a
  proprietary backend (gemini-cli → `cloudcode-pa.googleapis.com/v1internal`,
  a generateContent protocol). Our harness speaks OpenAI-compatible
  (`generativelanguage.googleapis.com/v1beta/openai`), which accepts the SAME
  OAuth access token as `Authorization: Bearer`. Match the OAuth
  identity/flow/storage exactly; document that the transport differs (porting
  the proprietary protocol is a separate, larger task).
- **Copy the EXACT auth endpoint the CLI's library uses — the newer-looking one can be wrong.** Google has two: `https://accounts.google.com/o/oauth2/auth` (v1, what `google-auth-library`'s `OAuth2Client.generateAuthUrl` defaults to) and `https://accounts.google.com/o/oauth2/v2/auth` (the GIS v2 endpoint, which LOOKS like the obvious/modern choice). The v2 GIS endpoint mishandles installed-app auth-code query params — it drops them on its internal redirect, so Google's consent page sees NO `response_type` and rejects with `Error 400: invalid_request — Required parameter is missing: response_type` EVEN THOUGH your URL contains `response_type=code`. (Token endpoint `https://oauth2.googleapis.com/token` is correct as-is.) Lesson: don't guess the "modern" URL — read it out of the reference CLI's library and guard it in the constants test.
- **Don't add a new crypto dep casually.** Service-account JWT signing needs
  RS256 → the `jsonwebtoken` crate is the clean choice (handles PEM parsing +
  RS256). Cache the exchanged SA access token in memory (`OnceLock<Mutex<…>>`)
  to avoid re-signing on every turn.
- **ms vs s expiry.** google-auth-library stores `expiry_date` in milliseconds;
  our `OAuthToken.expires_at` is seconds. Convert at the read/write boundary and
  unit-test the round-trip.
- **State must be percent-encoded** in the auth URL; so must the redirect_uri
  and the space-joined scope. A small `pct_encode` (RFC 3986 unreserved kept)
  beats hand-encoding — multiple scopes with spaces WILL break if unencoded.
- **`json!` macro + `jsonwebtoken`** must be imported/declared or the test
  binary fails to compile (`use serde_json::{json, Value};`; add `jsonwebtoken`
  to `Cargo.toml`). These look like pre-existing breakage but are just missing
  imports from the new code.
- **Distinguish internal JS function options from URL query params when
  extracting from a binary.** A compiled/minified CLI binary contains string
  constants for BOTH the `startOAuthFlow(options)` argument names AND the URL
  query parameter names — they appear in DIFFERENT clusters at different
  offsets. `isManual`, `port`, `codeChallenge`, `loginWithClaudeAi`,
  `inferenceOnly`, `loginHint`, `loginMethod`, `oauthClient`, `scopes` are
  JavaScript OPTION names — they control which `redirect_uri` is used or
  whether to open a browser, but they are NEVER sent as URL query params.
  The actual URL params (e.g. `client_id`, `response_type`, `redirect_uri`,
  `scope`, `code_challenge`, `code_challenge_method`, `state`) appear in a
  separate cluster near the URL construction code. A single `strings | grep`
  pass can conflate the two → you add `isManual=true&port=12345&code=true` to
  the authorize URL when the CLI never sends them. Cross-reference at multiple
  offsets before committing.
- **`state` goes in the AUTHORIZE URL but NOT in the token exchange body.**
  This is standard OAuth 2.0 (RFC 6749 §4.1.3) but easy to get wrong when
  trying to "match exactly" — you see `state` in the authorize URL and
  assume it should also be in the token exchange. It should NOT be. Sending
  `state` in the token exchange can cause the endpoint to reject the request
  (consuming the single-use authorization code), making ALL subsequent
  retries fail with `invalid_grant` / "Invalid 'code' in request" because
  the code was already burned on the first failed attempt. The token exchange
  body should contain ONLY: `grant_type`, `code`, `redirect_uri`, `client_id`,
  `code_verifier` (for PKCE flows). Same for refresh: `grant_type`,
  `refresh_token`, `client_id` — do NOT add `scope` unless the CLI's binary
  explicitly shows it in the refresh body.

## Reference (already-ported providers)

- **Gemini** → gemini-cli. Constants + storage in `core/src/oauth.rs`; see the
  `gemini-oauth-matches-gemini-cli` memory for the full value table. BOTH
  flows implemented: web (`authWithWeb`, loopback /oauth2callback, no PKCE)
  and no-browser (`authWithUserCode`, `codeassist.google.com/authcode` + PKCE,
  auto-selected over SSH).
- **Claude** → Claude Code CLI (`~/.claude/.credentials.json`, PKCE loopback,
  its own client_id `9d1c250a-…`). Unchanged by the gemini task.
