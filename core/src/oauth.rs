//! OAuth subscription login — performs the login here (so you don't need the
//! official CLI).
//!
//! All functions are best-effort: a missing file, a parse error, a network
//! failure, or a wrong constant returns `None` / `Err`, so callers fall back to
//! the API-key path with NO regression. This lets a user use their subscription
//! (ChatGPT Plus/Pro→Codex, Google One AI→Gemini, Claude Pro/Max) without a
//! separate pay-as-you-go API key.
//!
//! `/login` performs the OAuth flow here (no official CLI needed):
//!  - Gemini: **byte-for-byte the same OAuth installed-app flow the official
//!    `gemini` CLI uses** — same public client_id/secret, the exact three
//!    scopes (`cloud-platform` + `userinfo.email` + `userinfo.profile`),
//!    `state` CSRF, `access_type=offline`. Two paths, auto-selected by
//!    environment (`likely_headless`):
//!    - **web flow** (local machine) — fixed loopback
//!      `http://localhost:51121/oauth-callback` redirect with PKCE — this is
//!      the only URI registered for the Antigravity client (a different port
//!      or path causes Google's `redirect_uri_mismatch`). Binds on both IPv4
//!      and IPv6 because browsers may resolve `localhost` to `::1`; completes
//!      synchronously.
//!    - **no-browser flow** (SSH / headless, or `CATALYST_CODE_NO_BROWSER=1`)
//!      — **same** registered redirect URI (Antigravity does NOT register the
//!      gemini-cli OOB page `codeassist.google.com/authcode`, so using it
//!      yields Google's `redirect_uri_mismatch`). User opens the URL on any
//!      device; after consent Google redirects to `localhost:51121` (page may
//!      fail to load — that's fine). Paste the `code` query param (or full
//!      redirect URL) via `/oauth-code`. Optional: SSH `-L 51121:127.0.0.1:51121`
//!      + `CATALYST_CODE_NO_BROWSER=0` for automatic loopback capture.
//!      Tokens are written to `~/.gemini/oauth_creds.json` in the
//!      `google-auth-library` `Credentials` shape (same path Antigravity / gemini-cli
//!      use). Works for **regular Google accounts** (personal Gmail /
//!      Google One AI) **and Google Cloud / Workspace identities** alike — the
//!      Google consent screen lets the user pick the account.
//!  - Google *Cloud* accounts that prefer ADC: `gcloud auth
//!    application-default login` (`~/.config/gcloud/application_default_
//!    credentials.json`) and a service-account key file pointed at by
//!    `GOOGLE_APPLICATION_CREDENTIALS` are read and refreshed here too, so a
//!    headless Cloud workload needs no `/login`.
//!  - Claude: Anthropic **authorization-code + PKCE** matching Claude Code's
//!    subscription OAuth flow — local machines use `http://localhost:<port>/callback`
//!    loopback; SSH/headless sessions use Anthropic's hosted manual callback
//!    page and `/oauth-code`, so no port forwarding is required. Uses Claude
//!    Code's public client_id, authorize/token endpoints, scopes, JSON token
//!    exchange, and `oauth-2025-04-20` API beta header.
//!  - xAI Grok: **dual auth** — `XAI_API_KEY` (console.x.ai) OR **OAuth 2.0
//!    device-code** against `auth.x.ai` (same public client + scopes Hermes /
//!    Grok CLI use for SuperGrok / X Premium+). Works on local machines and
//!    over SSH/headless (prints verification URL + user code; polls until
//!    approved). Tokens refresh automatically against
//!    `https://auth.x.ai/oauth2/token`.
//!  - Qwen Code: **device-code + PKCE** against `chat.qwen.ai` (same public
//!    client_id / endpoints 9router uses). Works on local machines and over
//!    SSH/headless. Tokens refresh automatically; chat goes to
//!    `portal.qwen.ai/v1`.
//!
//! Tokens from `/login` are stored at `~/.gemini/oauth_creds.json` (Google,
//! 0600) or `~/.config/catalyst-code/oauth/<id>.json` (OpenAI/Claude/xAI, 0600)
//! and refreshed in place. OpenAI/Codex deliberately does NOT reuse the
//! official Codex CLI's `~/.codex/auth.json`; use this app's OAuth flow so the
//! selected account/subscription is explicit.
//!
//! Note on endpoints: `gemini` CLI routes its OAuth/subscription turns through
//! the Code Assist backend (`cloudcode-pa.googleapis.com`, a different request
//! shape). This harness speaks the OpenAI-compatible Gemini shim
//! (`generativelanguage.googleapis.com/v1beta/openai`), which accepts the same
//! OAuth access token (cloud-platform scope) as `Authorization: Bearer`. The
//! OAuth *identity* matches gemini-cli exactly; the transport stays OpenAI-
//! compatible. To use a Gemini API key instead, export `GEMINI_API_KEY`.

use crate::config::{home_dir, ProviderKind, ResolvedProvider};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// --- Google (Gemini / Antigravity) constants ----------------------------------
// Antigravity (Google's agentic IDE) OAuth client — NOT the older gemini-cli
// client. Reverse-engineered from the official `agy` binary + the community
// opencode-antigravity-auth plugin. The Antigravity client unlocks Gemini 3 /
// Claude-via-Antigravity models and the daily sandbox Code Assist gateway;
// gemini-cli's client is limited to the older Gemini 2.x Code Assist quota.
// The secret is an "installed application" secret (not treated as confidential
// per Google's OAuth2 installed-app docs).
const GOOGLE_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GOOGLE_CLIENT_SECRET: &str = "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";
// google-auth-library's OAuth2Client uses the v1 authorize endpoint (NOT the
// `/o/oauth2/v2/auth` GIS endpoint — GIS mishandles installed-app auth-code
// params and Google's consent rejects with "response_type missing").
const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Primary Code Assist / Antigravity API endpoint (daily sandbox — same order
/// CLIProxy / Antigravity itself prefer). Falls back to prod when needed.
/// `generativelanguage.googleapis.com` only accepts API keys; OAuth tokens
/// authenticate against `*/v1internal` which proxies Gemini + Claude.
const CODE_ASSIST_BASE_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal";
/// Production Code Assist endpoint (used as loadCodeAssist fallback).
const CODE_ASSIST_PROD_BASE_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal";
/// Autopush sandbox (last-resort fallback).
const CODE_ASSIST_AUTOPUSH_BASE_URL: &str =
    "https://autopush-cloudcode-pa.sandbox.googleapis.com/v1internal";
/// Fallback project id when loadCodeAssist returns none (business/workspace
/// accounts). Matches Antigravity / CLIProxy.
const ANTIGRAVITY_DEFAULT_PROJECT_ID: &str = "rising-fact-p41fc";
/// Antigravity client version baked into User-Agent / Client-Metadata.
const ANTIGRAVITY_VERSION: &str = "1.18.3";
// Antigravity scopes (cloud-platform + identity + cclog + experiments). The
// two extra scopes beyond gemini-cli are what unlock Antigravity-only models.
const GOOGLE_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];
fn google_scope_string() -> String {
    GOOGLE_SCOPES.join(" ")
}
// Antigravity / Gemini Code Assist post-redirect success page.
const GOOGLE_SUCCESS_URL: &str =
    "https://developers.google.com/gemini-code-assist/auth_success_gemini";
/// Only redirect URI registered for the Antigravity OAuth client. A different
/// port or path causes Google's `Error 400: redirect_uri_mismatch`. Used by
/// both the web (loopback listener) and SSH/manual (`/oauth-code`) flows.
/// NOTE: Do NOT use gemini-cli's OOB page (`https://codeassist.google.com/authcode`)
/// — that URI is only allow-listed for gemini-cli's client id, not Antigravity's.
const ANTIGRAVITY_REDIRECT_PORT: u16 = 51121;
const ANTIGRAVITY_REDIRECT_URI: &str = "http://localhost:51121/oauth-callback";

// --- OpenAI (Codex / ChatGPT subscription) constants ---------------------------
// Exact Codex CLI OAuth identity (openai/codex login crate). ChatGPT
// subscription tokens are accepted by chatgpt.com/backend-api/codex, not the
// public api.openai.com chat-completions endpoint.
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const OPENAI_REDIRECT_PORT: u16 = 1455;
const OPENAI_DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
const OPENAI_DEVICE_CALLBACK: &str = "https://auth.openai.com/deviceauth/callback";
// The `originator` value the official `codex` CLI sends as a default header on
// every request (codex-rs/login/src/auth/default_client.rs: DEFAULT_ORIGINATOR).
// OpenAI's gateway uses it to identify/route the first-party client.
const OPENAI_ORIGINATOR: &str = "codex_cli_rs";
// Codex's registered loopback redirect ports. Port 1455 is preferred; 1457 is
// the registered fallback (codex-rs/login/src/server.rs: FALLBACK_PORT) used
// when 1455 is already bound. Both are in the CLI's redirect-URI allow-list.
const OPENAI_REDIRECT_FALLBACK_PORT: u16 = 1457;

// --- Anthropic (Claude) constants ---------------------------------------------
// Claude Code's public OAuth identity and endpoints (verified against the
// official `@anthropic-ai/claude-code` native package). On any failure we fall
// back to the API-key path.
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_LEGACY_CLIENT_ID: &str = "https://claude.ai/oauth/claude-code-client-metadata";
const CLAUDE_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_MANUAL_REDIRECT_URL: &str = "https://platform.claude.com/oauth/code/callback";
const CLAUDE_SCOPES: &[&str] = &[
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];
fn claude_scope_string() -> String {
    CLAUDE_SCOPES.join(" ")
}

// --- xAI (Grok SuperGrok / X Premium+) constants ------------------------------
// Public OAuth client used by Hermes Agent and the Grok CLI for SuperGrok /
// X Premium+ subscription access (or XAI_API_KEY). Verified against
// hermes_cli/auth.py and live OIDC discovery at auth.x.ai.
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
/// xAI access tokens are short-lived (~6h). Refresh up to an hour early so
/// idle sessions don't hit a 401 on the next turn.
const XAI_REFRESH_SKEW_SECS: u64 = 3600;

// --- Qwen Code OAuth (device-code + PKCE) ------------------------------------
// Constants match 9router's open-sse/providers/registry/qwen.js PROVIDER_OAUTH.
const QWEN_CLIENT_ID: &str = "f0304373b74a44d2b584a3fb70ca9e56";
const QWEN_DEVICE_CODE_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/device/code";
const QWEN_TOKEN_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/token";
const QWEN_SCOPE: &str = "openid profile email model.completion";
/// Qwen access tokens are short-lived; refresh a few minutes early.
const QWEN_REFRESH_SKEW_SECS: u64 = 300;

// --- GitHub Copilot OAuth (device-code → Copilot session token) --------------
// Constants and required request headers match 9router's github provider.
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const GITHUB_SCOPE: &str = "read:user";
const GITHUB_REFRESH_SKEW_SECS: u64 = 60;

// --- Kimi Coding OAuth (device-code) -----------------------------------------
const KIMI_CODING_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_CODING_DEVICE_CODE_URL: &str = "https://auth.kimi.com/api/oauth/device_authorization";
const KIMI_CODING_TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";
const KIMI_CODING_REFRESH_SKEW_SECS: u64 = 300;

// --- Kilo Code device authorization -------------------------------------------
const KILOCODE_INITIATE_URL: &str = "https://api.kilo.ai/api/device-auth/codes";
const KILOCODE_POLL_BASE_URL: &str = "https://api.kilo.ai/api/device-auth/codes";
const KILOCODE_REFRESH_SKEW_SECS: u64 = 300;

// --- Cline / ClinePass OAuth (browser authorize → token paste/exchange) ------
// Matches 9router open-sse/providers/registry/{cline,clinepass}.js.
const CLINE_AUTHORIZE_URL: &str = "https://api.cline.bot/api/v1/auth/authorize";
const CLINE_TOKEN_URL: &str = "https://api.cline.bot/api/v1/auth/token";
const CLINE_REFRESH_URL: &str = "https://api.cline.bot/api/v1/auth/refresh";
/// Cline's extension OAuth redirects here with a base64-encoded token payload
/// (or a code to exchange). Users paste the redirect URL or raw code via
/// `/oauth-code`.
const CLINE_REDIRECT_URI: &str = "http://localhost:54321/callback";
const CLINE_REFRESH_SKEW_SECS: u64 = 300;

// --- Kimchi browser-token login ----------------------------------------------
// Matches 9router kimchi.js: open app.kimchi.dev/cli-auth, paste the token.
const KIMCHI_WEB_APP_URL: &str = "https://app.kimchi.dev";
const KIMCHI_VALIDATION_URL: &str = "https://api.cast.ai/v1/llm/openai/supported-providers";
const KIMCHI_USERINFO_URL: &str = "https://app.kimchi.dev/api/v1/me";

// --- Tencent CodeBuddy CN (device-style state poll) --------------------------
// Matches 9router codebuddy-cn.js + refreshCodebuddyToken.
const CODEBUDDY_STATE_URL: &str = "https://copilot.tencent.com/v2/plugin/auth/state";
const CODEBUDDY_TOKEN_URL: &str = "https://copilot.tencent.com/v2/plugin/auth/token";
const CODEBUDDY_REFRESH_URL: &str = "https://copilot.tencent.com/v2/plugin/auth/token/refresh";
const CODEBUDDY_USER_AGENT: &str = "CLI/2.63.2 CodeBuddy/2.63.2";
const CODEBUDDY_PLATFORM: &str = "CLI";
const CODEBUDDY_REFRESH_SKEW_SECS: u64 = 300;

// --- iFlow AI OAuth (authorization_code + Basic client auth) -----------------
// Matches 9router iflow.js. Chat uses the account apiKey (from userInfo), not
// the short-lived OAuth access token, plus per-request HMAC headers.
const IFLOW_CLIENT_ID: &str = "10009311001";
const IFLOW_CLIENT_SECRET: &str = "4Z3YjXycVsQvyGF1etiNlIBB4RsqSDtW";
const IFLOW_AUTHORIZE_URL: &str = "https://iflow.cn/oauth";
const IFLOW_TOKEN_URL: &str = "https://iflow.cn/oauth/token";
const IFLOW_USERINFO_URL: &str = "https://iflow.cn/api/oauth/getUserInfo";
const IFLOW_REDIRECT_URI: &str = "http://localhost:1455/callback";
const IFLOW_REFRESH_SKEW_SECS: u64 = 300;

/// A prompt shown to the user during an interactive OAuth login. Emitted as an
/// `oauth_prompt` event by the core handler.
#[derive(Clone, Debug, Serialize)]
pub struct OAuthPrompt {
    /// URL the user must visit (authorize endpoint, or device verification URL).
    pub url: String,
    /// The code to enter at the URL (device flow only; None for browser flows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable instructions.
    pub message: String,
}

/// An in-memory OAuth token. For Google we persist the `google-auth-library`
/// `Credentials` shape (see `GeminiCreds`); this struct is the working
/// representation used by refresh. For Claude we persist this struct directly.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct OAuthToken {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Absolute unix-SECONDS expiry (best-effort; 0 = unknown).
    #[serde(default)]
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// "google" | "claude" — which refresh path to use.
    #[serde(default)]
    pub kind: String,
    /// OpenID Connect id_token (Google returns one because the userinfo scopes
    /// imply OIDC). Persisted for interchangeability with gemini-cli, which
    /// uses it for account identity. Not required for API calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// Provider-specific opaque state required to refresh or use a token.
    /// GitHub Copilot needs the GitHub OAuth token to mint its short-lived
    /// Copilot session token; Kilo needs the organization ID from device auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

/// On-disk shape Google's `google-auth-library` (and the `gemini` CLI) writes
/// to `~/.gemini/oauth_creds.json`. `expiry_date` is in MILLISECONDS. We match
/// this exactly so our token file is interchangeable with gemini-cli's.
#[derive(Serialize, Deserialize, Clone, Default)]
struct GeminiCreds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_type: Option<String>,
    /// MILLISECONDS since epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expiry_date: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id_token: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --- credential file locations ------------------------------------------------

fn gcloud_adc_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".config/gcloud/application_default_credentials.json"))
}

fn claude_creds_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".claude/.credentials.json"))
}

fn codex_auth_path() -> Option<PathBuf> {
    // Use our own OAuth store. Do NOT auto-detect/reuse the official Codex
    // CLI's ~/.codex/auth.json; users should sign in here via this app's OAuth
    // flow so account selection and refresh state are under our control.
    Some(home_dir()?.join(".config/catalyst-code/oauth/openai.json"))
}

/// Where the `gemini` CLI (and now we) store Google OAuth tokens.
fn gemini_creds_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".gemini/oauth_creds.json"))
}

/// Our legacy pre-match token path (kept as a read-only fallback so an existing
/// login is not silently lost). New logins always write `~/.gemini/oauth_creds.json`.
fn legacy_gemini_token_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".config/catalyst-code/oauth/gemini.json"))
}

/// Where WE store Anthropic OAuth tokens obtained via `/login`.
fn stored_token_path(provider: &str) -> Option<PathBuf> {
    Some(home_dir()?.join(format!(".config/catalyst-code/oauth/{provider}.json")))
}

fn stored_token_dir() -> Option<PathBuf> {
    Some(home_dir()?.join(".config/catalyst-code/oauth"))
}

/// True when Gemini auth is available: our/gemini-cli token OR gcloud ADC OR a
/// `GOOGLE_APPLICATION_CREDENTIALS` file exists.
pub fn has_google_creds() -> bool {
    gemini_creds_path().map(|p| p.exists()).unwrap_or(false)
        || legacy_gemini_token_path()
            .map(|p| p.exists())
            .unwrap_or(false)
        || gcloud_adc_path().map(|p| p.exists()).unwrap_or(false)
        || std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|p| std::path::Path::new(&p).exists())
            .unwrap_or(false)
}

/// True when Claude auth is available: our stored token OR the CLI's creds exist.
pub fn has_claude_creds() -> bool {
    stored_token_path("anthropic")
        .map(|p| p.exists())
        .unwrap_or(false)
        || claude_creds_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn has_codex_creds() -> bool {
    codex_auth_path().map(|p| p.exists()).unwrap_or(false)
}

/// True when an xAI SuperGrok OAuth token file exists
/// (`~/.config/catalyst-code/oauth/xai.json`).
pub fn has_xai_creds() -> bool {
    stored_token_path("xai")
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// True when a Qwen Code OAuth token file exists
/// (`~/.config/catalyst-code/oauth/qwen.json`).
pub fn has_qwen_creds() -> bool {
    stored_token_path("qwen")
        .map(|p| p.exists())
        .unwrap_or(false)
}

fn has_stored_creds(provider: &str) -> bool {
    stored_token_path(provider)
        .map(|p| p.exists())
        .unwrap_or(false)
}

pub fn has_github_creds() -> bool {
    has_stored_creds("github")
}
pub fn has_kimi_coding_creds() -> bool {
    has_stored_creds("kimi-coding")
}
pub fn has_kilocode_creds() -> bool {
    has_stored_creds("kilocode")
}
pub fn has_cline_creds() -> bool {
    has_stored_creds("cline")
}
pub fn has_clinepass_creds() -> bool {
    has_stored_creds("clinepass")
}
pub fn has_kimchi_creds() -> bool {
    has_stored_creds("kimchi")
}
pub fn has_codebuddy_creds() -> bool {
    has_stored_creds("codebuddy-cn")
}
pub fn has_iflow_creds() -> bool {
    has_stored_creds("iflow")
}

/// Delete the OAuth credential files that our `/login` flow created for a
/// provider, so `/logout` fully clears the credentials (not just the provider
/// config + runtime key). Without this, `has_*_creds()` still returns true
/// after logout and the provider re-appears as "logged in" on the next
/// session — the stale token would even be used for model discovery + turns.
///
/// Only deletes credentials OUR login created. System-managed credentials
/// (gcloud ADC, `GOOGLE_APPLICATION_CREDENTIALS` service-account files) are
/// left alone — the user set those up independently and may still need them.
pub fn clear_oauth_creds(preset_id: &str) {
    let try_remove = |p: Option<std::path::PathBuf>| {
        if let Some(p) = p {
            let _ = std::fs::remove_file(&p);
        }
    };
    match preset_id {
        "gemini" => {
            // Our login writes ~/.gemini/oauth_creds.json (google-auth-library shape;
            // same path Antigravity / gemini-cli use).
            try_remove(gemini_creds_path());
            try_remove(legacy_gemini_token_path());
            // Clear the in-memory service-account cache so a stale SA token
            // isn't used after the personal token is gone.
            if let Ok(mut c) = sa_cache().lock() {
                *c = None;
            }
            // Do NOT delete gcloud ADC or GOOGLE_APPLICATION_CREDENTIALS —
            // those are system-managed credentials the user set up outside
            // this app.
        }
        "anthropic" => {
            // Our store only. Leave ~/.claude/.credentials.json (the Claude
            // CLI's own login) — if the user has the CLI set up, they're still
            // authenticated via it, which is correct.
            try_remove(stored_token_path("anthropic"));
        }
        "openai" => {
            try_remove(codex_auth_path());
        }
        "xai" => {
            try_remove(stored_token_path("xai"));
        }
        "qwen" => {
            try_remove(stored_token_path("qwen"));
        }
        "github" | "kimi-coding" | "kilocode" | "cline" | "clinepass" | "kimchi"
        | "codebuddy-cn" | "iflow" => {
            try_remove(stored_token_path(preset_id));
        }
        _ => {}
    }
}

// --- Google token storage (gemini-cli shape) ----------------------------------

fn read_gemini_token() -> Option<OAuthToken> {
    // Prefer the canonical gemini-cli path; fall back to our legacy file.
    let (path, legacy) = match gemini_creds_path() {
        Some(p) if p.exists() => (Some(p), false),
        _ => match legacy_gemini_token_path() {
            Some(p) if p.exists() => (Some(p), true),
            _ => (None, false),
        },
    };
    let path = path?;
    let data = std::fs::read_to_string(&path).ok()?;
    if legacy {
        // Legacy file is our own OAuthToken JSON.
        let tok: OAuthToken = serde_json::from_str(&data).ok()?;
        return if tok.access_token.is_empty() {
            None
        } else {
            Some(tok)
        };
    }
    let creds: GeminiCreds = serde_json::from_str(&data).ok()?;
    let access = creds.access_token?;
    Some(OAuthToken {
        access_token: access,
        refresh_token: creds.refresh_token,
        // gemini-cli stores MILLISECONDS; we work in seconds.
        expires_at: creds.expiry_date.map(|ms| ms / 1000).unwrap_or(0),
        client_id: Some(GOOGLE_CLIENT_ID.to_string()),
        client_secret: Some(GOOGLE_CLIENT_SECRET.to_string()),
        kind: "google".to_string(),
        id_token: creds.id_token,
        extra: None,
    })
}

fn write_gemini_token(tok: &OAuthToken) -> Option<()> {
    let path = gemini_creds_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    // Preserve the existing refresh_token + id_token when the new token doesn't
    // carry them. Google only returns a refresh_token on the FIRST authorization
    // for a given client+user; re-logins (same user, same client) return NO
    // refresh_token. Without this merge, a re-login would clobber the stored
    // refresh_token with None, making future refreshes impossible — the access
    // token expires in ~1h and the user is silently logged out. This mirrors
    // google-auth-library's `OAuth2Client.setCredentials` merge (Object.assign
    // over the existing credentials) that gemini-cli relies on.
    let existing = gemini_creds_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str::<GeminiCreds>(&s).ok());
    let creds = merged_gemini_creds(tok, existing.as_ref());
    let data = serde_json::to_string_pretty(&creds).ok()?;
    // Unique-temp atomic write + 0600 (fsutil): two processes refreshing the
    // same token concurrently never collide on a shared temp file.
    crate::fsutil::atomic_write_secure(&path, data.as_bytes()).ok()?;
    Some(())
}

/// Build the on-disk `GeminiCreds` from a freshly obtained `OAuthToken`,
/// merging with any existing credentials so fields Google omits on re-login
/// (refresh_token, id_token) are preserved rather than clobbered with None.
/// Google only returns a refresh_token on the FIRST authorization for a given
/// client+user; subsequent logins return NO refresh_token, so without this
/// merge a re-login would destroy the stored refresh_token and the access
/// token would become unrefreshable after its ~1h expiry. This mirrors
/// google-auth-library's `OAuth2Client.setCredentials` merge (Object.assign
/// over the existing credentials) that gemini-cli relies on.
fn merged_gemini_creds(tok: &OAuthToken, existing: Option<&GeminiCreds>) -> GeminiCreds {
    let prev_refresh = existing.and_then(|c| c.refresh_token.clone());
    let prev_id = existing.and_then(|c| c.id_token.clone());
    GeminiCreds {
        access_token: Some(tok.access_token.clone()),
        refresh_token: tok.refresh_token.clone().or(prev_refresh),
        token_type: Some("Bearer".to_string()),
        // seconds → milliseconds (match google-auth-library).
        expiry_date: Some(tok.expires_at.saturating_mul(1000)),
        scope: Some(google_scope_string()),
        id_token: tok.id_token.clone().or(prev_id),
    }
}

// --- Claude token storage (our own shape) -------------------------------------

fn read_stored_token(provider: &str) -> Option<OAuthToken> {
    let path = stored_token_path(provider)?;
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

fn store_token(provider: &str, tok: &OAuthToken) -> Option<()> {
    let dir = stored_token_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = stored_token_path(provider)?;
    let data = serde_json::to_string_pretty(tok).ok()?;
    crate::fsutil::atomic_write_secure(&path, data.as_bytes()).ok()?;
    Some(())
}

fn jwt_exp(jwt: &str) -> Option<u64> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("exp").and_then(|x| x.as_u64())
}

fn read_codex_token() -> Option<OAuthToken> {
    let data = std::fs::read_to_string(codex_auth_path()?).ok()?;
    let v: Value = serde_json::from_str(&data).ok()?;
    let tokens = v.get("tokens")?;
    let access = tokens
        .get("access_token")
        .and_then(|t| t.as_str())?
        .to_string();
    let refresh = tokens
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    // The Codex backend authenticates with the access_token JWT, and the official
    // codex CLI refreshes based on the ACCESS_TOKEN's `exp` claim
    // (manager.rs::should_refresh_proactive). The id_token's exp can differ, so
    // using it would either skip a needed refresh (401) or refresh needlessly.
    let expires_at = tokens
        .get("access_token")
        .and_then(|t| t.as_str())
        .and_then(jwt_exp)
        .or_else(|| {
            tokens
                .get("id_token")
                .and_then(|t| t.as_str())
                .and_then(jwt_exp)
        })
        .unwrap_or(0);
    Some(OAuthToken {
        access_token: access,
        refresh_token: refresh,
        expires_at,
        client_id: Some(OPENAI_CLIENT_ID.to_string()),
        client_secret: None,
        kind: "openai".to_string(),
        id_token: None,
        extra: None,
    })
}

fn write_codex_auth(tok: &OAuthToken, id_token: Option<&str>) -> Option<()> {
    let path = codex_auth_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let old: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let old_id = old
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(|t| t.as_str());
    let account_id = id_token
        .or(old_id)
        .and_then(jwt_account_id)
        .map(Value::String)
        .or_else(|| old.get("tokens").and_then(|t| t.get("account_id")).cloned())
        .unwrap_or(Value::Null);
    let data = json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": id_token.or(old_id).unwrap_or(""),
            "access_token": tok.access_token,
            "refresh_token": tok.refresh_token.clone().unwrap_or_default(),
            "account_id": account_id,
        },
    });
    let serialized = serde_json::to_string_pretty(&data).ok()?;
    crate::fsutil::atomic_write_secure(&path, serialized.as_bytes()).ok()?;
    Some(())
}

fn jwt_account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

// --- token refresh ------------------------------------------------------------

/// Refresh a Google OAuth token via gemini-cli's exact refresh request:
/// `refresh_token` + `client_id` + `client_secret` + `grant_type=refresh_token`
/// (client_secret in the BODY, not Basic; NO `x-goog-user-project` header).
/// Google does not always return a new refresh_token, so the old one is
/// preserved. Result is written back to `~/.gemini/oauth_creds.json`.
async fn refresh_google_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    let refresh_token = tok.refresh_token.clone()?;
    let client_id = tok
        .client_id
        .clone()
        .unwrap_or_else(|| GOOGLE_CLIENT_ID.to_string());
    let client_secret = tok
        .client_secret
        .clone()
        .unwrap_or_else(|| GOOGLE_CLIENT_SECRET.to_string());
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let access = v.get("access_token").and_then(|t| t.as_str())?.to_string();
    let expires_in = v.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    updated.expires_at = now_secs() + expires_in;
    // Preserve the refresh_token unless Google returned a new one.
    if let Some(rt) = v.get("refresh_token").and_then(|t| t.as_str()) {
        updated.refresh_token = Some(rt.to_string());
    }
    // Update the id_token when Google returns a fresh one (it does on refresh
    // when the openid scope was requested, as gemini-cli's scopes imply).
    if let Some(id) = v.get("id_token").and_then(|t| t.as_str()) {
        updated.id_token = Some(id.to_string());
    }
    write_gemini_token(&updated);
    Some(access)
}

/// Refresh a Claude token via Claude Code's exact refresh request: JSON body,
/// `grant_type=refresh_token`, `refresh_token`, `client_id`, and the same
/// subscription scopes. None on any failure.
async fn refresh_claude_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    let refresh_token = tok.refresh_token.clone()?;
    let mut client_id = tok
        .client_id
        .clone()
        .unwrap_or_else(|| CLAUDE_CLIENT_ID.to_string());
    // Migrate pre-production harness tokens that used the old dynamic-client
    // metadata URL. Fresh logins use Claude Code's first-party client UUID, and
    // rotated refreshes should keep using it.
    if client_id == CLAUDE_LEGACY_CLIENT_ID {
        client_id = CLAUDE_CLIENT_ID.to_string();
    }
    // Exact refresh body from the Claude Code binary (offset ~63928087):
    // grant_type=refresh_token, refresh_token, client_id. NO scope field.
    let req = json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": client_id,
    });
    let resp = client
        .post(CLAUDE_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let access = v.get("access_token").and_then(|t| t.as_str())?.to_string();
    let expires_in = v.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    updated.expires_at = now_secs() + expires_in;
    updated.client_id = Some(client_id);
    // Preserve the old refresh_token unless Anthropic returned a rotated one.
    if let Some(rt) = v.get("refresh_token").and_then(|t| t.as_str()) {
        updated.refresh_token = Some(rt.to_string());
    }
    let _ = store_token("anthropic", &updated);
    Some(access)
}

async fn refresh_codex_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    let refresh_token = tok.refresh_token.clone()?;
    let req = json!({
        "client_id": OPENAI_CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    });
    let resp = client
        .post(OPENAI_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let access = v.get("access_token").and_then(|t| t.as_str())?.to_string();
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    if let Some(rt) = v.get("refresh_token").and_then(|t| t.as_str()) {
        updated.refresh_token = Some(rt.to_string());
    }
    if let Some(id) = v.get("id_token").and_then(|t| t.as_str()) {
        updated.expires_at = jwt_exp(id).unwrap_or(0);
    }
    let new_id = v.get("id_token").and_then(|t| t.as_str());
    write_codex_auth(&updated, new_id);
    Some(access)
}

/// Refresh an xAI SuperGrok OAuth access token. Form-urlencoded body matching
/// Hermes / the xAI token endpoint (`grant_type=refresh_token`, `client_id`,
/// `refresh_token`). None on any failure (caller surfaces re-auth).
async fn refresh_xai_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    let refresh_token = tok.refresh_token.as_ref()?.clone();
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", XAI_CLIENT_ID),
        ("refresh_token", refresh_token.as_str()),
    ];
    let resp = client
        .post(XAI_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let access = v.get("access_token").and_then(|t| t.as_str())?.to_string();
    let expires_in = v
        .get("expires_in")
        .and_then(|t| t.as_u64())
        .unwrap_or(21600);
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    updated.expires_at = now_secs().saturating_add(expires_in);
    updated.client_id = Some(XAI_CLIENT_ID.to_string());
    if let Some(rt) = v.get("refresh_token").and_then(|t| t.as_str()) {
        updated.refresh_token = Some(rt.to_string());
    }
    if let Some(id) = v.get("id_token").and_then(|t| t.as_str()) {
        updated.id_token = Some(id.to_string());
    }
    let _ = store_token("xai", &updated);
    Some(access)
}

/// Refresh a Qwen Code OAuth access token (form-urlencoded refresh_token grant
/// matching 9router's qwen device-code flow).
async fn refresh_qwen_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    let refresh_token = tok.refresh_token.as_ref()?.clone();
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", QWEN_CLIENT_ID),
        ("refresh_token", refresh_token.as_str()),
    ];
    let resp = client
        .post(QWEN_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let access = v.get("access_token").and_then(|t| t.as_str())?.to_string();
    let expires_in = v.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    updated.expires_at = now_secs().saturating_add(expires_in);
    updated.client_id = Some(QWEN_CLIENT_ID.to_string());
    if let Some(rt) = v.get("refresh_token").and_then(|t| t.as_str()) {
        updated.refresh_token = Some(rt.to_string());
    }
    let _ = store_token("qwen", &updated);
    Some(access)
}

// --- token resolution (used at turn/discovery time by enrich_oauth) -----------

/// Resolve a Google OAuth access token. Resolution order matches "support
/// Google Cloud AND regular accounts":
///   1. `~/.gemini/oauth_creds.json` (our Antigravity `/login`) —
///      refreshed if expired. Works for personal + Workspace + Cloud identities.
///   2. `GOOGLE_APPLICATION_CREDENTIALS` (service-account JSON or an
///      `authorized_user` ADC file) — the canonical Google Cloud path.
///   3. gcloud ADC (`~/.config/gcloud/application_default_credentials.json`,
///      `authorized_user` type) — `gcloud auth application-default login`.
pub async fn google_token(client: &reqwest::Client) -> Option<String> {
    if let Some(tok) = read_gemini_token() {
        if !tok.access_token.is_empty() {
            let expired = tok.expires_at != 0 && tok.expires_at <= now_secs() + 60;
            if !expired {
                return Some(tok.access_token);
            }
            if let Some(refreshed) = refresh_google_token(client, &tok).await {
                return Some(refreshed);
            }
        }
    }
    if let Some(t) = google_token_from_gac(client).await {
        return Some(t);
    }
    google_token_from_adc(client).await
}

/// Cache for the Code Assist project ID (obtained via loadCodeAssist onboarding).
/// The project ID is stable per-user, so we fetch it once and reuse it.
static CODE_ASSIST_PROJECT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Antigravity Code Assist metadata (ideType/platform/pluginType). The daily
/// sandbox + Claude-via-Antigravity models require `ideType: ANTIGRAVITY`
/// (gemini-cli's `IDE_UNSPECIFIED` is rejected / limited to older models).
fn antigravity_metadata() -> Value {
    let platform = if cfg!(target_os = "windows") {
        "WINDOWS"
    } else if cfg!(target_os = "macos") {
        "MACOS"
    } else {
        // Antigravity only ships Windows/macOS clients; Linux hosts still
        // identify as MACOS (matches CLIProxy / opencode-antigravity-auth).
        "MACOS"
    };
    json!({
        "ideType": "ANTIGRAVITY",
        "platform": platform,
        "pluginType": "GEMINI",
    })
}

/// Headers Antigravity / CLIProxy send on every Code Assist request. Without
/// `Client-Metadata: ideType=ANTIGRAVITY` the gateway may refuse Gemini 3 /
/// Claude-via-Antigravity models even with a valid OAuth token.
pub fn antigravity_headers() -> Vec<(String, String)> {
    let platform = if cfg!(target_os = "windows") {
        "WINDOWS"
    } else {
        "MACOS"
    };
    let ua = format!(
        "antigravity/{ANTIGRAVITY_VERSION} {}",
        if cfg!(target_os = "windows") {
            "windows/amd64"
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "darwin/arm64"
            } else {
                "darwin/amd64"
            }
        } else {
            "darwin/amd64"
        }
    );
    let meta =
        format!(r#"{{"ideType":"ANTIGRAVITY","platform":"{platform}","pluginType":"GEMINI"}}"#);
    vec![
        ("User-Agent".to_string(), ua),
        (
            "X-Goog-Api-Client".to_string(),
            "google-cloud-sdk vscode_cloudshelleditor/0.1".to_string(),
        ),
        ("Client-Metadata".to_string(), meta),
    ]
}

/// Endpoint order for loadCodeAssist / project discovery. Prod first (best
/// support for managed project resolution), then daily, then autopush —
/// mirrors Antigravity / CLIProxy `ANTIGRAVITY_LOAD_ENDPOINTS`.
fn code_assist_load_endpoints() -> &'static [&'static str] {
    &[
        CODE_ASSIST_PROD_BASE_URL,
        CODE_ASSIST_BASE_URL,
        CODE_ASSIST_AUTOPUSH_BASE_URL,
    ]
}

/// Onboard the user on the Code Assist / Antigravity platform and return their
/// project ID. Every `generateContent` request requires a `project` field —
/// this fetches it via `loadCodeAssist` (and falls back to `onboardUser` for
/// new users, then to the Antigravity default project id). Cached for the
/// process lifetime (the project ID is stable).
pub async fn code_assist_project(client: &reqwest::Client) -> Option<String> {
    if let Some(cached) = CODE_ASSIST_PROJECT.get() {
        return cached.clone();
    }
    let token = google_token(client).await?;
    let result = fetch_code_assist_project(client, &token).await;
    // Only cache a successful onboarding. A transient failure on the FIRST call
    // (network blip / 10s timeout) returning None would otherwise be cached for
    // the process lifetime, permanently breaking Gemini-OAuth until restart.
    if result.is_some() {
        let _ = CODE_ASSIST_PROJECT.set(result.clone());
    }
    result
}

async fn fetch_code_assist_project(client: &reqwest::Client, token: &str) -> Option<String> {
    let body = json!({ "metadata": antigravity_metadata() });
    for base in code_assist_load_endpoints() {
        let mut req = client
            .post(format!("{base}:loadCodeAssist"))
            .bearer_auth(token)
            .json(&body)
            .timeout(std::time::Duration::from_secs(10));
        for (k, v) in antigravity_headers() {
            req = req.header(k, v);
        }
        let Ok(resp) = req.send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(v) = resp.json::<Value>().await else {
            continue;
        };
        // cloudaicompanionProject may be a bare string OR `{id: "..."}`.
        if let Some(p) = v.get("cloudaicompanionProject").and_then(|p| p.as_str()) {
            if !p.is_empty() {
                return Some(p.to_string());
            }
        }
        if let Some(p) = v
            .get("cloudaicompanionProject")
            .and_then(|p| p.get("id"))
            .and_then(|p| p.as_str())
        {
            if !p.is_empty() {
                return Some(p.to_string());
            }
        }
        // New user not yet provisioned — try onboard on this same endpoint.
        if let Some(p) = onboard_code_assist_user(client, token, base).await {
            return Some(p);
        }
    }
    // Last resort: Antigravity's hardcoded default project (business accounts).
    Some(ANTIGRAVITY_DEFAULT_PROJECT_ID.to_string())
}

async fn onboard_code_assist_user(
    client: &reqwest::Client,
    token: &str,
    base: &str,
) -> Option<String> {
    let body = json!({ "metadata": antigravity_metadata() });
    let mut req = client
        .post(format!("{base}:onboardUser"))
        .bearer_auth(token)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10));
    for (k, v) in antigravity_headers() {
        req = req.header(k, v);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    // onboardUser responses vary: top-level projectId, or nested
    // response.cloudaicompanionProject.id.
    if let Some(p) = v.get("projectId").and_then(|p| p.as_str()) {
        if !p.is_empty() {
            return Some(p.to_string());
        }
    }
    if let Some(p) = v
        .get("response")
        .and_then(|r| r.get("cloudaicompanionProject"))
        .and_then(|p| p.get("id"))
        .and_then(|p| p.as_str())
    {
        if !p.is_empty() {
            return Some(p.to_string());
        }
    }
    None
}

#[derive(Deserialize)]
struct GcloudAdc {
    #[serde(rename = "type", default)]
    typ: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

async fn google_token_from_adc(client: &reqwest::Client) -> Option<String> {
    let path = gcloud_adc_path()?;
    let data = std::fs::read_to_string(&path).ok()?;
    let adc: GcloudAdc = serde_json::from_str(&data).ok()?;
    if let Some(t) = adc.access_token.filter(|s| !s.is_empty()) {
        return Some(t);
    }
    if adc.typ.as_deref() != Some("authorized_user") {
        return None;
    }
    let client_id = adc.client_id?;
    let client_secret = adc.client_secret?;
    let refresh_token = adc.refresh_token?;
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    v.get("access_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

/// Read `GOOGLE_APPLICATION_CREDENTIALS` (a Google Cloud key file). Dispatches
/// by `type`: `service_account` → JWT-bearer exchange (RS256-signed assertion);
/// `authorized_user` → refresh_token grant. This is the standard Google Cloud
/// auth path — it lets a service account (headless Cloud workload) authenticate
/// with no interactive `/login`.
async fn google_token_from_gac(client: &reqwest::Client) -> Option<String> {
    let p = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .ok()
        .filter(|s| !s.is_empty())?;
    let data = std::fs::read_to_string(&p).ok()?;
    let v: Value = serde_json::from_str(&data).ok()?;
    match v.get("type").and_then(|t| t.as_str()) {
        Some("service_account") => google_token_from_service_account(client, &v).await,
        Some("authorized_user") => google_token_from_authorized_user(client, &v).await,
        _ => None,
    }
}

async fn google_token_from_authorized_user(client: &reqwest::Client, v: &Value) -> Option<String> {
    let client_id = v.get("client_id").and_then(|t| t.as_str())?.to_string();
    let client_secret = v.get("client_secret").and_then(|t| t.as_str())?.to_string();
    let refresh_token = v.get("refresh_token").and_then(|t| t.as_str())?.to_string();
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let r: Value = resp.json().await.ok()?;
    r.get("access_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

// In-memory cache for service-account access tokens (short-lived; avoids
// re-signing + re-exchanging on every turn). (token, expiry_secs).
static SA_CACHE: OnceLock<std::sync::Mutex<Option<(String, u64)>>> = OnceLock::new();
fn sa_cache() -> &'static std::sync::Mutex<Option<(String, u64)>> {
    SA_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Exchange a service-account key for an OAuth access token via the JWT-bearer
/// grant (RFC 7523): sign a RS256 JWT `{iss, scope, aud, iat, exp}` with the
/// service account's private key, POST it as `assertion` with
/// `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer`. This is exactly
/// what `google-auth-library`'s `GoogleAuth.fromJSON(service_account)` does.
async fn google_token_from_service_account(client: &reqwest::Client, v: &Value) -> Option<String> {
    // Return a cached token if still valid (60s skew).
    {
        let cache = sa_cache().lock().ok()?;
        if let Some((tok, exp)) = cache.as_ref() {
            if *exp > now_secs() + 60 {
                return Some(tok.clone());
            }
        }
    }

    let iss = v.get("client_email").and_then(|t| t.as_str())?;
    let private_key_pem = v.get("private_key").and_then(|t| t.as_str())?;
    let token_uri = v
        .get("token_uri")
        .and_then(|t| t.as_str())
        .unwrap_or(GOOGLE_TOKEN_URL);

    let now = now_secs();
    let claims = json!({
        "iss": iss,
        "scope": google_scope_string(),
        "aud": token_uri,
        "iat": now,
        "exp": now + 3600,
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.typ = Some("JWT".to_string());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key_pem.as_bytes()).ok()?;
    let assertion = jsonwebtoken::encode(&header, &claims, &key).ok()?;

    let resp = client
        .post(token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let r: Value = resp.json().await.ok()?;
    let access = r.get("access_token").and_then(|t| t.as_str())?.to_string();
    let expires_in = r.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
    let exp = now_secs() + expires_in;
    if let Ok(mut cache) = sa_cache().lock() {
        *cache = Some((access.clone(), exp));
    }
    Some(access)
}

/// Resolve an Anthropic (Claude.ai) OAuth access token. Prefers our stored
/// token (refreshing), then the CLI's `~/.claude/.credentials.json`.
pub async fn claude_token(client: &reqwest::Client) -> Option<String> {
    if let Some(tok) = read_stored_token("anthropic") {
        if !tok.access_token.is_empty() {
            let expired = tok.expires_at != 0 && tok.expires_at <= now_secs() + 60;
            if !expired {
                return Some(tok.access_token);
            }
            if let Some(refreshed) = refresh_claude_token(client, &tok).await {
                return Some(refreshed);
            }
        }
    }
    claude_token_from_cli(client).await
}

#[derive(Deserialize)]
struct ClaudeCreds {
    #[serde(rename = "claudeAiOauth", default)]
    oauth: Option<ClaudeOauth>,
}
#[derive(Deserialize)]
struct ClaudeOauth {
    #[serde(default, rename = "accessToken")]
    access_token: Option<String>,
    #[serde(default, rename = "refreshToken")]
    #[allow(dead_code)]
    refresh_token: Option<String>,
    #[serde(default, rename = "expiresAt")]
    expires_at: Option<u64>,
}

async fn claude_token_from_cli(client: &reqwest::Client) -> Option<String> {
    let path = claude_creds_path()?;
    let data = std::fs::read_to_string(&path).ok()?;
    let creds: ClaudeCreds = serde_json::from_str(&data).ok()?;
    let oauth = creds.oauth?;
    let now = now_secs();
    let expired = oauth
        .expires_at
        .map(|exp| {
            (if exp > 1_000_000_000_000 {
                exp / 1000
            } else {
                exp
            }) <= now + 60
        })
        .unwrap_or(false);
    if !expired {
        if let Some(t) = oauth.access_token.filter(|s| !s.is_empty()) {
            return Some(t);
        }
    }
    let _ = client; // CLI-token refresh not attempted (client_id not stored by the CLI).
    None
}

pub fn codex_account_id() -> Option<String> {
    let data = std::fs::read_to_string(codex_auth_path()?).ok()?;
    let v: Value = serde_json::from_str(&data).ok()?;
    v.get("tokens")
        .and_then(|t| t.get("account_id"))
        .and_then(|a| a.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            v.get("tokens")
                .and_then(|t| t.get("id_token"))
                .and_then(|t| t.as_str())
                .and_then(jwt_account_id)
        })
}

pub async fn codex_token(client: &reqwest::Client) -> Option<String> {
    if let Some(tok) = read_codex_token() {
        if !tok.access_token.is_empty() {
            let expired = tok.expires_at != 0 && tok.expires_at <= now_secs() + 60;
            if !expired {
                return Some(tok.access_token);
            }
            if let Some(refreshed) = refresh_codex_token(client, &tok).await {
                return Some(refreshed);
            }
        }
    }
    None
}

/// Resolve an xAI SuperGrok OAuth access token from
/// `~/.config/catalyst-code/oauth/xai.json`, refreshing proactively (up to
/// [`XAI_REFRESH_SKEW_SECS`] early) when a refresh_token is present.
pub async fn xai_token(client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("xai")?;
    if tok.access_token.is_empty() {
        return None;
    }
    let near_expiry =
        tok.expires_at != 0 && tok.expires_at <= now_secs().saturating_add(XAI_REFRESH_SKEW_SECS);
    if !near_expiry {
        return Some(tok.access_token);
    }
    if let Some(refreshed) = refresh_xai_token(client, &tok).await {
        return Some(refreshed);
    }
    // Refresh failed — still try the existing access token if it hasn't fully
    // expired (skew window only). Otherwise force re-login.
    if tok.expires_at == 0 || tok.expires_at > now_secs() + 30 {
        return Some(tok.access_token);
    }
    None
}

/// Resolve a Qwen Code OAuth access token from
/// `~/.config/catalyst-code/oauth/qwen.json`, refreshing proactively.
pub async fn qwen_token(client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("qwen")?;
    if tok.access_token.is_empty() {
        return None;
    }
    let near_expiry =
        tok.expires_at != 0 && tok.expires_at <= now_secs().saturating_add(QWEN_REFRESH_SKEW_SECS);
    if !near_expiry {
        return Some(tok.access_token);
    }
    if let Some(refreshed) = refresh_qwen_token(client, &tok).await {
        return Some(refreshed);
    }
    if tok.expires_at == 0 || tok.expires_at > now_secs() + 30 {
        return Some(tok.access_token);
    }
    None
}

/// Fill in a subscription OAuth token for `rp` when it has no API key. An
/// explicit API key always wins (manual override / env). Returns `rp` unchanged
/// (still keyless) when no token is available — callers fall back to the API-key
/// path. Flips `rp.oauth = true` for Anthropic (Bearer + oauth-beta header).
pub async fn enrich_oauth(
    mut rp: ResolvedProvider,
    client: &reqwest::Client,
    pm: Option<&crate::plugins::PluginManager>,
) -> ResolvedProvider {
    // The Codex (ChatGPT subscription) backend at chatgpt.com/backend-api/codex
    // requires the same default headers the official `codex` CLI sets on EVERY
    // request (`codex-rs/login/src/auth/default_client.rs::default_headers`):
    // `originator: codex_cli_rs` and a codex-shaped `User-Agent`. Without them
    // the gateway rejects model/discovery requests (looks like a foreign
    // client), so a successful `/login` is followed by 401/403 on every turn.
    // Inject them regardless of auth mode so OAuth-subscription AND API-key
    // turns both reach the backend. (The token-exchange to auth.openai.com uses
    // a separate raw client and does NOT send these — matching the CLI.)
    if crate::provider::is_codex_endpoint(&rp.base_url) {
        inject_codex_headers(&mut rp.headers);
    }
    if rp.api_key.is_some() {
        return rp;
    }
    match rp.kind {
        ProviderKind::Anthropic if crate::provider::is_anthropic_endpoint(&rp.base_url) => {
            if let Some(t) = claude_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_codex_endpoint(&rp.base_url) => {
            if let Some(t) = codex_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
                if let Some(account_id) = codex_account_id() {
                    rp.headers
                        .push(("chatgpt-account-id".to_string(), account_id));
                }
            }
        }
        _ if crate::provider::is_gemini_endpoint(&rp.base_url) => {
            if let Some(t) = google_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
                // CRITICAL: the Antigravity OAuth token is NOT for
                // generativelanguage.googleapis.com — that endpoint only accepts
                // API keys (returns 401 for OAuth Bearer tokens). Route OAuth
                // turns through the Antigravity Code Assist gateway (daily
                // sandbox first) which proxies Gemini 3 + Claude-via-Antigravity.
                rp.base_url = CODE_ASSIST_BASE_URL.to_string();
                inject_antigravity_headers(&mut rp.headers);
            }
        }
        _ if crate::provider::is_xai_endpoint(&rp.base_url) => {
            // SuperGrok / X Premium+ subscription token when no XAI_API_KEY is set.
            if let Some(t) = xai_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_qwen_endpoint(&rp.base_url) => {
            if let Some(t) = qwen_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_github_copilot_endpoint(&rp.base_url) => {
            if let Some(t) = github_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_kimi_coding_endpoint(&rp.base_url) => {
            if let Some(t) = kimi_coding_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_kilocode_endpoint(&rp.base_url) => {
            if let Some(t) = kilocode_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
                if let Some(org) = read_stored_token("kilocode")
                    .and_then(|t| t.extra)
                    .and_then(|v| {
                        v.get("organization_id")
                            .and_then(Value::as_str)
                            .map(String::from)
                    })
                {
                    if !rp
                        .headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("x-kilocode-organizationid"))
                    {
                        rp.headers
                            .push(("x-kilocode-organizationid".to_string(), org));
                    }
                }
            }
        }
        _ if crate::provider::is_cline_endpoint(&rp.base_url) => {
            let store = if rp.name == "clinepass" {
                "clinepass"
            } else {
                "cline"
            };
            if let Some(t) = cline_family_token(client, store).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_kimchi_endpoint(&rp.base_url) => {
            if let Some(t) = kimchi_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_codebuddy_endpoint(&rp.base_url) => {
            if let Some(t) = codebuddy_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ if crate::provider::is_iflow_endpoint(&rp.base_url) => {
            if let Some(t) = iflow_token(client).await {
                rp.api_key = Some(t);
                rp.oauth = true;
            }
        }
        _ => {}
    }
    // Plugin-declared OAuth providers: when no built-in flow matched and there
    // is still no API key, consult plugins. A plugin owns its token's on-disk
    // format; we just resolve (cached) and inject as `Authorization: Bearer`.
    // This is the no-recompile path to add a subscription-OAuth provider
    // (Grok device-code, a corporate SSO, …) the way OpenAI/Claude/Gemini work.
    if rp.api_key.is_none() {
        if let Some(pm) = pm {
            if let Some(cfg) = pm.oauth_config_for_provider(&rp) {
                if let Some(token) = pm.resolve_oauth_token(&cfg.provider_id).await {
                    rp.api_key = Some(token);
                    rp.oauth = true;
                }
            }
        }
    }
    rp
}

/// The default headers the official `codex` CLI attaches to every request
/// Attach Antigravity Client-Metadata / User-Agent so the Code Assist gateway
/// routes the request to Gemini 3 / Claude-via-Antigravity (without these the
/// gateway may treat the call as gemini-cli and refuse newer models).
fn inject_antigravity_headers(headers: &mut Vec<(String, String)>) {
    for (k, v) in antigravity_headers() {
        if !headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(&k)) {
            headers.push((k, v));
        }
    }
}

/// against the ChatGPT Codex backend. `originator` identifies the client to
/// OpenAI's gateway (routing/entitlement); `User-Agent` avoids looking like a
/// bare bot to Cloudflare. Idempotent — skips a header already present so
/// repeated `enrich_oauth` calls on the same provider don't duplicate.
fn inject_codex_headers(headers: &mut Vec<(String, String)>) {
    let mut has_originator = false;
    let mut has_ua = false;
    for (k, _) in headers.iter() {
        let kl = k.to_ascii_lowercase();
        if kl == "originator" {
            has_originator = true;
        }
        if kl == "user-agent" {
            has_ua = true;
        }
    }
    if !has_originator {
        headers.push(("originator".to_string(), OPENAI_ORIGINATOR.to_string()));
    }
    if !has_ua {
        headers.push(("User-Agent".to_string(), codex_user_agent()));
    }
}

/// Build a `codex_cli_rs/<version> (<os>)` User-Agent, mirroring the official
/// codex CLI's `get_codex_user_agent()` shape closely enough that the gateway
/// treats us as the same first-party client.
fn codex_user_agent() -> String {
    let os = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        "Unix"
    };
    format!(
        "codex_cli_rs/{} ({}; {})",
        env!("CARGO_PKG_VERSION"),
        os,
        std::env::consts::ARCH
    )
}

// --- interactive OAuth login flows (`/login`) --------------------------------

fn random_b64url(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(&bytes)
}

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut s = String::with_capacity(n * 2);
    for b in &bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn pkce_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

/// Percent-encode a string for an OAuth URL query value (RFC 3986 unreserved
/// set kept; everything else %-encoded). Matches what `google-auth-library`
/// effectively produces when building the authorize URL.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' || c == '~' {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Bind a loopback OAuth callback server for a plugin's web flow: IPv4 on
/// `127.0.0.1:<port>` (port 0 = OS-assigned ephemeral) plus IPv6 best-effort on
/// `::1:<same port>`, because browsers often resolve `localhost` to `::1`
/// before `127.0.0.1`. Returns the IPv4 listener, an optional IPv6 listener,
/// and the port actually bound (use it to build the redirect_uri). Mirrors
/// `bind_claude_callback` but parametric on the port so a plugin can request a
/// registered port when its provider requires one.
pub async fn bind_loopback(
    port: u16,
) -> Result<
    (
        tokio::net::TcpListener,
        Option<tokio::net::TcpListener>,
        u16,
    ),
    String,
> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("could not bind localhost OAuth callback: {e}"))?;
    let bound_port = listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())?;
    let listener_v6 = tokio::net::TcpListener::bind(("::1", bound_port))
        .await
        .ok();
    Ok((listener, listener_v6, bound_port))
}

pub async fn await_redirect_dual(
    listener: tokio::net::TcpListener,
    listener_v6: Option<tokio::net::TcpListener>,
    state: &str,
    success_url: Option<&str>,
) -> Result<String, String> {
    tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            let stream = if let Some(v6) = &listener_v6 {
                tokio::select! {
                    r = listener.accept() => r.map_err(|e| e.to_string())?.0,
                    r = v6.accept() => r.map_err(|e| e.to_string())?.0,
                }
            } else {
                listener.accept().await.map_err(|e| e.to_string())?.0
            };
            if let Some(code) = handle_redirect_stream(stream, state, success_url).await? {
                return Ok(code);
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for the browser redirect".to_string())?
}

async fn handle_redirect_stream(
    mut stream: tokio::net::TcpStream,
    state: &str,
    success_url: Option<&str>,
) -> Result<Option<String>, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let resp = if let Some(url) = success_url {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    } else {
        let body = "<html><body><h2>Login complete</h2><p>You can close this tab and return to the terminal.</p></body></html>";
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    };
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.flush().await;
    let Some(line) = req.lines().next() else {
        return Ok(None);
    };
    let Some(path) = line.split(' ').nth(1) else {
        return Ok(None);
    };
    let Some(qs) = path.split('?').nth(1) else {
        return Ok(None);
    };
    let mut code = None;
    let mut st = None;
    for pair in qs.split('&') {
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("");
        let v = it.next().unwrap_or("");
        if k == "code" {
            code = Some(percent_decode(v));
        } else if k == "state" {
            st = Some(percent_decode(v));
        }
    }
    // An empty `state` means the caller requested no CSRF protection (plugin
    // OAuth web flows that don't use state) — accept any redirect. A non-empty
    // state requires an exact match (the built-in flows always pass one).
    if state.is_empty() || st.as_deref() == Some(state) {
        return code
            .map(Some)
            .ok_or_else(|| "no code in redirect".to_string());
    }
    Ok(None)
}

/// Pending state for the no-browser (manual-code) OAuth flow — held in `State`
/// until the user pastes the authorization code back via the `oauth_code`
/// command, at which point `complete_oauth` finishes the exchange.
#[derive(Clone)]
pub struct PendingOauth {
    /// The preset id this login is for (e.g. "gemini") — which `complete_oauth`
    /// path to use and which provider to configure on success.
    pub kind: String,
    /// PKCE code_verifier (no-browser flow only).
    pub code_verifier: String,
    /// The CSRF state sent in the authorize URL (kept for parity/audit).
    #[allow(dead_code)]
    pub state: String,
    /// The redirect_uri used (e.g. `http://localhost:51121/oauth-callback`).
    pub redirect_uri: String,
    /// Opaque JSON blob a plugin's `login` action returned and `complete` needs
    /// (e.g. a PKCE verifier, a device-auth id). Only set for plugin OAuth
    /// providers; `None` for the built-in flows (which carry state inline).
    pub plugin_pending: Option<serde_json::Value>,
}

impl PendingOauth {
    /// Build a `PendingOauth` for a plugin OAuth provider (manual flow). The
    /// opaque `pending` blob returned by the plugin's `login` action is stashed
    /// here and passed back to its `complete` action verbatim.
    pub fn plugin(provider_id: &str, state: String, pending: Option<serde_json::Value>) -> Self {
        PendingOauth {
            kind: provider_id.to_string(),
            code_verifier: String::new(),
            state,
            redirect_uri: String::new(),
            plugin_pending: pending,
        }
    }
}

/// Outcome of starting an interactive OAuth login (`oauth::login`). The web
/// flow completes synchronously (`Done`); the no-browser flow returns
/// `AwaitingCode` and defers the token exchange to `complete_oauth`.
pub enum LoginOutcome {
    /// Login finished — token stored; provider is ready.
    Done,
    /// No-browser flow: the authorize URL was already emitted via `oauth_prompt`
    /// (`google_login_manual` emits it); the user pastes the code back via
    /// `oauth_code`. `pending` holds the verifier needed to finish the exchange.
    AwaitingCode { pending: PendingOauth },
}

/// Whether we appear to be in a headless / remote (SSH) session where launching
/// a local browser and capturing a loopback redirect will NOT work. When true,
/// `google_login` uses the no-browser (manual-code) flow. Override explicitly
/// with `CATALYST_CODE_NO_BROWSER=1` (force manual) or `=0` (force web, e.g.
/// when you've set up SSH port forwarding to the loopback port).
pub fn likely_headless() -> bool {
    fn env_truthy(name: &str) -> Option<bool> {
        std::env::var(name).ok().map(|v| {
            let v = v.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "no" || v == "off")
        })
    }
    match env_truthy("CATALYST_CODE_NO_BROWSER").or_else(|| env_truthy("NO_BROWSER")) {
        Some(true) => return true,
        Some(false) => return false,
        None => {}
    }
    // SSH session → the remote box can't open a browser the user can see, and
    // the user's local browser can't reach the remote loopback redirect.
    if std::env::var("SSH_CONNECTION").is_ok() || std::env::var("SSH_TTY").is_ok() {
        return true;
    }
    // No display server on Linux/BSD → no GUI browser. (macOS doesn't use
    // DISPLAY, so this only applies to non-macOS Unix.)
    if cfg!(unix) && !cfg!(target_os = "macos") {
        let display = std::env::var("DISPLAY").unwrap_or_default();
        let wayland = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
        if display.is_empty() && wayland.is_empty() {
            return true;
        }
    }
    false
}

/// Start the Google OAuth login. Picks the flow by environment:
///  - web flow (default, local machine): loopback redirect + PKCE — completes
///    synchronously when the browser hits `http://localhost:51121/oauth-callback`.
///  - no-browser flow (SSH / headless, or `CATALYST_CODE_NO_BROWSER=1`):
///    same registered redirect URI + PKCE; returns `AwaitingCode` and the user
///    pastes the code (or full redirect URL) via `oauth_code`. Antigravity does
///    not register gemini-cli's OOB page, so we cannot use that shortcut.
pub async fn google_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    if likely_headless() {
        return google_login_manual(emit);
    }
    google_login_web(client, emit)
        .await
        .map(|_| LoginOutcome::Done)
}

/// The no-browser Google / Antigravity OAuth flow. Uses the **same** registered
/// redirect URI as the web flow (`http://localhost:51121/oauth-callback`). After
/// the user consents, Google redirects the browser there; if nothing is
/// listening (typical over SSH without `-L 51121`), the page fails to load but
/// the address bar still contains `?code=…` — paste that code or the full URL
/// with `/oauth-code`. Emits the authorize URL; returns `AwaitingCode` with the
/// stashed PKCE verifier. `complete_oauth` finishes the exchange.
fn google_login_manual(emit: &(dyn Fn(OAuthPrompt) + Send + Sync)) -> Result<LoginOutcome, String> {
    let verifier = random_b64url(96); // 128 chars (matches gemini-cli's verifier length)
    let challenge = pkce_challenge(&verifier);
    let state = random_hex(32); // 64 hex chars
                                // MUST match the Antigravity client's registered redirect. The gemini-cli
                                // OOB URI (codeassist.google.com/authcode) is NOT registered for this client
                                // and produces Error 400: redirect_uri_mismatch.
    let redirect = ANTIGRAVITY_REDIRECT_URI.to_string();
    let redirect_enc = pct_encode(&redirect);
    let scope_enc = pct_encode(&google_scope_string());
    let authorize = format!(
        "{GOOGLE_AUTHORIZE_URL}?client_id={GOOGLE_CLIENT_ID}&redirect_uri={redirect_enc}&response_type=code&scope={scope_enc}&access_type=offline&prompt=consent&code_challenge={challenge}&code_challenge_method=S256&state={state}"
    );
    emit(OAuthPrompt {
        url: authorize.clone(),
        code: None,
        message: format!(
            "No browser detected (SSH/headless). Open this URL on ANY device, sign in and approve. \
Google will redirect to {ANTIGRAVITY_REDIRECT_URI} (the page may fail to load — that's OK). \
Copy the `code` query value from the address bar (or the full URL) and paste it here with: \
/oauth-code <code-or-url>. Tip: `ssh -L {ANTIGRAVITY_REDIRECT_PORT}:127.0.0.1:{ANTIGRAVITY_REDIRECT_PORT}` \
plus CATALYST_CODE_NO_BROWSER=0 enables automatic capture."
        ),
    });
    Ok(LoginOutcome::AwaitingCode {
        pending: PendingOauth {
            kind: "gemini".to_string(),
            code_verifier: verifier,
            state,
            redirect_uri: redirect,
            plugin_pending: None,
        },
    })
}

/// Complete a pending no-browser Google OAuth login by exchanging the code the
/// user pasted. Uses the stashed PKCE verifier + the Antigravity-registered
/// loopback redirect_uri. Stores the token at `~/.gemini/oauth_creds.json`.
pub async fn complete_google_login(
    client: &reqwest::Client,
    pending: &PendingOauth,
    code: &str,
) -> Result<OAuthToken, String> {
    let code = extract_auth_code(code);
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", pending.redirect_uri.as_str()),
        ("client_id", GOOGLE_CLIENT_ID),
        ("client_secret", GOOGLE_CLIENT_SECRET),
        ("code_verifier", pending.code_verifier.as_str()),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("token exchange failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token endpoint returned {status}: {body}"));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("token parse failed: {e}"))?;
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or("no access_token")?
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let exp = v.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
    let id_token = v
        .get("id_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let tok = OAuthToken {
        access_token: access,
        refresh_token: refresh,
        expires_at: now_secs() + exp,
        client_id: Some(GOOGLE_CLIENT_ID.to_string()),
        client_secret: Some(GOOGLE_CLIENT_SECRET.to_string()),
        kind: "google".to_string(),
        id_token,
        extra: None,
    };
    write_gemini_token(&tok);
    if let Ok(mut c) = sa_cache().lock() {
        *c = None;
    }
    Ok(tok)
}

/// Finish a pending OAuth login (dispatch by provider kind). Called by the
/// `oauth_code` command handler.
pub async fn complete_oauth(
    preset_id: &str,
    client: &reqwest::Client,
    pending: &PendingOauth,
    code: &str,
) -> Result<OAuthToken, String> {
    match preset_id {
        "openai" => complete_codex_login(client, pending, code).await,
        "gemini" => complete_google_login(client, pending, code).await,
        "anthropic" => complete_claude_login(client, pending, code).await,
        "cline" | "clinepass" => complete_cline_login(client, pending, code).await,
        "kimchi" => complete_kimchi_login(client, pending, code).await,
        "iflow" => complete_iflow_login(client, pending, code).await,
        other => Err(format!("no manual OAuth completion for '{other}'")),
    }
}

/// Accept either a bare authorization code OR a full redirect URL and extract
/// the `code` query param (URL-decoding it). Users sometimes paste the whole
/// `http://localhost:51121/oauth-callback?code=4%2F0Axx…&scope=…` (or an old
/// gemini-cli OOB URL) after the browser redirect.
fn extract_auth_code(input: &str) -> String {
    query_param(input, "code").unwrap_or_else(|| input.trim().to_string())
}

fn query_param(input: &str, key: &str) -> Option<String> {
    let s = input.trim();
    let query = s.split_once('?').map(|(_, q)| q).unwrap_or(s);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// The web flow: fixed loopback `http://localhost:51121/oauth-callback`
/// redirect with PKCE — the only URI registered for the Antigravity client.
/// A different port/path causes Google's `redirect_uri_mismatch`. Binds on
/// IPv4 + IPv6 because browsers may resolve `localhost` to `::1`. Completes
/// synchronously (blocks until the browser redirect arrives, up to 5 min).
async fn google_login_web(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<OAuthToken, String> {
    let (listener, listener_v6, _port) = bind_loopback(ANTIGRAVITY_REDIRECT_PORT).await?;
    let redirect = ANTIGRAVITY_REDIRECT_URI.to_string();
    let redirect_enc = pct_encode(&redirect);
    let scope_enc = pct_encode(&google_scope_string());
    let verifier = random_b64url(48);
    let challenge = pkce_challenge(&verifier);
    let state = random_b64url(32);

    let authorize = format!(
        "{GOOGLE_AUTHORIZE_URL}?client_id={GOOGLE_CLIENT_ID}&redirect_uri={redirect_enc}&response_type=code&scope={scope_enc}&access_type=offline&prompt=consent&state={state}&code_challenge={challenge}&code_challenge_method=S256"
    );

    emit(OAuthPrompt {
        url: authorize.clone(),
        code: None,
        message: "Open the URL in a browser to log in to Google (Antigravity / Gemini). After approving, this continues automatically.".to_string(),
    });
    let _ = open_browser(&authorize);

    let code = await_redirect_dual(listener, listener_v6, &state, Some(GOOGLE_SUCCESS_URL)).await?;

    // Token exchange — Antigravity requires PKCE (code_verifier) for the
    // installed-app client. client_secret stays in the body (Google installed-app
    // convention), not HTTP Basic.
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("client_id", GOOGLE_CLIENT_ID),
        ("client_secret", GOOGLE_CLIENT_SECRET),
        ("code_verifier", verifier.as_str()),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("token exchange failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token endpoint returned {status}: {body}"));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("token parse failed: {e}"))?;
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or("no access_token")?
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let exp = v.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
    let id_token = v
        .get("id_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let tok = OAuthToken {
        access_token: access,
        refresh_token: refresh,
        expires_at: now_secs() + exp,
        client_id: Some(GOOGLE_CLIENT_ID.to_string()),
        client_secret: Some(GOOGLE_CLIENT_SECRET.to_string()),
        kind: "google".to_string(),
        id_token,
        extra: None,
    };
    write_gemini_token(&tok);
    // Best-effort: clear any stale service-account cache so the new personal
    // token is used immediately.
    if let Ok(mut c) = sa_cache().lock() {
        *c = None;
    }
    Ok(tok)
}

/// Start Anthropic Claude subscription OAuth. Local machines use Claude Code's
/// loopback flow (`http://localhost:<port>/callback`). SSH/headless sessions use
/// Claude Code's manual flow (`https://platform.claude.com/oauth/code/callback`)
/// and complete via `/oauth-code <code-or-callback-url>`, so no port forwarding
/// is required.
pub async fn claude_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    if likely_headless() {
        return claude_login_manual(emit);
    }
    claude_login_web(client, emit)
        .await
        .map(|_| LoginOutcome::Done)
}

fn claude_authorize_url(challenge: &str, state: &str, redirect_uri: &str) -> String {
    // Exact query params from the Claude Code binary (offset ~63918986):
    // client_id, response_type, redirect_uri, scope, code_challenge,
    // code_challenge_method=S256, state.  (login_hint/login_method are optional
    // and omitted; isManual/port/code=true are internal JS options, NOT URL params.)
    let params = [
        ("client_id", CLAUDE_CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("scope", &claude_scope_string()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", pct_encode(k), pct_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{CLAUDE_AUTHORIZE_URL}?{query}")
}

fn claude_login_manual(emit: &(dyn Fn(OAuthPrompt) + Send + Sync)) -> Result<LoginOutcome, String> {
    let verifier = random_b64url(48);
    let challenge = pkce_challenge(&verifier);
    let state = random_b64url(32);
    let redirect = CLAUDE_MANUAL_REDIRECT_URL.to_string();
    let authorize = claude_authorize_url(&challenge, &state, &redirect);
    emit(OAuthPrompt {
        url: authorize,
        code: None,
        message: "No browser detected (SSH/headless). Open this Claude URL on ANY device, sign in and approve, then paste the authorization code or the final callback URL back here with: /oauth-code <code-or-url>".to_string(),
    });
    Ok(LoginOutcome::AwaitingCode {
        pending: PendingOauth {
            kind: "anthropic".to_string(),
            code_verifier: verifier,
            state,
            redirect_uri: redirect,
            plugin_pending: None,
        },
    })
}

async fn claude_login_web(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<OAuthToken, String> {
    let verifier = random_b64url(48);
    let challenge = pkce_challenge(&verifier);
    let state = random_b64url(32);
    let (listener, listener_v6, port) = bind_claude_callback().await?;
    let redirect = format!("http://localhost:{port}/callback");
    let authorize = claude_authorize_url(&challenge, &state, &redirect);

    emit(OAuthPrompt {
        url: authorize.clone(),
        code: None,
        message: "Open the URL in a browser to log in to Claude. After approving, this continues automatically.".to_string(),
    });
    let _ = open_browser(&authorize);

    let code = await_redirect_dual(listener, listener_v6, &state, None).await?;
    exchange_claude_code(client, &code, &redirect, &verifier).await
}

async fn bind_claude_callback() -> Result<
    (
        tokio::net::TcpListener,
        Option<tokio::net::TcpListener>,
        u16,
    ),
    String,
> {
    // Claude Code uses a loopback localhost redirect with an arbitrary available
    // port. Bind IPv4 first and IPv6 best-effort because browsers often resolve
    // localhost to ::1 before 127.0.0.1.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("could not bind localhost OAuth callback: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let listener_v6 = tokio::net::TcpListener::bind(("::1", port)).await.ok();
    Ok((listener, listener_v6, port))
}

pub async fn complete_claude_login(
    client: &reqwest::Client,
    pending: &PendingOauth,
    code: &str,
) -> Result<OAuthToken, String> {
    // The manual callback page may present the code as `authorizationCode#state`
    // (Claude Code's own paste format). We already stashed the state at login
    // start, so strip anything after `#`. Also handle full callback URLs.
    let raw = code.trim();
    let stripped = if let Some((auth, _)) = raw.split_once('#') {
        auth
    } else {
        raw
    };
    let extracted = extract_auth_code(stripped);
    if extracted.is_empty() {
        return Err("No authorization code found in the input. Paste the code from the Claude callback page (format: code#state) or the full callback URL.".to_string());
    }
    exchange_claude_code(
        client,
        &extracted,
        &pending.redirect_uri,
        &pending.code_verifier,
    )
    .await
    .map_err(|e| {
        format!(
            "{e}  (extracted code: {} chars, starts with {:?})",
            extracted.len(),
            extracted.chars().take(16).collect::<String>()
        )
    })
}

async fn exchange_claude_code(
    client: &reqwest::Client,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<OAuthToken, String> {
    // Exact token exchange body from the Claude Code binary (offset ~63928087):
    // grant_type, code, redirect_uri, client_id, code_verifier.
    // NO state field — Claude Code does not send it in the token exchange.
    let req = json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": CLAUDE_CLIENT_ID,
        "code_verifier": verifier,
    });
    let resp = client
        .post(CLAUDE_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("token exchange failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token endpoint returned {status}: {body}"));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("token parse failed: {e}"))?;
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or("no access_token")?
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let exp = v.get("expires_in").and_then(|t| t.as_u64()).unwrap_or(3600);
    let tok = OAuthToken {
        access_token: access,
        refresh_token: refresh,
        expires_at: now_secs() + exp,
        client_id: Some(CLAUDE_CLIENT_ID.to_string()),
        client_secret: None,
        kind: "claude".to_string(),
        id_token: None,
        extra: None,
    };
    store_token("anthropic", &tok);
    Ok(tok)
}

#[derive(Deserialize)]
struct CodexDeviceUserCode {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default, deserialize_with = "deserialize_interval")]
    interval: u64,
}

#[derive(Deserialize)]
struct CodexDeviceToken {
    authorization_code: String,
    code_verifier: String,
}

fn deserialize_interval<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.trim().parse::<u64>().map_err(serde::de::Error::custom)
}

async fn codex_device_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<OAuthToken, String> {
    let base = "https://auth.openai.com/api/accounts";
    let uc: CodexDeviceUserCode = client
        .post(format!("{base}/deviceauth/usercode"))
        .json(&json!({ "client_id": OPENAI_CLIENT_ID }))
        .send()
        .await
        .map_err(|e| format!("device-code request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("device-code request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("device-code parse failed: {e}"))?;

    emit(OAuthPrompt {
        url: OPENAI_DEVICE_VERIFY_URL.to_string(),
        code: Some(uc.user_code.clone()),
        message: format!(
            "SSH/headless detected. Open {OPENAI_DEVICE_VERIFY_URL} on any browser, sign in, and enter this code: {}. Waiting here until it is approved.",
            uc.user_code
        ),
    });

    // Poll the deviceauth/token endpoint until the user completes the browser
    // flow. Mirrors codex-rs/login/src/device_code_auth.rs::poll_for_token:
    // 403/404 means "still pending" (keep polling); success returns an
    // authorization_code + code_verifier to exchange. Deadline is checked
    // INSIDE the pending branch (after a poll), not before a sleep, so we
    // never sleep past the 15-minute cap.
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    loop {
        let resp = client
            .post(format!("{base}/deviceauth/token"))
            .json(&json!({ "device_auth_id": uc.device_auth_id, "user_code": uc.user_code }))
            .send()
            .await
            .map_err(|e| format!("device-code poll failed: {e}"))?;
        let status = resp.status().as_u16();
        if status == 403 || status == 404 {
            if Instant::now() >= deadline {
                return Err("device-code login timed out after 15 minutes".to_string());
            }
            // Sleep the server-suggested interval, but never past the deadline.
            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(remaining.min(Duration::from_secs(uc.interval.max(1)))).await;
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("device-code poll failed: HTTP {}", resp.status()));
        }
        let tok: CodexDeviceToken = resp
            .json()
            .await
            .map_err(|e| format!("device-code token parse failed: {e}"))?;
        return exchange_codex_code(
            client,
            &tok.authorization_code,
            OPENAI_DEVICE_CALLBACK,
            &tok.code_verifier,
        )
        .await;
    }
}

pub async fn codex_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    if likely_headless() {
        return codex_device_login(client, emit)
            .await
            .map(|_| LoginOutcome::Done);
    }

    let verifier = random_b64url(64);
    let challenge = pkce_challenge(&verifier);
    let state = random_b64url(32);

    // Bind the loopback callback server FIRST so the redirect_uri can carry the
    // port we actually bound. Codex's registered redirect URIs are
    // http://localhost:{1455|1457}/auth/callback; prefer 1455, fall back to the
    // registered fallback port 1457 (codex-rs/login/src/server.rs:
    // FALLBACK_PORT) when 1455 is already in use. Listen on both IPv4 and IPv6 —
    // browsers often resolve `localhost` to `::1` before `127.0.0.1`.
    let (listener, listener_v6, port) = bind_codex_callback().await?;
    let redirect = format!("http://localhost:{port}/auth/callback");
    let authorize = format!(
        "{OPENAI_AUTHORIZE_URL}?response_type=code&client_id={OPENAI_CLIENT_ID}&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&state={state}&originator=codex_cli_rs",
        pct_encode(&redirect),
        pct_encode(OPENAI_SCOPE),
    );

    emit(OAuthPrompt {
        url: authorize.clone(),
        code: None,
        message: "Open the URL in a browser to log in to ChatGPT (Codex). After approving, this continues automatically.".to_string(),
    });
    let _ = open_browser(&authorize);

    let code = await_redirect_dual(listener, listener_v6, &state, None).await?;
    exchange_codex_code(client, &code, &redirect, &verifier)
        .await
        .map(|_| LoginOutcome::Done)
}

/// Bind the Codex OAuth loopback callback server. Tries the preferred port
/// (1455), then the registered fallback (1457) — both are in the official CLI's
/// redirect-URI allow-list. Listens on IPv4 and (best-effort) IPv6, since
/// browsers frequently resolve `localhost` to `::1`. Returns the IPv4 listener,
/// an optional IPv6 listener, and the port actually bound (which must be used
/// in the redirect_uri).
async fn bind_codex_callback() -> Result<
    (
        tokio::net::TcpListener,
        Option<tokio::net::TcpListener>,
        u16,
    ),
    String,
> {
    for &port in &[OPENAI_REDIRECT_PORT, OPENAI_REDIRECT_FALLBACK_PORT] {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => {
                let v6 = tokio::net::TcpListener::bind(("::1", port)).await.ok();
                return Ok((listener, v6, port));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(format!("could not bind localhost:{port}: {e}")),
        }
    }
    Err(format!(
        "could not bind localhost:{} or {} for the OAuth callback — both are in use",
        OPENAI_REDIRECT_PORT, OPENAI_REDIRECT_FALLBACK_PORT
    ))
}

pub async fn complete_codex_login(
    client: &reqwest::Client,
    pending: &PendingOauth,
    code: &str,
) -> Result<OAuthToken, String> {
    if let Some(st) = query_param(code, "state") {
        if st != pending.state {
            return Err("OAuth state mismatch".to_string());
        }
    }
    let code = extract_auth_code(code);
    exchange_codex_code(client, &code, &pending.redirect_uri, &pending.code_verifier).await
}

async fn exchange_codex_code(
    client: &reqwest::Client,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> Result<OAuthToken, String> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", OPENAI_CLIENT_ID),
        ("code_verifier", verifier),
    ];
    let resp = client
        .post(OPENAI_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("token exchange failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token endpoint returned {status}: {body}"));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("token parse failed: {e}"))?;
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or("no access_token")?
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let id_token = v.get("id_token").and_then(|t| t.as_str());
    let tok = OAuthToken {
        access_token: access,
        refresh_token: refresh,
        expires_at: id_token.and_then(jwt_exp).unwrap_or(0),
        client_id: Some(OPENAI_CLIENT_ID.to_string()),
        client_secret: None,
        kind: "openai".to_string(),
        id_token: id_token.map(|s| s.to_string()),
        extra: None,
    };
    write_codex_auth(&tok, id_token);
    Ok(tok)
}

/// Which presets support an interactive OAuth login flow here.
pub fn supports_login(preset_id: &str) -> bool {
    matches!(
        preset_id,
        "openai"
            | "gemini"
            | "anthropic"
            | "xai"
            | "qwen"
            | "github"
            | "kimi-coding"
            | "kilocode"
            | "cline"
            | "clinepass"
            | "kimchi"
            | "codebuddy-cn"
            | "iflow"
    )
}

/// Drive the interactive OAuth login for a preset. For the web flow this
/// completes synchronously and returns `LoginOutcome::Done`. For the
/// no-browser flow (Google over SSH/headless) it emits the URL and returns
/// `LoginOutcome::AwaitingCode`; the caller stashes the `pending` state and
/// the user submits the code via the `oauth_code` command, which calls
/// `complete_oauth`. xAI always uses device-code and completes in-process
/// (no paste step).
pub async fn login(
    preset_id: &str,
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    match preset_id {
        "openai" => codex_login(client, emit).await,
        "gemini" => google_login(client, emit).await,
        "anthropic" => claude_login(client, emit).await,
        "xai" => xai_login(client, emit).await,
        "qwen" => qwen_login(client, emit).await,
        "github" => github_login(client, emit).await,
        "kimi-coding" => kimi_coding_login(client, emit).await,
        "kilocode" => kilocode_login(client, emit).await,
        "cline" => cline_login(client, emit, "cline").await,
        "clinepass" => cline_login(client, emit, "clinepass").await,
        "kimchi" => kimchi_login(client, emit).await,
        "codebuddy-cn" => codebuddy_login(client, emit).await,
        "iflow" => iflow_login(client, emit).await,
        other => Err(format!("'{other}' has no OAuth login flow yet")),
    }
}

/// xAI SuperGrok / X Premium+ device-code login (RFC 8628).
///
/// 1. POST `auth.x.ai/oauth2/device/code` with the public Grok-CLI client_id.
/// 2. Emit verification URL + user code (and open the browser when possible).
/// 3. Poll `auth.x.ai/oauth2/token` with `grant_type=device_code` until approved.
/// 4. Persist tokens to `~/.config/catalyst-code/oauth/xai.json`.
///
/// Works over SSH/headless without port forwarding — the user opens the URL on
/// any device. Completes when xAI reports approval (`LoginOutcome::Done`).
/// Progress prompts are re-emitted every ~15s so a headless session does not
/// look stuck while waiting for browser approval.
pub async fn xai_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    let resp = client
        .post(XAI_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[("client_id", XAI_CLIENT_ID), ("scope", XAI_SCOPE)])
        .send()
        .await
        .map_err(|e| format!("xAI device-code request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "xAI device-code request failed (HTTP {status}): {body}"
        ));
    }
    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("xAI device-code parse failed: {e}"))?;
    let device_code = data
        .get("device_code")
        .and_then(|v| v.as_str())
        .ok_or("xAI device-code response missing device_code")?
        .to_string();
    let user_code = data
        .get("user_code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let verification_url = data
        .get("verification_uri_complete")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("verification_uri").and_then(|v| v.as_str()))
        .unwrap_or("https://accounts.x.ai/oauth2/device")
        .to_string();
    let expires_in = data
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(1800);
    let interval = data
        .get("interval")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .max(1);

    let headless = likely_headless()
        || std::env::var("DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_none()
            && std::env::var("WAYLAND_DISPLAY")
                .ok()
                .filter(|s| !s.is_empty())
                .is_none();

    let code_opt = if user_code.is_empty() {
        None
    } else {
        Some(user_code.clone())
    };
    let code_hint = if user_code.is_empty() {
        String::new()
    } else {
        format!(" If prompted, enter code: {user_code}.")
    };
    let open_hint = if headless {
        "This machine has no graphical browser — paste the URL on your laptop (it was copied to your clipboard if your terminal supports OSC 52)."
    } else {
        "Opening your browser if possible — if nothing opens, paste the URL manually."
    };
    let initial_msg = format!(
        "xAI SuperGrok OAuth: open the URL below, sign in, and APPROVE access.{code_hint} {open_hint} Waiting for approval (auto-completes within a few seconds after you approve; polls every {interval}s, expires in {expires_in}s)…"
    );
    emit(OAuthPrompt {
        url: verification_url.clone(),
        code: code_opt.clone(),
        message: initial_msg,
    });
    if !headless {
        if let Err(e) = open_browser(&verification_url) {
            emit(OAuthPrompt {
                url: verification_url.clone(),
                code: code_opt.clone(),
                message: format!(
                    "Could not open a browser automatically ({e}). Paste this URL on any device and approve:{code_hint}"
                ),
            });
        }
    }

    let deadline = Instant::now() + Duration::from_secs(expires_in);
    let mut current_interval = interval;
    let mut last_progress = Instant::now();
    let started = Instant::now();
    // Poll immediately, then sleep on authorization_pending. After the user
    // clicks Approve in the browser, the next poll (≤ interval) completes.
    loop {
        if Instant::now() >= deadline {
            return Err(
                "Timed out waiting for xAI device authorization. Open the verification URL, click Approve, then run /login again."
                    .to_string(),
            );
        }
        let poll = client
            .post(XAI_TOKEN_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", XAI_CLIENT_ID),
                ("device_code", device_code.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("xAI device-code poll failed: {e}"))?;

        let status = poll.status();
        let body: Value = poll.json().await.unwrap_or(Value::Null);

        if status.is_success() {
            let access = body
                .get("access_token")
                .and_then(|t| t.as_str())
                .ok_or("xAI token response missing access_token")?
                .to_string();
            let refresh = body
                .get("refresh_token")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
                .ok_or(
                    "xAI token response missing refresh_token — re-run /login and fully Approve in the browser",
                )?;
            let expires_in_tok = body
                .get("expires_in")
                .and_then(|t| t.as_u64())
                .unwrap_or(21600);
            let id_token = body
                .get("id_token")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());
            let tok = OAuthToken {
                access_token: access,
                refresh_token: Some(refresh),
                expires_at: now_secs().saturating_add(expires_in_tok),
                client_id: Some(XAI_CLIENT_ID.to_string()),
                client_secret: None,
                kind: "xai".to_string(),
                id_token,
                extra: None,
            };
            store_token("xai", &tok).ok_or("could not write xAI OAuth credentials to disk")?;
            return Ok(LoginOutcome::Done);
        }

        // Pending / slow_down / errors. xAI returns authorization_pending as
        // a non-2xx body with {"error":"authorization_pending",...}.
        let err = body.get("error").and_then(|e| e.as_str()).unwrap_or("");
        match err {
            "authorization_pending" | "slow_down" => {
                if err == "slow_down" {
                    current_interval = (current_interval + 1).min(30);
                }
                // Heartbeat so a 30s wait doesn't look hung — especially over
                // SSH where no browser ever pops open on this machine.
                if last_progress.elapsed() >= Duration::from_secs(15) {
                    let elapsed = started.elapsed().as_secs();
                    let left = deadline.saturating_duration_since(Instant::now()).as_secs();
                    emit(OAuthPrompt {
                        url: verification_url.clone(),
                        code: code_opt.clone(),
                        message: format!(
                            "Still waiting for xAI approval… ({elapsed}s elapsed, ~{left}s left). Open the URL, sign in with SuperGrok / X Premium+, and click Approve — login finishes automatically after that.{code_hint}"
                        ),
                    });
                    last_progress = Instant::now();
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                tokio::time::sleep(remaining.min(Duration::from_secs(current_interval))).await;
                continue;
            }
            other if other.is_empty() => {
                // Non-JSON or unexpected body — don't hang forever; surface it.
                return Err(format!(
                    "xAI device-code token polling failed (HTTP {status}): {body}"
                ));
            }
            other => {
                let desc = body
                    .get("error_description")
                    .and_then(|d| d.as_str())
                    .unwrap_or(other);
                return Err(format!("xAI device-code token polling failed: {desc}"));
            }
        }
    }
}

/// Qwen Code device-code login (RFC 8628 + PKCE).
///
/// Matches 9router's qwen provider:
/// 1. POST `chat.qwen.ai/api/v1/oauth2/device/code` with client_id + scope + PKCE challenge.
/// 2. Emit verification URL + user code.
/// 3. Poll token endpoint with device_code + code_verifier until approved.
/// 4. Persist tokens to `~/.config/catalyst-code/oauth/qwen.json`.
pub async fn qwen_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    // 9router uses a 96-byte random verifier (base64url).
    let verifier = random_b64url(96);
    let challenge = pkce_challenge(&verifier);

    let resp = client
        .post(QWEN_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", QWEN_CLIENT_ID),
            ("scope", QWEN_SCOPE),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await
        .map_err(|e| format!("Qwen device-code request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Qwen device-code request failed (HTTP {status}): {body}"
        ));
    }
    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("Qwen device-code parse failed: {e}"))?;
    let device_code = data
        .get("device_code")
        .and_then(|v| v.as_str())
        .ok_or("Qwen device-code response missing device_code")?
        .to_string();
    let user_code = data
        .get("user_code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let verification_url = data
        .get("verification_uri_complete")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("verification_uri").and_then(|v| v.as_str()))
        .unwrap_or("https://chat.qwen.ai")
        .to_string();
    let expires_in = data
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(1800);
    let interval = data
        .get("interval")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .max(1);

    let headless = likely_headless();
    let code_opt = if user_code.is_empty() {
        None
    } else {
        Some(user_code.clone())
    };
    let code_hint = if user_code.is_empty() {
        String::new()
    } else {
        format!(" If prompted, enter code: {user_code}.")
    };
    let open_hint = if headless {
        "This machine has no graphical browser — paste the URL on your laptop."
    } else {
        "Opening your browser if possible — if nothing opens, paste the URL manually."
    };
    emit(OAuthPrompt {
        url: verification_url.clone(),
        code: code_opt.clone(),
        message: format!(
            "Qwen Code OAuth: open the URL below, sign in, and APPROVE access.{code_hint} {open_hint} Waiting for approval (polls every {interval}s, expires in {expires_in}s)…"
        ),
    });
    if !headless {
        let _ = open_browser(&verification_url);
    }

    let deadline = Instant::now() + Duration::from_secs(expires_in);
    let mut current_interval = interval;
    let mut last_progress = Instant::now();
    let started = Instant::now();
    loop {
        if Instant::now() >= deadline {
            return Err(
                "Timed out waiting for Qwen device authorization. Open the verification URL, approve, then run /login again."
                    .to_string(),
            );
        }
        let poll = client
            .post(QWEN_TOKEN_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", QWEN_CLIENT_ID),
                ("device_code", device_code.as_str()),
                ("code_verifier", verifier.as_str()),
            ])
            .send()
            .await
            .map_err(|e| format!("Qwen device-code poll failed: {e}"))?;

        let status = poll.status();
        let body: Value = poll.json().await.unwrap_or(Value::Null);

        if status.is_success() {
            let access = body
                .get("access_token")
                .and_then(|t| t.as_str())
                .ok_or("Qwen token response missing access_token")?
                .to_string();
            let refresh = body
                .get("refresh_token")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());
            let expires_in_tok = body
                .get("expires_in")
                .and_then(|t| t.as_u64())
                .unwrap_or(3600);
            let tok = OAuthToken {
                access_token: access,
                refresh_token: refresh,
                expires_at: now_secs().saturating_add(expires_in_tok),
                client_id: Some(QWEN_CLIENT_ID.to_string()),
                client_secret: None,
                kind: "qwen".to_string(),
                id_token: body
                    .get("id_token")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string()),
                extra: None,
            };
            store_token("qwen", &tok).ok_or("could not write Qwen OAuth credentials to disk")?;
            return Ok(LoginOutcome::Done);
        }

        let err = body.get("error").and_then(|e| e.as_str()).unwrap_or("");
        match err {
            "authorization_pending" | "slow_down" => {
                if err == "slow_down" {
                    current_interval = (current_interval + 1).min(30);
                }
                if last_progress.elapsed() >= Duration::from_secs(15) {
                    let elapsed = started.elapsed().as_secs();
                    let left = deadline.saturating_duration_since(Instant::now()).as_secs();
                    emit(OAuthPrompt {
                        url: verification_url.clone(),
                        code: code_opt.clone(),
                        message: format!(
                            "Still waiting for Qwen approval… ({elapsed}s elapsed, ~{left}s left). Open the URL and approve — login finishes automatically after that.{code_hint}"
                        ),
                    });
                    last_progress = Instant::now();
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                tokio::time::sleep(remaining.min(Duration::from_secs(current_interval))).await;
                continue;
            }
            other if other.is_empty() => {
                return Err(format!(
                    "Qwen device-code token polling failed (HTTP {status}): {body}"
                ));
            }
            other => {
                let desc = body
                    .get("error_description")
                    .and_then(|d| d.as_str())
                    .unwrap_or(other);
                return Err(format!("Qwen device-code token polling failed: {desc}"));
            }
        }
    }
}

/// Request an RFC 8628 device code and present its verification URL.
async fn request_device_code(
    client: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
    provider: &str,
    fallback_url: &str,
) -> Result<(String, Option<String>, String, u64, u64), String> {
    let resp = client
        .post(endpoint)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(form)
        .send()
        .await
        .map_err(|e| format!("{provider} device-code request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "{provider} device-code request failed (HTTP {status}): {body}"
        ));
    }
    let device_code = body
        .get("device_code")
        .or_else(|| body.get("code"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{provider} device-code response missing device_code"))?
        .to_string();
    let user_code = body
        .get("user_code")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let url = body
        .get("verification_uri_complete")
        .or_else(|| body.get("verificationUrl"))
        .or_else(|| body.get("verification_uri"))
        .and_then(Value::as_str)
        .unwrap_or(fallback_url)
        .to_string();
    let expires = body
        .get("expires_in")
        .or_else(|| body.get("expiresIn"))
        .and_then(Value::as_u64)
        .unwrap_or(1800);
    let interval = body
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .max(1);
    Ok((device_code, user_code, url, expires, interval))
}

fn emit_device_prompt(
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
    provider: &str,
    url: &str,
    code: &Option<String>,
    interval: u64,
    expires: u64,
) {
    let code_hint = code
        .as_deref()
        .map(|c| format!(" Enter code: {c}."))
        .unwrap_or_default();
    emit(OAuthPrompt {
        url: url.to_string(),
        code: code.clone(),
        message: format!(
            "{provider} OAuth: open the URL below, sign in, and approve access.{code_hint} Waiting for approval (polls every {interval}s; expires in {expires}s)…"
        ),
    });
    if !likely_headless() {
        let _ = open_browser(url);
    }
}

async fn github_copilot_session_token(
    client: &reqwest::Client,
    github_token: &str,
) -> Result<(String, u64), String> {
    let resp = client
        .get(GITHUB_COPILOT_TOKEN_URL)
        .header("Accept", "application/json")
        .header("Authorization", format!("token {github_token}"))
        .header("User-Agent", "GitHubCopilotChat/0.26.7")
        .header("Editor-Version", "vscode/1.85.0")
        .header("Editor-Plugin-Version", "copilot-chat/0.26.7")
        .send()
        .await
        .map_err(|e| format!("GitHub Copilot token request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "GitHub Copilot token request failed (HTTP {status}): {body}"
        ));
    }
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .ok_or("GitHub Copilot token response missing token")?
        .to_string();
    let expires_at = body
        .get("expires_at")
        .and_then(Value::as_u64)
        .or_else(|| jwt_exp(&token))
        .unwrap_or_else(|| now_secs().saturating_add(1800));
    Ok((token, expires_at))
}

async fn refresh_github_copilot_token(
    client: &reqwest::Client,
    tok: &OAuthToken,
) -> Option<String> {
    let github_token = tok.extra.as_ref()?.get("github_token")?.as_str()?;
    let (access, expires_at) = github_copilot_session_token(client, github_token)
        .await
        .ok()?;
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    updated.expires_at = expires_at;
    let _ = store_token("github", &updated);
    Some(access)
}

async fn github_token(client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("github")?;
    if tok.access_token.is_empty() {
        return None;
    }
    if tok.expires_at == 0 || tok.expires_at > now_secs().saturating_add(GITHUB_REFRESH_SKEW_SECS) {
        return Some(tok.access_token);
    }
    refresh_github_copilot_token(client, &tok).await
}

fn kimi_oauth_request(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    crate::config::kimi_coding_headers()
        .into_iter()
        .fold(req, |req, (key, value)| req.header(key, value))
}

async fn refresh_kimi_coding_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    let refresh = tok.refresh_token.as_deref()?;
    let resp = kimi_oauth_request(
        client
            .post(KIMI_CODING_TOKEN_URL)
            .header("Accept", "application/json"),
    )
    .form(&[
        ("grant_type", "refresh_token"),
        ("client_id", KIMI_CODING_CLIENT_ID),
        ("refresh_token", refresh),
    ])
    .send()
    .await
    .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let access = v.get("access_token")?.as_str()?.to_string();
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    updated.expires_at =
        now_secs().saturating_add(v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600));
    if let Some(refresh) = v.get("refresh_token").and_then(Value::as_str) {
        updated.refresh_token = Some(refresh.to_string());
    }
    let _ = store_token("kimi-coding", &updated);
    Some(access)
}

async fn kimi_coding_token(client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("kimi-coding")?;
    if tok.access_token.is_empty() {
        return None;
    }
    if tok.expires_at == 0
        || tok.expires_at > now_secs().saturating_add(KIMI_CODING_REFRESH_SKEW_SECS)
    {
        return Some(tok.access_token);
    }
    refresh_kimi_coding_token(client, &tok).await
}

async fn kilocode_token(_client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("kilocode")?;
    if tok.access_token.is_empty() {
        return None;
    }
    // Kilo's device token is long-lived and its current API has no refresh
    // grant. An expired token is treated as a re-login requirement.
    if tok.expires_at != 0
        && tok.expires_at <= now_secs().saturating_add(KILOCODE_REFRESH_SKEW_SECS)
    {
        return None;
    }
    Some(tok.access_token)
}

/// GitHub Copilot OAuth: GitHub device code followed by the documented
/// Copilot session-token exchange. The stored access_token is only the
/// short-lived Copilot token; the GitHub token stays in `extra` so sessions
/// refresh safely without exposing it to model requests.
pub async fn github_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    let (device_code, code, url, expires, interval) = request_device_code(
        client,
        GITHUB_DEVICE_CODE_URL,
        &[("client_id", GITHUB_CLIENT_ID), ("scope", GITHUB_SCOPE)],
        "GitHub Copilot",
        "https://github.com/login/device",
    )
    .await?;
    emit_device_prompt(emit, "GitHub Copilot", &url, &code, interval, expires);
    let deadline = Instant::now() + Duration::from_secs(expires);
    let mut wait = interval;
    loop {
        if Instant::now() >= deadline {
            return Err(
                "Timed out waiting for GitHub device authorization. Run /login again.".into(),
            );
        }
        let resp = client
            .post(GITHUB_TOKEN_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", GITHUB_CLIENT_ID),
                ("device_code", device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|e| format!("GitHub device-code poll failed: {e}"))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() && v.get("access_token").and_then(Value::as_str).is_some() {
            let github_token = v.get("access_token").and_then(Value::as_str).unwrap();
            let (copilot_token, expires_at) =
                github_copilot_session_token(client, github_token).await?;
            let tok = OAuthToken {
                access_token: copilot_token,
                refresh_token: None,
                expires_at,
                client_id: Some(GITHUB_CLIENT_ID.into()),
                client_secret: None,
                kind: "github".into(),
                id_token: None,
                extra: Some(serde_json::json!({"github_token": github_token})),
            };
            store_token("github", &tok).ok_or("could not write GitHub Copilot credentials")?;
            return Ok(LoginOutcome::Done);
        }
        match v.get("error").and_then(Value::as_str).unwrap_or("") {
            "authorization_pending" => {}
            "slow_down" => wait = (wait + 5).min(30),
            "expired_token" => return Err("GitHub device code expired; run /login again.".into()),
            "access_denied" => return Err("GitHub device authorization was denied.".into()),
            _ => {
                return Err(format!(
                    "GitHub device-code poll failed (HTTP {status}): {v}"
                ))
            }
        }
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

/// Kimi Coding OAuth device-code login. Kimi's public client is identical to
/// 9router's `kimi-coding` registry flow.
pub async fn kimi_coding_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    let resp = kimi_oauth_request(
        client
            .post(KIMI_CODING_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded"),
    )
    .form(&[("client_id", KIMI_CODING_CLIENT_ID)])
    .send()
    .await
    .map_err(|e| format!("Kimi Coding device-code request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "Kimi Coding device-code request failed (HTTP {status}): {body}"
        ));
    }
    let device_code = body
        .get("device_code")
        .and_then(Value::as_str)
        .ok_or("Kimi Coding device-code response missing device_code")?
        .to_string();
    let code = body
        .get("user_code")
        .and_then(Value::as_str)
        .map(String::from);
    let url = body
        .get("verification_uri_complete")
        .or_else(|| body.get("verification_uri"))
        .and_then(Value::as_str)
        .unwrap_or("https://www.kimi.com/code/authorize_device")
        .to_string();
    let expires = body
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(1800);
    let interval = body
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .max(1);
    emit_device_prompt(emit, "Kimi Coding", &url, &code, interval, expires);
    let deadline = Instant::now() + Duration::from_secs(expires);
    loop {
        if Instant::now() >= deadline {
            return Err(
                "Timed out waiting for Kimi Coding authorization. Run /login again.".into(),
            );
        }
        let resp = kimi_oauth_request(
            client
                .post(KIMI_CODING_TOKEN_URL)
                .header("Accept", "application/json"),
        )
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", KIMI_CODING_CLIENT_ID),
            ("device_code", device_code.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Kimi Coding device-code poll failed: {e}"))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() && v.get("access_token").and_then(Value::as_str).is_some() {
            let tok = OAuthToken {
                access_token: v
                    .get("access_token")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
                refresh_token: v
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .map(String::from),
                expires_at: now_secs()
                    .saturating_add(v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600)),
                client_id: Some(KIMI_CODING_CLIENT_ID.into()),
                client_secret: None,
                kind: "kimi-coding".into(),
                id_token: None,
                extra: None,
            };
            store_token("kimi-coding", &tok).ok_or("could not write Kimi Coding credentials")?;
            return Ok(LoginOutcome::Done);
        }
        match v.get("error").and_then(Value::as_str).unwrap_or("") {
            "authorization_pending" => {}
            "slow_down" => {}
            "expired_token" => {
                return Err("Kimi Coding device code expired; run /login again.".into())
            }
            "access_denied" => return Err("Kimi Coding authorization was denied.".into()),
            _ => {
                return Err(format!(
                    "Kimi Coding device-code poll failed (HTTP {status}): {v}"
                ))
            }
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

/// Kilo Code's non-standard device authorization. The approval response
/// returns a long-lived gateway token; a profile request provides the optional
/// organization header required by some Kilo accounts.
pub async fn kilocode_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    let resp = client
        .post(KILOCODE_INITIATE_URL)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Kilo Code device authorization failed: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "Kilo Code device authorization failed (HTTP {status}): {v}"
        ));
    }
    let code = v
        .get("code")
        .and_then(Value::as_str)
        .ok_or("Kilo Code response missing code")?
        .to_string();
    let url = v
        .get("verificationUrl")
        .and_then(Value::as_str)
        .unwrap_or("https://app.kilo.ai")
        .to_string();
    let expires = v.get("expiresIn").and_then(Value::as_u64).unwrap_or(300);
    let code_opt = Some(code.clone());
    emit_device_prompt(emit, "Kilo Code", &url, &code_opt, 3, expires);
    let deadline = Instant::now() + Duration::from_secs(expires);
    loop {
        if Instant::now() >= deadline {
            return Err("Timed out waiting for Kilo Code authorization. Run /login again.".into());
        }
        let resp = client
            .get(format!("{KILOCODE_POLL_BASE_URL}/{code}"))
            .send()
            .await
            .map_err(|e| format!("Kilo Code authorization poll failed: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::ACCEPTED {
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err("Kilo Code authorization was denied.".into());
        }
        if status == reqwest::StatusCode::GONE {
            return Err("Kilo Code device code expired; run /login again.".into());
        }
        if !status.is_success() {
            return Err(format!(
                "Kilo Code authorization poll failed (HTTP {status}): {v}"
            ));
        }
        if v.get("status").and_then(Value::as_str) != Some("approved") {
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }
        let access = v
            .get("token")
            .and_then(Value::as_str)
            .ok_or("Kilo Code approval missing token")?
            .to_string();
        let profile = match client
            .get("https://api.kilo.ai/api/profile")
            .header("Authorization", format!("Bearer {access}"))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp.json::<Value>().await.ok(),
            _ => None,
        };
        let org_id = profile
            .as_ref()
            .and_then(|p| p.get("organizations"))
            .and_then(Value::as_array)
            .and_then(|o| o.first())
            .and_then(|o| o.get("id"))
            .and_then(Value::as_str);
        let tok = OAuthToken {
            access_token: access,
            refresh_token: None,
            expires_at: 0,
            client_id: None,
            client_secret: None,
            kind: "kilocode".into(),
            id_token: None,
            extra: org_id.map(|id| serde_json::json!({"organization_id": id})),
        };
        store_token("kilocode", &tok).ok_or("could not write Kilo Code credentials")?;
        return Ok(LoginOutcome::Done);
    }
}

// --- Cline / ClinePass -------------------------------------------------------

fn cline_workos_token(token: &str) -> String {
    let t = token.trim();
    if t.is_empty() || t.starts_with("workos:") {
        t.to_string()
    } else {
        format!("workos:{t}")
    }
}

fn parse_iso_expires_at(s: &str) -> Option<u64> {
    // Accept RFC3339 / ISO-8601 timestamps when present; otherwise None.
    // We only need a coarse unix-seconds bound for refresh skew.
    // Prefer chrono-free parsing of common "2025-01-01T00:00:00.000Z" form.
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // If it's already a unix seconds number:
    if let Ok(n) = s.parse::<u64>() {
        return Some(if n > 1_000_000_000_000 { n / 1000 } else { n });
    }
    None
}

fn decode_cline_embedded_token(code: &str) -> Option<(String, Option<String>, Option<u64>)> {
    // Cline often returns base64(JSON{accessToken,refreshToken,expiresAt}) in the
    // `code` query param of the redirect URL (9router cline.exchangeToken).
    let raw = extract_auth_code(code);
    let mut b64 = raw.trim().to_string();
    // URL-safe base64 sometimes appears without padding.
    b64 = b64.replace('-', "+").replace('_', "/");
    let pad = (4 - (b64.len() % 4)) % 4;
    b64.push_str(&"=".repeat(pad));
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let last = text.rfind('}')?;
    let v: Value = serde_json::from_str(&text[..=last]).ok()?;
    let access = v
        .get("accessToken")
        .or_else(|| v.get("access_token"))
        .and_then(Value::as_str)?
        .to_string();
    let refresh = v
        .get("refreshToken")
        .or_else(|| v.get("refresh_token"))
        .and_then(Value::as_str)
        .map(String::from);
    let expires_at = v
        .get("expiresAt")
        .or_else(|| v.get("expires_at"))
        .and_then(|x| {
            x.as_str()
                .and_then(parse_iso_expires_at)
                .or_else(|| x.as_u64())
        });
    Some((access, refresh, expires_at))
}

async fn refresh_cline_family_token(
    client: &reqwest::Client,
    store: &str,
    tok: &OAuthToken,
) -> Option<String> {
    let refresh = tok.refresh_token.as_deref()?;
    let resp = client
        .post(CLINE_REFRESH_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "refreshToken": refresh,
            "grantType": "refresh_token",
            "clientType": "extension",
        }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let payload: Value = resp.json().await.ok()?;
    let data = payload.get("data").cloned().unwrap_or(payload);
    let mut access = data.get("accessToken").and_then(Value::as_str)?.to_string();
    access = cline_workos_token(&access);
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    if let Some(rt) = data.get("refreshToken").and_then(Value::as_str) {
        updated.refresh_token = Some(rt.to_string());
    }
    if let Some(exp) = data
        .get("expiresAt")
        .and_then(Value::as_str)
        .and_then(parse_iso_expires_at)
    {
        updated.expires_at = exp;
    } else if let Some(secs) = data.get("expiresIn").and_then(Value::as_u64) {
        updated.expires_at = now_secs().saturating_add(secs);
    }
    let _ = store_token(store, &updated);
    Some(access)
}

async fn cline_family_token(client: &reqwest::Client, store: &str) -> Option<String> {
    let tok = read_stored_token(store)?;
    if tok.access_token.is_empty() {
        return None;
    }
    if tok.expires_at == 0 || tok.expires_at > now_secs().saturating_add(CLINE_REFRESH_SKEW_SECS) {
        return Some(tok.access_token);
    }
    refresh_cline_family_token(client, store, &tok).await
}

/// Cline / ClinePass browser OAuth. Emits the authorize URL and returns
/// `AwaitingCode` so the user pastes the redirect URL (or embedded token) via
/// `/oauth-code`.
pub async fn cline_login(
    _client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
    store: &str,
) -> Result<LoginOutcome, String> {
    let state = random_b64url(16);
    let params = [
        ("client_type", "extension"),
        ("callback_url", CLINE_REDIRECT_URI),
        ("redirect_uri", CLINE_REDIRECT_URI),
        ("state", state.as_str()),
    ];
    let qs = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let url = format!("{CLINE_AUTHORIZE_URL}?{qs}");
    let label = if store == "clinepass" {
        "ClinePass"
    } else {
        "Cline"
    };
    emit(OAuthPrompt {
        url: url.clone(),
        code: None,
        message: format!(
            "{label} OAuth: open the URL, sign in, then paste the final redirect URL (or the code/token it contains) with /oauth-code <value>."
        ),
    });
    if !likely_headless() {
        let _ = open_browser(&url);
    }
    Ok(LoginOutcome::AwaitingCode {
        pending: PendingOauth {
            kind: store.to_string(),
            code_verifier: String::new(),
            state,
            redirect_uri: CLINE_REDIRECT_URI.to_string(),
            plugin_pending: None,
        },
    })
}

pub async fn complete_cline_login(
    client: &reqwest::Client,
    pending: &PendingOauth,
    code: &str,
) -> Result<OAuthToken, String> {
    let store = pending.kind.as_str();
    // 1) Try embedded base64 token payload first (common Cline redirect shape).
    if let Some((access, refresh, expires_at)) = decode_cline_embedded_token(code) {
        let tok = OAuthToken {
            access_token: cline_workos_token(&access),
            refresh_token: refresh,
            expires_at: expires_at.unwrap_or(0),
            client_id: None,
            client_secret: None,
            kind: store.to_string(),
            id_token: None,
            extra: None,
        };
        store_token(store, &tok).ok_or("could not write Cline credentials")?;
        return Ok(tok);
    }
    // 2) Fall back to authorization_code exchange.
    let code = extract_auth_code(code);
    let resp = client
        .post(CLINE_TOKEN_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "client_type": "extension",
            "redirect_uri": pending.redirect_uri,
        }))
        .send()
        .await
        .map_err(|e| format!("Cline token exchange failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "Cline token exchange failed (HTTP {status}): {body}"
        ));
    }
    let data = body.get("data").cloned().unwrap_or(body);
    let access = data
        .get("accessToken")
        .or_else(|| data.get("access_token"))
        .and_then(Value::as_str)
        .ok_or("Cline token response missing accessToken")?
        .to_string();
    let refresh = data
        .get("refreshToken")
        .or_else(|| data.get("refresh_token"))
        .and_then(Value::as_str)
        .map(String::from);
    let expires_at = data
        .get("expiresAt")
        .and_then(Value::as_str)
        .and_then(parse_iso_expires_at)
        .unwrap_or(0);
    let tok = OAuthToken {
        access_token: cline_workos_token(&access),
        refresh_token: refresh,
        expires_at,
        client_id: None,
        client_secret: None,
        kind: store.to_string(),
        id_token: None,
        extra: None,
    };
    store_token(store, &tok).ok_or("could not write Cline credentials")?;
    Ok(tok)
}

// --- Kimchi ------------------------------------------------------------------

async fn kimchi_token(_client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("kimchi")?;
    if tok.access_token.is_empty() {
        None
    } else {
        Some(tok.access_token)
    }
}

/// Kimchi browser-token login: open cli-auth, paste the issued token via
/// `/oauth-code`.
pub async fn kimchi_login(
    _client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    let state = random_b64url(16);
    let redirect = "http://localhost:1456/callback";
    let url = format!(
        "{KIMCHI_WEB_APP_URL}/cli-auth?callback={}&state={}",
        urlencoding_encode(redirect),
        urlencoding_encode(&state)
    );
    emit(OAuthPrompt {
        url: url.clone(),
        code: None,
        message: "Kimchi OAuth: open the URL, sign in, then paste the access token with /oauth-code <token>.".into(),
    });
    if !likely_headless() {
        let _ = open_browser(&url);
    }
    Ok(LoginOutcome::AwaitingCode {
        pending: PendingOauth {
            kind: "kimchi".into(),
            code_verifier: String::new(),
            state,
            redirect_uri: redirect.into(),
            plugin_pending: None,
        },
    })
}

pub async fn complete_kimchi_login(
    client: &reqwest::Client,
    _pending: &PendingOauth,
    code: &str,
) -> Result<OAuthToken, String> {
    let access = extract_auth_code(code);
    if access.is_empty() {
        return Err("Missing Kimchi token".into());
    }
    let resp = client
        .get(KIMCHI_VALIDATION_URL)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await
        .map_err(|e| format!("Kimchi token validation failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "Kimchi token validation failed (HTTP {})",
            resp.status()
        ));
    }
    // Best-effort userinfo (non-fatal).
    let _ = client
        .get(KIMCHI_USERINFO_URL)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {access}"))
        .send()
        .await;
    let tok = OAuthToken {
        access_token: access,
        refresh_token: None,
        expires_at: 0,
        client_id: None,
        client_secret: None,
        kind: "kimchi".into(),
        id_token: None,
        extra: None,
    };
    store_token("kimchi", &tok).ok_or("could not write Kimchi credentials")?;
    Ok(tok)
}

// --- CodeBuddy CN ------------------------------------------------------------

fn codebuddy_auth_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Accept", "application/json"),
        ("User-Agent", CODEBUDDY_USER_AGENT),
        ("X-Requested-With", "XMLHttpRequest"),
        ("X-Domain", "copilot.tencent.com"),
        ("X-No-Authorization", "true"),
        ("X-No-User-Id", "true"),
        ("X-Product", "SaaS"),
    ]
}

async fn refresh_codebuddy_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    let refresh = tok.refresh_token.as_deref()?;
    let mut req = client
        .post(CODEBUDDY_REFRESH_URL)
        .header("Content-Type", "application/json")
        .header("X-Refresh-Token", refresh)
        .header("X-Auth-Refresh-Source", "plugin")
        .body("{}");
    for (k, v) in codebuddy_auth_headers() {
        req = req.header(k, v);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    let data = body.get("data").cloned().unwrap_or(body);
    let access = data
        .get("accessToken")
        .or_else(|| data.get("access_token"))
        .and_then(Value::as_str)?
        .to_string();
    let mut updated = tok.clone();
    updated.access_token = access.clone();
    if let Some(rt) = data
        .get("refreshToken")
        .or_else(|| data.get("refresh_token"))
        .and_then(Value::as_str)
    {
        updated.refresh_token = Some(rt.to_string());
    }
    if let Some(secs) = data
        .get("expiresIn")
        .or_else(|| data.get("expires_in"))
        .and_then(Value::as_u64)
    {
        updated.expires_at = now_secs().saturating_add(secs);
    }
    let _ = store_token("codebuddy-cn", &updated);
    Some(access)
}

async fn codebuddy_token(client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("codebuddy-cn")?;
    if tok.access_token.is_empty() {
        return None;
    }
    if tok.expires_at == 0
        || tok.expires_at > now_secs().saturating_add(CODEBUDDY_REFRESH_SKEW_SECS)
    {
        return Some(tok.access_token);
    }
    refresh_codebuddy_token(client, &tok).await
}

/// Tencent CodeBuddy CN device-style login (state + poll), matching 9router.
pub async fn codebuddy_login(
    client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    let mut req = client
        .post(format!(
            "{CODEBUDDY_STATE_URL}?platform={CODEBUDDY_PLATFORM}"
        ))
        .header("Content-Type", "application/json")
        .body("{}");
    for (k, v) in codebuddy_auth_headers() {
        req = req.header(k, v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("CodeBuddy state request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "CodeBuddy state request failed (HTTP {status}): {body}"
        ));
    }
    if body.get("code").and_then(Value::as_i64) != Some(0) {
        return Err(format!(
            "CodeBuddy state error: {}",
            body.get("msg").and_then(Value::as_str).unwrap_or("unknown")
        ));
    }
    let data = body.get("data").cloned().unwrap_or(Value::Null);
    let state = data
        .get("state")
        .and_then(Value::as_str)
        .ok_or("CodeBuddy state response missing state")?
        .to_string();
    let url = data
        .get("authUrl")
        .and_then(Value::as_str)
        .ok_or("CodeBuddy state response missing authUrl")?
        .to_string();
    emit_device_prompt(emit, "CodeBuddy", &url, &None, 5, 600);
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        if Instant::now() >= deadline {
            return Err("Timed out waiting for CodeBuddy authorization. Run /login again.".into());
        }
        let mut req = client.get(format!(
            "{CODEBUDDY_TOKEN_URL}?state={}",
            urlencoding_encode(&state)
        ));
        for (k, v) in codebuddy_auth_headers() {
            req = req.header(k, v);
        }
        // Extra "no enterprise" headers used by the official poll path.
        req = req
            .header("X-No-Enterprise-Id", "true")
            .header("X-No-Department-Info", "true");
        let resp = req
            .send()
            .await
            .map_err(|e| format!("CodeBuddy token poll failed: {e}"))?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(format!(
                "CodeBuddy token poll failed (HTTP {status}): {body}"
            ));
        }
        let code = body.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code == 0 {
            let data = body.get("data").cloned().unwrap_or(Value::Null);
            let access = data
                .get("accessToken")
                .and_then(Value::as_str)
                .ok_or("CodeBuddy approval missing accessToken")?
                .to_string();
            let refresh = data
                .get("refreshToken")
                .and_then(Value::as_str)
                .map(String::from);
            let expires_in = data
                .get("expiresIn")
                .and_then(Value::as_u64)
                .unwrap_or(86400);
            let tok = OAuthToken {
                access_token: access,
                refresh_token: refresh,
                expires_at: now_secs().saturating_add(expires_in),
                client_id: None,
                client_secret: None,
                kind: "codebuddy-cn".into(),
                id_token: None,
                extra: None,
            };
            store_token("codebuddy-cn", &tok).ok_or("could not write CodeBuddy credentials")?;
            return Ok(LoginOutcome::Done);
        }
        // 11217 = pending (RetryFetchToken)
        if code == 11217 {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        return Err(format!(
            "CodeBuddy token poll failed: {}",
            body.get("msg").and_then(Value::as_str).unwrap_or("unknown")
        ));
    }
}

// --- iFlow -------------------------------------------------------------------

fn iflow_basic_auth() -> String {
    use base64::Engine;
    let raw = format!("{IFLOW_CLIENT_ID}:{IFLOW_CLIENT_SECRET}");
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
    )
}

async fn iflow_fetch_api_key(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<String, String> {
    let url = format!(
        "{IFLOW_USERINFO_URL}?accessToken={}",
        urlencoding_encode(access_token)
    );
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("iFlow userInfo failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("iFlow userInfo failed (HTTP {status}): {body}"));
    }
    if body.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(format!(
            "iFlow userInfo failed: {}",
            body.get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    let data = body.get("data").cloned().unwrap_or(body);
    let api_key = data
        .get("apiKey")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or("Empty API key returned from iFlow userInfo")?
        .to_string();
    Ok(api_key)
}

async fn refresh_iflow_token(client: &reqwest::Client, tok: &OAuthToken) -> Option<String> {
    // Refresh the OAuth access token, then re-fetch the account apiKey used for
    // chat (9router stores apiKey from userInfo for request signing/auth).
    let refresh = tok.refresh_token.as_deref()?;
    let resp = client
        .post(IFLOW_TOKEN_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Authorization", iflow_basic_auth())
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", IFLOW_CLIENT_ID),
            ("client_secret", IFLOW_CLIENT_SECRET),
        ])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let access = v.get("access_token")?.as_str()?.to_string();
    let api_key = iflow_fetch_api_key(client, &access).await.ok()?;
    let mut updated = tok.clone();
    updated.access_token = api_key.clone();
    updated.expires_at =
        now_secs().saturating_add(v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600));
    if let Some(rt) = v.get("refresh_token").and_then(Value::as_str) {
        updated.refresh_token = Some(rt.to_string());
    }
    // Keep the raw OAuth access token in extra for debugging/future refresh paths.
    updated.extra = Some(serde_json::json!({ "oauth_access_token": access }));
    let _ = store_token("iflow", &updated);
    Some(api_key)
}

async fn iflow_token(client: &reqwest::Client) -> Option<String> {
    let tok = read_stored_token("iflow")?;
    if tok.access_token.is_empty() {
        return None;
    }
    // access_token field holds the account apiKey used for chat.
    if tok.expires_at == 0 || tok.expires_at > now_secs().saturating_add(IFLOW_REFRESH_SKEW_SECS) {
        return Some(tok.access_token);
    }
    refresh_iflow_token(client, &tok).await
}

/// iFlow browser OAuth. Emits authorize URL; user pastes the redirect URL or
/// code via `/oauth-code`. On success we store the account apiKey (not the
/// short-lived OAuth token) as the request credential.
pub async fn iflow_login(
    _client: &reqwest::Client,
    emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
) -> Result<LoginOutcome, String> {
    let state = random_b64url(16);
    let params = [
        ("loginMethod", "phone"),
        ("type", "phone"),
        ("redirect", IFLOW_REDIRECT_URI),
        ("state", state.as_str()),
        ("client_id", IFLOW_CLIENT_ID),
    ];
    let qs = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let url = format!("{IFLOW_AUTHORIZE_URL}?{qs}");
    emit(OAuthPrompt {
        url: url.clone(),
        code: None,
        message: "iFlow OAuth: open the URL, sign in, then paste the final redirect URL (or code) with /oauth-code <value>.".into(),
    });
    if !likely_headless() {
        let _ = open_browser(&url);
    }
    Ok(LoginOutcome::AwaitingCode {
        pending: PendingOauth {
            kind: "iflow".into(),
            code_verifier: String::new(),
            state,
            redirect_uri: IFLOW_REDIRECT_URI.into(),
            plugin_pending: None,
        },
    })
}

pub async fn complete_iflow_login(
    client: &reqwest::Client,
    pending: &PendingOauth,
    code: &str,
) -> Result<OAuthToken, String> {
    let code = extract_auth_code(code);
    let resp = client
        .post(IFLOW_TOKEN_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Authorization", iflow_basic_auth())
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("client_id", IFLOW_CLIENT_ID),
            ("client_secret", IFLOW_CLIENT_SECRET),
        ])
        .send()
        .await
        .map_err(|e| format!("iFlow token exchange failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "iFlow token exchange failed (HTTP {status}): {body}"
        ));
    }
    let oauth_access = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or("iFlow token response missing access_token")?
        .to_string();
    let refresh = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(String::from);
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let api_key = iflow_fetch_api_key(client, &oauth_access).await?;
    let tok = OAuthToken {
        access_token: api_key,
        refresh_token: refresh,
        expires_at: now_secs().saturating_add(expires_in),
        client_id: Some(IFLOW_CLIENT_ID.into()),
        client_secret: None,
        kind: "iflow".into(),
        id_token: None,
        extra: Some(serde_json::json!({ "oauth_access_token": oauth_access })),
    };
    store_token("iflow", &tok).ok_or("could not write iFlow credentials")?;
    Ok(tok)
}

/// Minimal form-urlencoded encoder for query params we control (no external crate).
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn open_browser(url: &str) -> std::io::Result<()> {
    let (prog, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else if cfg!(unix) {
        ("xdg-open", vec![url])
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no browser opener for this platform",
        ));
    };
    std::process::Command::new(prog)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_constants_match_codex_cli() {
        assert_eq!(OPENAI_CLIENT_ID, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(
            OPENAI_AUTHORIZE_URL,
            "https://auth.openai.com/oauth/authorize"
        );
        assert_eq!(OPENAI_TOKEN_URL, "https://auth.openai.com/oauth/token");
        assert_eq!(OPENAI_REDIRECT_PORT, 1455);
        assert_eq!(
            OPENAI_SCOPE,
            "openid profile email offline_access api.connectors.read api.connectors.invoke"
        );
    }

    #[test]
    fn google_constants_match_antigravity() {
        // Guard the exact Antigravity OAuth client against accidental drift
        // (NOT the older gemini-cli client 681255809395-...).
        assert_eq!(
            GOOGLE_CLIENT_ID,
            "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
        );
        assert_eq!(GOOGLE_CLIENT_SECRET, "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");
        // google-auth-library's OAuth2Client uses the v1 endpoint, NOT the /v2 GIS
        // endpoint (which mishandles installed-app auth-code params → "response_type missing").
        assert_eq!(
            GOOGLE_AUTHORIZE_URL,
            "https://accounts.google.com/o/oauth2/auth"
        );
        // Antigravity scopes include cclog + experimentsandconfigs beyond the
        // three gemini-cli scopes — those two unlock Antigravity-only models.
        assert_eq!(
            google_scope_string(),
            "https://www.googleapis.com/auth/cloud-platform \
             https://www.googleapis.com/auth/userinfo.email \
             https://www.googleapis.com/auth/userinfo.profile \
             https://www.googleapis.com/auth/cclog \
             https://www.googleapis.com/auth/experimentsandconfigs"
        );
        // Daily sandbox is the primary Code Assist endpoint (Antigravity).
        assert!(CODE_ASSIST_BASE_URL.contains("daily-cloudcode-pa.sandbox.googleapis.com"));
        assert!(CODE_ASSIST_PROD_BASE_URL.contains("cloudcode-pa.googleapis.com"));
        // Metadata identifies as ANTIGRAVITY (not IDE_UNSPECIFIED).
        let meta = antigravity_metadata();
        assert_eq!(meta["ideType"], "ANTIGRAVITY");
        assert_eq!(meta["pluginType"], "GEMINI");
        let headers = antigravity_headers();
        assert!(headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.starts_with("antigravity/")));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Client-Metadata" && v.contains("ANTIGRAVITY")));
    }

    #[test]
    fn xai_constants_match_hermes() {
        assert_eq!(XAI_CLIENT_ID, "b1a00492-073a-47ea-816f-4c329264a828");
        assert_eq!(XAI_DEVICE_CODE_URL, "https://auth.x.ai/oauth2/device/code");
        assert_eq!(XAI_TOKEN_URL, "https://auth.x.ai/oauth2/token");
        assert!(XAI_SCOPE.contains("offline_access"));
        assert!(XAI_SCOPE.contains("grok-cli:access"));
        assert!(XAI_SCOPE.contains("api:access"));
        assert_eq!(XAI_REFRESH_SKEW_SECS, 3600);
    }

    #[test]
    fn qwen_constants_match_9router() {
        assert_eq!(QWEN_CLIENT_ID, "f0304373b74a44d2b584a3fb70ca9e56");
        assert_eq!(
            QWEN_DEVICE_CODE_URL,
            "https://chat.qwen.ai/api/v1/oauth2/device/code"
        );
        assert_eq!(QWEN_TOKEN_URL, "https://chat.qwen.ai/api/v1/oauth2/token");
        assert_eq!(QWEN_SCOPE, "openid profile email model.completion");
        assert!(supports_login("qwen"));
    }

    #[test]
    fn nine_router_coding_oauth_constants_are_wired() {
        assert_eq!(GITHUB_CLIENT_ID, "Iv1.b507a08c87ecfe98");
        assert_eq!(
            GITHUB_DEVICE_CODE_URL,
            "https://github.com/login/device/code"
        );
        assert_eq!(
            GITHUB_TOKEN_URL,
            "https://github.com/login/oauth/access_token"
        );
        assert_eq!(
            GITHUB_COPILOT_TOKEN_URL,
            "https://api.github.com/copilot_internal/v2/token"
        );
        assert_eq!(
            KIMI_CODING_DEVICE_CODE_URL,
            "https://auth.kimi.com/api/oauth/device_authorization"
        );
        assert_eq!(
            KIMI_CODING_TOKEN_URL,
            "https://auth.kimi.com/api/oauth/token"
        );
        assert_eq!(
            KILOCODE_INITIATE_URL,
            "https://api.kilo.ai/api/device-auth/codes"
        );
        for provider in [
            "github",
            "kimi-coding",
            "kilocode",
            "cline",
            "clinepass",
            "kimchi",
            "codebuddy-cn",
            "iflow",
        ] {
            assert!(supports_login(provider), "{provider} must support /login");
        }
        assert_eq!(
            CLINE_AUTHORIZE_URL,
            "https://api.cline.bot/api/v1/auth/authorize"
        );
        assert_eq!(IFLOW_CLIENT_ID, "10009311001");
        assert_eq!(
            CODEBUDDY_STATE_URL,
            "https://copilot.tencent.com/v2/plugin/auth/state"
        );
    }

    #[test]
    fn claude_constants_match_claude_code() {
        assert_eq!(CLAUDE_CLIENT_ID, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
        assert_eq!(
            CLAUDE_AUTHORIZE_URL,
            "https://claude.com/cai/oauth/authorize"
        );
        assert_eq!(
            CLAUDE_TOKEN_URL,
            "https://platform.claude.com/v1/oauth/token"
        );
        assert_eq!(
            CLAUDE_MANUAL_REDIRECT_URL,
            "https://platform.claude.com/oauth/code/callback"
        );
        assert_eq!(
            claude_scope_string(),
            "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"
        );
    }

    #[test]
    fn claude_authorize_url_has_correct_params() {
        // Local loopback mode
        let local = claude_authorize_url("challenge", "state", "http://localhost:54321/callback");
        assert!(local.starts_with("https://claude.com/cai/oauth/authorize?"));
        // NO code=true, isManual, or port — those are internal JS options, not URL params
        assert!(!local.contains("code=true"));
        assert!(!local.contains("isManual"));
        assert!(!local.contains("port="));
        assert!(local.contains("client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e"));
        assert!(local.contains("response_type=code"));
        assert!(local.contains("redirect_uri=http%3A%2F%2Flocalhost%3A54321%2Fcallback"));
        assert!(local.contains("scope=user%3Aprofile%20user%3Ainference%20user%3Asessions%3Aclaude_code%20user%3Amcp_servers%20user%3Afile_upload"));
        assert!(local.contains("code_challenge=challenge"));
        assert!(local.contains("code_challenge_method=S256"));
        assert!(local.contains("state=state"));

        // Manual/headless mode — same params, different redirect_uri
        let manual = claude_authorize_url("challenge", "state", CLAUDE_MANUAL_REDIRECT_URL);
        assert!(manual
            .contains("redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback"));
        assert!(!manual.contains("isManual"));
        assert!(!manual.contains("port="));
    }

    #[test]
    fn pct_encode_encodes_reserved() {
        assert_eq!(pct_encode("hello world"), "hello%20world");
        assert_eq!(
            pct_encode("http://127.0.0.1:41008/callback"),
            "http%3A%2F%2F127.0.0.1%3A41008%2Fcallback"
        );
        // unreserved set passes through.
        assert_eq!(pct_encode("AZaz09-._~"), "AZaz09-._~");
    }

    #[test]
    fn random_hex_is_64_chars_for_32_bytes() {
        let s = random_hex(32);
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn gemini_creds_roundtrip_ms_expiry() {
        // expiry_date is stored in MILLISECONDS; ensure the s<->ms mapping
        // survives a write→read round-trip.
        let tmp = OAuthToken {
            access_token: "ya29.test".into(),
            refresh_token: Some("1//refresh".into()),
            expires_at: 1719000000, // seconds
            client_id: Some(GOOGLE_CLIENT_ID.into()),
            client_secret: Some(GOOGLE_CLIENT_SECRET.into()),
            kind: "google".into(),
            id_token: None,
            extra: None,
        };
        let creds = GeminiCreds {
            access_token: Some(tmp.access_token.clone()),
            refresh_token: tmp.refresh_token.clone(),
            token_type: Some("Bearer".into()),
            expiry_date: Some(tmp.expires_at * 1000),
            scope: Some(google_scope_string()),
            id_token: None,
        };
        let json = serde_json::to_string(&creds).unwrap();
        let parsed: GeminiCreds = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.expiry_date, Some(1719000000000u64));
        // ms → s on read:
        assert_eq!(parsed.expiry_date.unwrap() / 1000, tmp.expires_at);
    }

    #[test]
    fn merged_gemini_creds_preserves_refresh_token_on_relogin() {
        // Google only returns a refresh_token on the FIRST authorization for a
        // client+user. A re-login returns NO refresh_token. Without the merge,
        // write_gemini_token would clobber the stored refresh_token with None,
        // making future refreshes impossible (access token expires in ~1h →
        // silently logged out). The merge must preserve the existing one.
        let existing = GeminiCreds {
            access_token: Some("ya29.old".into()),
            refresh_token: Some("1//old-refresh".into()),
            token_type: Some("Bearer".into()),
            expiry_date: Some(1_719_000_000_000),
            scope: None,
            id_token: Some("old.jwt".into()),
        };
        // New login: fresh access token, but NO refresh_token / id_token (the
        // re-authorization case Google hits us with).
        let new_tok = OAuthToken {
            access_token: "ya29.new".into(),
            refresh_token: None,
            expires_at: 1_719_003_600,
            client_id: Some(GOOGLE_CLIENT_ID.into()),
            client_secret: Some(GOOGLE_CLIENT_SECRET.into()),
            kind: "google".into(),
            id_token: None,
            extra: None,
        };
        let merged = merged_gemini_creds(&new_tok, Some(&existing));
        assert_eq!(merged.access_token, Some("ya29.new".into()));
        // CRITICAL: the old refresh_token survives the re-login.
        assert_eq!(merged.refresh_token, Some("1//old-refresh".into()));
        // The old id_token is also preserved (interchangeability with gemini-cli).
        assert_eq!(merged.id_token, Some("old.jwt".into()));
        assert_eq!(merged.expiry_date, Some(1_719_003_600_000)); // ms
    }

    #[test]
    fn merged_gemini_creds_uses_new_refresh_when_google_returns_one() {
        // When Google DOES return a new refresh_token (first-ever login, or a
        // forced re-consent), the new one wins.
        let existing = GeminiCreds {
            access_token: Some("ya29.old".into()),
            refresh_token: Some("1//old-refresh".into()),
            token_type: Some("Bearer".into()),
            expiry_date: Some(1_719_000_000_000),
            scope: None,
            id_token: None,
        };
        let new_tok = OAuthToken {
            access_token: "ya29.new".into(),
            refresh_token: Some("1//new-refresh".into()),
            expires_at: 1_719_003_600,
            client_id: Some(GOOGLE_CLIENT_ID.into()),
            client_secret: Some(GOOGLE_CLIENT_SECRET.into()),
            kind: "google".into(),
            id_token: Some("new.jwt".into()),
            extra: None,
        };
        let merged = merged_gemini_creds(&new_tok, Some(&existing));
        assert_eq!(merged.refresh_token, Some("1//new-refresh".into()));
        assert_eq!(merged.id_token, Some("new.jwt".into()));
    }

    #[test]
    fn extract_auth_code_handles_bare_and_url() {
        // Bare code from the address bar after Google redirects.
        assert_eq!(extract_auth_code("4/0AanRRr6secret"), "4/0AanRRr6secret");
        // Antigravity loopback redirect URL with %-encoded slash + extra params.
        assert_eq!(
            extract_auth_code(
                "http://localhost:51121/oauth-callback?code=4%2F0AanRRr6secret&scope=a+b&state=xyz"
            ),
            "4/0AanRRr6secret"
        );
        // Legacy gemini-cli OOB URL still parses if pasted.
        assert_eq!(
            extract_auth_code(
                "https://codeassist.google.com/authcode?code=4%2F0AanRRr6secret&scope=a+b&state=xyz"
            ),
            "4/0AanRRr6secret"
        );
        // Leading/trailing whitespace trimmed.
        assert_eq!(extract_auth_code("  4/0AanRRr6  \n"), "4/0AanRRr6");
        // `+` decodes to space (form-encoding parity).
        assert_eq!(extract_auth_code("code=a+b&x=1"), "a b");
    }

    #[test]
    fn antigravity_manual_and_web_share_registered_redirect() {
        // Both flows must use the same registered URI — OOB codeassist is
        // gemini-cli only and causes redirect_uri_mismatch on Antigravity.
        assert_eq!(
            ANTIGRAVITY_REDIRECT_URI,
            "http://localhost:51121/oauth-callback"
        );
        assert_eq!(ANTIGRAVITY_REDIRECT_PORT, 51121);
        assert!(!ANTIGRAVITY_REDIRECT_URI.contains("codeassist.google.com"));
    }

    #[test]
    fn headless_google_login_uses_antigravity_loopback_redirect() {
        // Force headless path regardless of ambient SSH/DISPLAY.
        std::env::set_var("CATALYST_CODE_NO_BROWSER", "1");
        let captured: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        let emit = |p: OAuthPrompt| {
            *captured.lock().unwrap() = Some(p.url);
        };
        let outcome = google_login_manual(&emit).expect("manual login starts");
        std::env::remove_var("CATALYST_CODE_NO_BROWSER");
        let url = captured.lock().unwrap().clone().expect("emitted url");
        assert!(
            url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A51121%2Foauth-callback"),
            "expected Antigravity loopback redirect in authorize URL, got: {url}"
        );
        assert!(
            !url.contains("codeassist.google.com"),
            "must not use gemini-cli OOB redirect: {url}"
        );
        assert!(
            url.contains("prompt=consent"),
            "should request offline consent"
        );
        match outcome {
            LoginOutcome::AwaitingCode { pending } => {
                assert_eq!(pending.redirect_uri, ANTIGRAVITY_REDIRECT_URI);
                assert_eq!(pending.kind, "gemini");
                assert!(!pending.code_verifier.is_empty());
            }
            LoginOutcome::Done => panic!("expected AwaitingCode, got Done"),
        }
    }
}
