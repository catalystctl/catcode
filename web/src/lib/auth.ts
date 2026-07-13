import { betterAuth, APIError } from "better-auth";
import { passkey } from "@better-auth/passkey";
import { twoFactor } from "better-auth/plugins/two-factor";
import { getMigrations } from "better-auth/db/migration";
import Database from "better-sqlite3";
import { Kysely, SqliteDialect } from "kysely";
import crypto from "crypto";
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "fs";
import path from "path";
import os from "os";

// ── paths ──────────────────────────────────────────────────
const CONFIG_DIR = path.join(process.env.HOME || os.homedir(), ".config", "catalyst-code");
const DB_PATH = path.join(CONFIG_DIR, "auth.db");
const SECRET_PATH = path.join(CONFIG_DIR, "auth-secret");

// ── origin / rpID (passkey WebAuthn) ────────────────────────
// CATCODE_WEB_ORIGIN lets a user override when accessing via a tunnel/domain.
// Single-user default stays localhost; tunnels/domains must set the env var.
const ORIGIN = process.env.CATCODE_WEB_ORIGIN || "http://localhost:49283";
const RPID = new URL(ORIGIN).hostname;
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

// ── database (SQLite via better-sqlite3 + Kysely) ───────────
// better-sqlite3 requires the parent directory to already exist (it does not
// mkdir). Create it before open so Next.js "collect page data" / first boot
// on a fresh machine (and CI release builds) don't fail with
// "Cannot open database because the directory does not exist".
mkdirSync(CONFIG_DIR, { recursive: true });
const dialect = new SqliteDialect({ database: new Database(DB_PATH) });
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
  trustedOrigins: [ORIGIN],
  emailAndPassword: { enabled: true, autoSignIn: true },
  plugins: [
    passkey({ rpID: RPID, rpName: "Catalyst Code", origin: ORIGIN }),
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
