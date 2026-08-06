---
name: reset-webui-password
description: Reset the Catalyst Code web UI (Better Auth single-account) password directly in ~/.config/catalyst-code/auth.db and restart the web service. Use when the user asks to reset/change their webui login password.
---

# Reset WebUI password

When to use: user asks to reset or change their Catalyst Code web UI password. There is no forgot-password flow in the app, so reset directly against the SQLite DB.

## Steps

1. Confirm the install: DB at `~/.config/catalyst-code/auth.db`, account in table `user` (`sqlite3 ... "select email from user;"`), service `catalyst-code-web.service` (user or system scope — check both).
2. Generate a Better Auth scrypt hash from the web app's own node_modules (guarantees same algorithm/params):
   ```sh
   cd web && node -e "const {hashPassword}=require('better-auth/crypto'); hashPassword('<NEWPW>').then(h=>console.log(h))"
   ```
   Hash format: `<salt_hex>:<scrypt_hex>`.
3. Update the credential account and invalidate all sessions:
   ```sql
   UPDATE account SET password='<HASH>' WHERE providerId='credential';
   DELETE FROM session;
   ```
4. Restart the service: `systemctl --user restart catalyst-code-web.service` (fall back to `sudo systemctl restart catalyst-code-web.service`).
5. Verify end-to-end with a real sign-in:
   ```sh
   curl -s -X POST http://127.0.0.1:49283/api/auth/sign-in/email \
     -H 'Content-Type: application/json' -H 'Origin: http://127.0.0.1:49283' \
     -d '{"email":"<EMAIL>","password":"<NEWPW>"}'
   ```
   Success returns JSON with a `token` and the `user` object.

## Gotchas

- Do NOT hash with a generic scrypt CLI — use `better-auth/crypto` from `web/node_modules` so params match.
- The password row lives in `account` (providerId='credential'), not `user`.
- `DELETE FROM session` logs out every active session; mention this to the user.
- The systemd unit may be `--user` scoped; check `systemctl --user` first.
