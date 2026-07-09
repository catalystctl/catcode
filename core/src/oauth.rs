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
//!    - **web flow** (local machine) — loopback
//!      `http://127.0.0.1:<random>/oauth2callback` redirect, no PKCE (matches
//!      gemini-cli's `authWithWeb`); completes synchronously.
//!    - **no-browser flow** (SSH / headless, or `CATALYST_CODE_NO_BROWSER=1`)
//!      — redirect to `https://codeassist.google.com/authcode` (Google's
//!      out-of-band page that shows the code to copy) + PKCE; the user opens the
//!      URL on ANY device and pastes the code back via `/oauth-code` (matches
//!      gemini-cli's `authWithUserCode`). Works over SSH with no port forwarding.
//!      Tokens are written to `~/.gemini/oauth_creds.json` in the
//!      `google-auth-library` `Credentials` shape, so a token obtained here is
//!      interchangeable with one obtained by `gemini`'s own `/login` (and vice
//!      versa). This works for **regular Google accounts** (personal Gmail /
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
//!
//! Tokens from `/login` are stored at `~/.gemini/oauth_creds.json` (Google,
//! 0600) or `~/.config/catalyst-code/oauth/<id>.json` (OpenAI/Claude, 0600)
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

// --- Google (Gemini) constants ------------------------------------------------
// The EXACT public OAuth client the official `gemini` CLI uses (reverse-
// engineered from `google-gemini/gemini-cli` packages/core/src/code_assist/
// oauth2.ts). It is an "installed application" whose secret is not treated as a
// secret per Google's own OAuth2 installed-app docs. Using gemini-cli's client
// (not gcloud's SDK client) makes our consent screen, scopes, and stored token
// identical to `gemini`'s — so a login here is interchangeable with one there.
const GOOGLE_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GOOGLE_CLIENT_SECRET: &str = "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";
// The authorization endpoint google-auth-library's OAuth2Client uses (v1,
// NOT the `/o/oauth2/v2/auth` GIS endpoint — GIS mishandles the installed-app
// auth-code params and Google's consent rejects with "response_type missing").
const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// The Code Assist API endpoint — the backend gemini-cli routes OAuth tokens
/// through. `generativelanguage.googleapis.com` only accepts API keys; the
/// gemini-cli OAuth token (client_id 681255809395-...) authenticates against
/// `cloudcode-pa.googleapis.com/v1internal`, which proxies Gemini requests for
/// personal Google accounts (the free tier).
const CODE_ASSIST_BASE_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal";
// The exact three scopes gemini-cli requests (space-joined in the auth URL).
// cloud-platform alone is NOT enough — userinfo.email + userinfo.profile are
// what gemini-cli requests for account identity.
const GOOGLE_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
];
fn google_scope_string() -> String {
    GOOGLE_SCOPES.join(" ")
}
// gemini-cli's post-redirect success page (the loopback server 302s here on
// success; on failure it redirects to the failure URL).
const GOOGLE_SUCCESS_URL: &str =
    "https://developers.google.com/gemini-code-assist/auth_success_gemini";
// Google's installed-app "out-of-band" redirect: a Google-hosted page that
// shows the authorization code for the user to copy. Used by the no-browser
// (manual-code) flow (gemini-cli's `authWithUserCode`) so login works over SSH
// with no loopback / port forwarding.
const GOOGLE_OOB_REDIRECT: &str = "https://codeassist.google.com/authcode";

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
            // Our login writes ~/.gemini/oauth_creds.json (shared with gemini-cli
            // — deleting it logs out of both, which is the point of /logout).
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

// --- token resolution (used at turn/discovery time by enrich_oauth) -----------

/// Resolve a Google OAuth access token. Resolution order matches "support
/// Google Cloud AND regular accounts":
///   1. `~/.gemini/oauth_creds.json` (our `/login` OR gemini-cli's `/login`) —
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

/// Onboard the user on the Code Assist platform and return their project ID.
/// Every `generateContent` request to `cloudcode-pa.googleapis.com` requires
/// a `project` field — this fetches it via `loadCodeAssist` (and falls back to
/// `onboardUser` for new users who haven't been provisioned yet).
/// The result is cached for the process lifetime (the project ID is stable).
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
    let body = json!({
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        }
    });
    let resp = client
        .post(format!("{CODE_ASSIST_BASE_URL}:loadCodeAssist"))
        .bearer_auth(token)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let v: Value = resp.json().await.ok()?;
    // cloudaicompanionProject is at the top level of the response.
    if let Some(p) = v.get("cloudaicompanionProject").and_then(|p| p.as_str()) {
        if !p.is_empty() {
            return Some(p.to_string());
        }
    }
    // New user not yet provisioned — onboard them.
    onboard_code_assist_user(client, token).await
}

async fn onboard_code_assist_user(client: &reqwest::Client, token: &str) -> Option<String> {
    let body = json!({
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        }
    });
    let resp = client
        .post(format!("{CODE_ASSIST_BASE_URL}:onboardUser"))
        .bearer_auth(token)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let v: Value = resp.json().await.ok()?;
    v.get("projectId")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
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
        ProviderKind::Anthropic => {
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
                // CRITICAL: the gemini-cli OAuth token (from client_id
                // 681255809395-...) is NOT for generativelanguage.googleapis.com
                // — that endpoint only accepts API keys (returns 401 "missing
                // authentication credential" for OAuth Bearer tokens). The OAuth
                // token is for the Code Assist API at cloudcode-pa.googleapis.com,
                // which proxies Gemini requests for personal Google accounts.
                // Route all turns through the Code Assist API when using OAuth.
                rp.base_url = CODE_ASSIST_BASE_URL.to_string();
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

/// Bind the loopback OAuth callback server. Host defaults to 127.0.0.1 (Google's
/// Desktop-app policy mandates a loopback IP literal in the redirect_uri); port
/// defaults to 0 (OS-assigned ephemeral) — matching gemini-cli, which uses a
/// RANDOM port. `OAUTH_CALLBACK_HOST` / `OAUTH_CALLBACK_PORT` override (gemini-cli
/// honors the same envs). Returns (listener, the port actually bound).
async fn bind_callback() -> Result<(tokio::net::TcpListener, u16), String> {
    let host = std::env::var("OAUTH_CALLBACK_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = std::env::var("OAUTH_CALLBACK_PORT")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| format!("could not bind {host}:{port}: {e}"))?;
    let bound_port = listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())?;
    Ok((listener, bound_port))
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

/// Wait for the OAuth redirect on a bound loopback server: accept one
/// connection, parse `?code=&state=` from the GET line, verify `state`, respond
/// (a 302 to `success_url` when given — matching gemini-cli's success page — or
/// a 200 HTML confirmation), return the code. Times out after 5 min.
async fn await_redirect(
    listener: tokio::net::TcpListener,
    state: &str,
    success_url: Option<&str>,
) -> Result<String, String> {
    await_redirect_dual(listener, None, state, success_url).await
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
    /// The redirect_uri used (e.g. `https://codeassist.google.com/authcode`).
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
///  - web flow (default, local machine): loopback redirect, no PKCE — completes
///    synchronously.
///  - no-browser flow (SSH / headless, or `CATALYST_CODE_NO_BROWSER=1`):
///    redirect to `https://codeassist.google.com/authcode` + PKCE; returns
///    `AwaitingCode` and the user pastes the code back via `oauth_code`. This
///    is gemini-cli's `authWithUserCode` path, so it works over SSH with no
///    port forwarding.
pub async fn google_login(
    client: &reqwest::Client,
    emit: &dyn Fn(OAuthPrompt),
) -> Result<LoginOutcome, String> {
    if likely_headless() {
        return google_login_manual(emit);
    }
    google_login_web(client, emit)
        .await
        .map(|_| LoginOutcome::Done)
}

/// The no-browser Google OAuth flow (gemini-cli `authWithUserCode`): redirect
/// to `https://codeassist.google.com/authcode` (Google's installed-app
/// out-of-band page that displays the code to copy) + PKCE. Emits the URL;
/// returns `AwaitingCode` with the stashed verifier. `complete_oauth` finishes
/// the exchange once the user pastes the code.
fn google_login_manual(emit: &dyn Fn(OAuthPrompt)) -> Result<LoginOutcome, String> {
    let verifier = random_b64url(96); // 128 chars (matches gemini-cli's verifier length)
    let challenge = pkce_challenge(&verifier);
    let state = random_hex(32); // 64 hex chars
    let redirect = GOOGLE_OOB_REDIRECT.to_string();
    let redirect_enc = pct_encode(&redirect);
    let scope_enc = pct_encode(&google_scope_string());
    let authorize = format!(
        "{GOOGLE_AUTHORIZE_URL}?client_id={GOOGLE_CLIENT_ID}&redirect_uri={redirect_enc}&response_type=code&scope={scope_enc}&access_type=offline&code_challenge={challenge}&code_challenge_method=S256&state={state}"
    );
    emit(OAuthPrompt {
        url: authorize.clone(),
        code: None,
        message: "No browser detected (SSH/headless). Open this URL on ANY device, sign in and approve, then paste the authorization code back here with: /oauth-code <code>".to_string(),
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
/// user pasted. Uses the stashed PKCE verifier + the `codeassist.google.com`
/// redirect_uri (matching gemini-cli's `authWithUserCode` exchange). Stores the
/// token at `~/.gemini/oauth_creds.json`.
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
        other => Err(format!("no manual OAuth completion for '{other}'")),
    }
}

/// Accept either a bare authorization code OR a full redirect URL and extract
/// the `code` query param (URL-decoding it). Users sometimes paste the whole
/// `https://codeassist.google.com/authcode?code=4%2F0Axx…&scope=…`.
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

/// The web flow: `http://127.0.0.1:<RANDOM>/oauth2callback` loopback redirect,
/// no PKCE — byte-for-byte gemini-cli's `authWithWeb`. Completes synchronously
/// (blocks until the browser redirect arrives, up to 5 min). Use the manual
/// flow (`google_login_manual`) over SSH.
async fn google_login_web(
    client: &reqwest::Client,
    emit: &dyn Fn(OAuthPrompt),
) -> Result<OAuthToken, String> {
    let (listener, port) = bind_callback().await?;
    // redirect_uri sent to Google always uses the 127.0.0.1 loopback literal
    // (even if the bind host was overridden to 0.0.0.0) per Google's policy.
    let redirect = format!("http://127.0.0.1:{port}/oauth2callback");
    let redirect_enc = pct_encode(&redirect);
    let scope_enc = pct_encode(&google_scope_string());
    // gemini-cli: crypto.randomBytes(32).toString('hex') → 64 hex chars.
    let state = random_hex(32);

    let authorize = format!(
        "{GOOGLE_AUTHORIZE_URL}?client_id={GOOGLE_CLIENT_ID}&redirect_uri={redirect_enc}&response_type=code&scope={scope_enc}&access_type=offline&state={state}"
    );

    emit(OAuthPrompt {
        url: authorize.clone(),
        code: None,
        message: "Open the URL in a browser to log in to Google (Gemini). After approving, this continues automatically.".to_string(),
    });
    let _ = open_browser(&authorize);

    let code = await_redirect(listener, &state, Some(GOOGLE_SUCCESS_URL)).await?;

    // Token exchange — gemini-cli's exact form (client_secret in the BODY, not
    // Basic; no code_verifier in the web flow).
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("client_id", GOOGLE_CLIENT_ID),
        ("client_secret", GOOGLE_CLIENT_SECRET),
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
    emit: &dyn Fn(OAuthPrompt),
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

fn claude_login_manual(emit: &dyn Fn(OAuthPrompt)) -> Result<LoginOutcome, String> {
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
    emit: &dyn Fn(OAuthPrompt),
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
    emit: &dyn Fn(OAuthPrompt),
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
    emit: &dyn Fn(OAuthPrompt),
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
    };
    write_codex_auth(&tok, id_token);
    Ok(tok)
}

/// Which presets support an interactive OAuth login flow here.
pub fn supports_login(preset_id: &str) -> bool {
    matches!(preset_id, "openai" | "gemini" | "anthropic")
}

/// Drive the interactive OAuth login for a preset. For the web flow this
/// completes synchronously and returns `LoginOutcome::Done`. For the
/// no-browser flow (Google over SSH/headless) it emits the URL and returns
/// `LoginOutcome::AwaitingCode`; the caller stashes the `pending` state and
/// the user submits the code via the `oauth_code` command, which calls
/// `complete_oauth`.
pub async fn login(
    preset_id: &str,
    client: &reqwest::Client,
    emit: &dyn Fn(OAuthPrompt),
) -> Result<LoginOutcome, String> {
    match preset_id {
        "openai" => codex_login(client, emit).await,
        "gemini" => google_login(client, emit).await,
        "anthropic" => claude_login(client, emit).await,
        other => Err(format!("'{other}' has no OAuth login flow yet")),
    }
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
    fn google_constants_match_gemini_cli() {
        // Guard the exact gemini-cli values against accidental drift.
        assert_eq!(
            GOOGLE_CLIENT_ID,
            "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com"
        );
        assert_eq!(GOOGLE_CLIENT_SECRET, "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");
        // google-auth-library's OAuth2Client uses the v1 endpoint, NOT the /v2 GIS
        // endpoint (which mishandles installed-app auth-code params → "response_type missing").
        assert_eq!(
            GOOGLE_AUTHORIZE_URL,
            "https://accounts.google.com/o/oauth2/auth"
        );
        assert_eq!(
            google_scope_string(),
            "https://www.googleapis.com/auth/cloud-platform \
             https://www.googleapis.com/auth/userinfo.email \
             https://www.googleapis.com/auth/userinfo.profile"
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
        };
        let merged = merged_gemini_creds(&new_tok, Some(&existing));
        assert_eq!(merged.refresh_token, Some("1//new-refresh".into()));
        assert_eq!(merged.id_token, Some("new.jwt".into()));
    }

    #[test]
    fn extract_auth_code_handles_bare_and_url() {
        // Bare code (codeassist.google.com shows just this).
        assert_eq!(extract_auth_code("4/0AanRRr6secret"), "4/0AanRRr6secret");
        // Full redirect URL with %-encoded slash in the code + extra params.
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
}
