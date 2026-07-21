import { betterAuth, APIError } from "better-auth";
import { passkey } from "@better-auth/passkey";
import { twoFactor } from "better-auth/plugins/two-factor";
import { getMigrations } from "better-auth/db/migration";
import { DatabaseSync } from "node:sqlite";
import { NodeSqliteDialect } from "@better-auth/kysely-adapter/node-sqlite-dialect";
import { Kysely } from "kysely";
import crypto from "crypto";
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "fs";
import path from "path";
import os from "os";

// ── paths ──────────────────────────────────────────────────
const CONFIG_DIR = path.join(process.env.HOME || os.homedir(), ".config", "catalyst-code");
const DB_PATH = path.join(CONFIG_DIR, "auth.db");
const SECRET_PATH = path.join(CONFIG_DIR, "auth-secret");

// ── origins ────────────────────────────────────────────────
// The app is served on two loopback ports: the systemd service (49283) and
// local dev (3000). Better Auth's CSRF check rejects authenticated POSTs
// (sign-out, 2FA verify, passkey add, change-password …) whose Origin header
// isn't in trustedOrigins, so BOTH ports must be listed — for localhost AND
// 127.0.0.1 (the service binds to 127.0.0.1; either address may be in the
// URL bar). Set CATCODE_WEB_ORIGIN to add a tunnel / custom domain.
// For MULTIPLE origins (e.g. behind a proxy serving several domains, or a
// domain + a direct LAN IP), set BETTER_AUTH_TRUSTED_ORIGINS (comma-separated);
// better-auth unions it with the list below (helpers.mjs getTrustedOrigins),
// so no code change is needed here. The installer exposes this as
// --trusted-origins. Passkey rpID stays bound to the single CATCODE_WEB_ORIGIN
// hostname (a WebAuthn limitation); CSRF/cookie auth works for every trusted origin.
const SYSTEMD_ORIGIN = "http://localhost:49283";
const DEV_ORIGIN = "http://localhost:3000";
const ORIGIN = process.env.CATCODE_WEB_ORIGIN || SYSTEMD_ORIGIN;
const RPID = new URL(ORIGIN).hostname; // "localhost" — shared by both ports
const TRUSTED_ORIGINS = Array.from(
  new Set([
    ORIGIN,
    SYSTEMD_ORIGIN,
    DEV_ORIGIN,
    "http://127.0.0.1:49283",
    "http://127.0.0.1:3000",
  ]),
);
if (
  process.env.NODE_ENV === "production" &&
  !process.env.CATCODE_WEB_ORIGIN &&
  process.env.NEXT_PHASE !== "phase-production-build" &&
  !(globalThis as { __catcodeOriginWarned?: boolean }).__catcodeOriginWarned
) {
  (globalThis as { __catcodeOriginWarned?: boolean }).__catcodeOriginWarned = true;
  console.warn(
    "[auth] CATCODE_WEB_ORIGIN is unset; defaulting to http://localhost:49283. " +
      "Set CATCODE_WEB_ORIGIN when accessing via a tunnel or custom domain (passkeys / cookies).",
  );
}

// ── secret (auto-generate + persist on first run) ──────────
function getSecret(): string {
  try {
    const s = readFileSync(SECRET_PATH, "utf8").trim();
    if (s.length >= 32) return s;
  } catch {
    /* not yet generated */
  }
  const s = crypto.randomBytes(32).toString("hex");
  mkdirSync(CONFIG_DIR, { recursive: true });
  writeFileSync(SECRET_PATH, s, { mode: 0o600 });
  return s;
}

// ── database (SQLite via node:sqlite + Kysely) ────────────
// node:sqlite requires the parent directory to already exist (it does not
// mkdir). Create it before open so Next.js "collect page data" / first boot
// on a fresh machine (and CI release builds) don't fail with
// "Cannot open database because the directory does not exist".
mkdirSync(CONFIG_DIR, { recursive: true });
const dialect = new NodeSqliteDialect({ database: new DatabaseSync(DB_PATH) });
const db = new Kysely<any>({ dialect });

// ── single-account enforcement ──────────────────────────────
// Only ONE user may ever exist. The setup screen creates it; every later
// attempt to create a user (signUp, plugin, direct API) is rejected.
async function rejectIfAccountExists() {
  const existing = await db.selectFrom("user").select("id").limit(1).execute();
  if (existing.length > 0) {
    throw new APIError("FORBIDDEN", {
      message: "An account already exists. Only one account is allowed.",
    });
  }
}

export const auth = betterAuth({
  database: { db, type: "sqlite" as const },
  secret: getSecret(),
  baseURL: ORIGIN,
  trustedOrigins: TRUSTED_ORIGINS,
  emailAndPassword: { enabled: true, autoSignIn: true },
  plugins: [
    // `origin` is intentionally omitted: the passkey plugin derives the
    // WebAuthn expectedOrigin from each request's Origin header
    // (options?.origin || ctx.headers.get("origin")), so a passkey registered
    // on port 3000 validates on 49283 and vice-versa — rpID "localhost" is
    // shared by both ports. (The Origin header is already CSRF-validated by
    // better-auth's origin-check middleware before this handler runs.)
    passkey({ rpID: RPID, rpName: "Catalyst Code" }),
    twoFactor({ issuer: "Catalyst Code", skipVerificationOnEnable: false }),
  ],
  databaseHooks: {
    user: {
      create: {
        before: async (user: any) => {
          await rejectIfAccountExists();
          return user;
        },
      },
    },
  },
  advanced: {
    // localhost over HTTP — cookies don't need Secure, and SameSite=Lax lets
    // the SSE EventSource (same-origin GET) carry the cookie.
    cookies: {},
  },
  rateLimit: { enabled: true },
});

// ── auto-migrate (zero manual steps) ────────────────────────
// Creates Better Auth's tables (user, session, account, verification, passkey,
// twoFactor) on first run. Idempotent — only creates what's missing.
const ready: Promise<void> = (async () => {
  try {
    const { toBeCreated, toBeAdded, runMigrations } = await getMigrations(
      auth.options as any,
    );
    if (toBeCreated.length > 0 || toBeAdded.length > 0) {
      await runMigrations();
    }
  } catch (e: any) {
    // "table already exists" is expected when the DB persists across builds
    // (the build-time static-gen import re-runs this). Tables are present, so
    // it's safe to ignore; surface anything else.
    const msg = String(e?.message ?? e);
    if (!/already exists/i.test(msg)) console.error("[auth] migration failed:", e);
  }
})();

/** Await before any auth operation to guarantee tables exist. */
export function ensureAuthReady(): Promise<void> {
  return ready;
}

/** Does the single account exist yet? (drives setup vs login routing.) */
export async function accountExists(): Promise<boolean> {
  await ensureAuthReady();
  try {
    const rows = await db.selectFrom("user").select("id").limit(1).execute();
    return rows.length > 0;
  } catch {
    return false;
  }
}

/** Validate the session cookie from request headers. Returns null if unauthed. */
export async function getSession(headers: Headers) {
  await ensureAuthReady();
  return auth.api.getSession({ headers });
}

export { db };
