import { createAuthClient } from "better-auth/react";
import { passkeyClient } from "@better-auth/passkey/client";
import { twoFactorClient } from "better-auth/plugins/two-factor";

// The 2FA redirect fires from a fetch-plugin hook inside createAuthClient,
// so it can't directly touch component state. The login form registers a
// handler here; when signIn.email needs 2FA, the hook calls it.
let twoFactorHandler: ((methods: string[]) => void) | null = null;
export function setTwoFactorHandler(fn: ((methods: string[]) => void) | null) {
  twoFactorHandler = fn;
}

export const authClient = createAuthClient({
  plugins: [
    passkeyClient(),
    twoFactorClient({
      onTwoFactorRedirect: ({ twoFactorMethods }) => {
        twoFactorHandler?.(twoFactorMethods ?? ["totp"]);
      },
    }),
  ],
});

export const {
  signIn,
  signUp,
  signOut,
  useSession,
  changePassword,
  passkey,
  twoFactor: twoFactorActions,
} = authClient;
