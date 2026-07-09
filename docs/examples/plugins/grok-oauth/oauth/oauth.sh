#!/bin/bash
#
# Example plugin OAuth script: xAI (Grok) device-code flow (RFC 8628).
#
# This is ONE script handling all four actions dispatched by the `action`
# field in the stdin context: `login`, `complete`, `token`, `clear`. The
# harness owns the /login picker, the oauth_prompt event, and the /oauth-code
# paste path; this script owns the token's on-disk format (a small JSON file
# at $token_path).
#
# Contract (stdin → one JSON object on stdout):
#   login    → { url, code?, message, flow, state?, pending? }
#   complete → { ok }            (writes the token to $token_path)
#   token    → { access_token, expires_at? } | { access_token: null }
#   clear    → { ok }
#
# Requires: curl, jq. chmod +x this file.
set -euo pipefail

# --- vendor constants (fill in for your OAuth provider) ---------------------
CLIENT_ID="${GROK_OAUTH_CLIENT_ID:-your-client-id-here}"
# xAI device-code endpoints (verify against your provider's docs):
DEVICE_CODE_URL="https://auth.x.ai/oauth/device/code"
TOKEN_URL="https://auth.x.ai/oauth/token"
SCOPES="openid profile email offline_access"

# --- read the harness context ------------------------------------------------
input="$(cat)"
action="$(printf '%s' "$input" | jq -r '.action')"
token_path="$(printf '%s' "$input" | jq -r '.token_path')"
provider_id="$(printf '%s' "$input" | jq -r '.provider_id')"

mkdir -p "$(dirname "$token_path")"

case "$action" in

  # --- login: start the device-code flow, return the verify URL -------------
  login)
    resp="$(curl -fsS -X POST "$DEVICE_CODE_URL" \
      -d "client_id=$CLIENT_ID" \
      -d "scope=$(printf '%s' "$SCOPES" | jq -sRr @uri)")"
    device_code="$(printf '%s' "$resp" | jq -r '.device_code')"
    user_code="$(printf '%s' "$resp" | jq -r '.user_code')"
    verify_uri="$(printf '%s' "$resp" | jq -r '.verification_uri // .verification_uri_complete // ""')"
    # If the provider gives a complete URL (embeds the code), prefer it;
    # otherwise build the standard "?user_code=" form.
    [ -z "$verify_uri" ] && verify_uri="$(printf '%s' "$resp" | jq -r '.verification_uri')?user_code=$user_code"
    # Stash the device_code in `pending` so `complete` can poll with it.
    jq -n --arg url "$verify_uri" --arg code "$user_code" \
          --arg dc "$device_code" \
      '{ url: $url, code: $code, flow: "manual",
         message: "Open the URL, approve the request, then run /oauth-code <code>.",
         pending: { device_code: $dc } }'
    ;;

  # --- complete: poll the token endpoint until the user approves ------------
  complete)
    code="$(printf '%s' "$input" | jq -r '.code')"
    device_code="$(printf '%s' "$input" | jq -r '.pending.device_code // .code')"
    # Poll until approved, expired, or denied (device-code grant).
    for _ in $(seq 1 60); do
      resp="$(curl -sS -X POST "$TOKEN_URL" \
        -d "grant_type=urn:ietf:params:oauth:grant-type:device_code" \
        -d "client_id=$CLIENT_ID" \
        -d "device_code=$device_code")" || true
      err="$(printf '%s' "$resp" | jq -r '.error // empty')"
      case "$err" in
        authorization_pending|slow_down) sleep 3 ;;
        expired_token) echo '{"ok":false,"error":"device code expired"}'; exit 0 ;;
        access_denied)  echo '{"ok":false,"error":"user denied the request"}'; exit 0 ;;
        "")            break ;;  # success — a token object
        *)             echo "{\"ok\":false,\"error\":\"$err\"}"; exit 0 ;;
      esac
    done
    # Persist the token (the plugin owns the format).
    printf '%s' "$resp" | jq '{ access_token, refresh_token,
        expires_at: ((now + (.expires_in // 3600)) | floor) }' > "$token_path"
    echo '{"ok":true}'
    ;;

  # --- token: return a fresh access token, refreshing if expired -----------
  token)
    if [ ! -f "$token_path" ]; then
      echo '{"access_token":null}'; exit 0
    fi
    access_token="$(jq -r '.access_token // empty' "$token_path")"
    expires_at="$(jq -r '.expires_at // 0' "$token_path")"
    now="$(date +%s)"
    # Refresh when missing or within 60s of expiry.
    if [ -z "$access_token" ] || [ "$expires_at" -le "$((now + 60))" ]; then
      refresh_token="$(jq -r '.refresh_token // empty' "$token_path")"
      if [ -z "$refresh_token" ]; then
        echo '{"access_token":null}'; exit 0
      fi
      resp="$(curl -fsS -X POST "$TOKEN_URL" \
        -d "grant_type=refresh_token" \
        -d "client_id=$CLIENT_ID" \
        -d "refresh_token=$refresh_token")" || { echo '{"access_token":null}'; exit 0; }
      # Merge the refreshed token back (preserve the refresh_token if not returned).
      jq --argjson new "$resp" \
         ' .access_token = $new.access_token
         | .expires_at = ((now + ($new.expires_in // 3600)) | floor)
         | .refresh_token = ($new.refresh_token // .refresh_token) ' "$token_path" > "$token_path.tmp" \
        && mv "$token_path.tmp" "$token_path"
      access_token="$(jq -r '.access_token' "$token_path")"
      expires_at="$(jq -r '.expires_at' "$token_path")"
    fi
    jq -n --arg t "$access_token" --argjson e "$expires_at" \
      '{ access_token: $t, expires_at: $e }'
    ;;

  # --- clear: delete the token file (the harness also deletes it) ----------
  clear)
    rm -f "$token_path"
    echo '{"ok":true}'
    ;;

  *)
    echo "{\"ok\":false,\"error\":\"unknown action: $action\"}"
    ;;
esac
