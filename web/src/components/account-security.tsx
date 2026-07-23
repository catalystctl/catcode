"use client";

import { useCallback, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { QRCodeSVG } from "qrcode.react";
import { authClient } from "@/lib/auth-client";
import { AppDialogHost, useAppDialog } from "./app-dialog";

// Loosely typed — Better Auth infers the exact shapes at runtime.
interface Passkey {
  id: string;
  name?: string | null;
  createdAt?: Date | string;
  deviceType?: string | null;
}

export function AccountSecurity() {
  const { data: session } = authClient.useSession();
  const router = useRouter();
  const user = session?.user as any;
  const { confirm, prompt, dialog } = useAppDialog();

  return (
    <div className="space-y-5">
      <AppDialogHost dialog={dialog} />
      <AccountInfo email={user?.email} name={user?.name} />
      <PasskeyManager confirm={confirm} prompt={prompt} />
      <TotpManager enabled={Boolean(user?.twoFactorEnabled)} prompt={prompt} />
      <ChangePassword />
      <SignOut onDone={() => router.replace("/login")} />
    </div>
  );
}

// ── Account info ──────────────────────────────────────────────
function AccountInfo({ email, name }: { email?: string | null; name?: string | null }) {
  return (
    <div className="rounded-sm border border-ink-800 bg-ink-900 px-4 py-3">
      <div className="font-mono text-[10px] uppercase tracking-wider text-ink-500">Account</div>
      <div className="mt-1 text-[13px] font-medium text-ink-100">{name || email || "—"}</div>
      {email && email !== name && (
        <div className="text-[11px] text-ink-500">{email}</div>
      )}
      <div className="mt-1.5 text-[10px] text-ink-600">
        Single-account instance — no further sign-ups are allowed.
      </div>
    </div>
  );
}

// ── Passkeys ──────────────────────────────────────────────────
function PasskeyManager({
  confirm,
  prompt,
}: {
  confirm: (opts: { title: string; message: string; confirmLabel?: string; danger?: boolean }) => Promise<boolean>;
  prompt: (opts: {
    title: string;
    message?: string;
    placeholder?: string;
    defaultValue?: string;
  }) => Promise<string | null>;
}) {
  const [passkeys, setPasskeys] = useState<Passkey[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error } = await authClient.passkey.listUserPasskeys();
    if (error) setError(error.message ?? "Failed to load passkeys.");
    else setPasskeys((data as unknown as Passkey[]) ?? []);
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function addPasskey() {
    setError(null);
    const name = await prompt({
      title: "Name this passkey",
      message: "Optional label (e.g. MacBook, YubiKey).",
      placeholder: "MacBook",
    });
    if (name === null) return;
    setLoading(true);
    const { error } = await authClient.passkey.addPasskey({ name: name || undefined });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Failed to add passkey.");
      return;
    }
    await refresh();
  }

  async function deletePasskey(id: string) {
    const ok = await confirm({
      title: "Remove passkey",
      message: "Remove this passkey? You won't be able to sign in with it.",
      confirmLabel: "Remove",
      danger: true,
    });
    if (!ok) return;
    setError(null);
    const { error } = await authClient.passkey.deletePasskey({ id });
    if (error) {
      setError(error.message ?? "Failed to delete passkey.");
      return;
    }
    await refresh();
  }

  return (
    <div>
      <SectionLabel>Passkeys</SectionLabel>
      <div className="overflow-hidden rounded-sm border border-ink-800 bg-ink-900">
        {passkeys.length === 0 && (
          <div className="px-3 py-2.5 text-[12px] text-ink-500">No passkeys registered.</div>
        )}
        {passkeys.map((pk, i) => (
          <div
            key={pk.id}
            className={`flex items-center gap-2 px-3 py-2 ${i > 0 ? "border-t border-ink-800" : ""}`}
          >
            <span className="text-[13px]">🔑</span>
            <div className="min-w-0 flex-1">
              <div className="truncate text-[12px] font-medium text-ink-100">
                {pk.name || "Unnamed passkey"}
              </div>
              {pk.createdAt && (
                <div className="text-[10px] text-ink-500">
                  {new Date(pk.createdAt).toLocaleDateString()}
                </div>
              )}
            </div>
            <button
              onClick={() => deletePasskey(pk.id)}
              className="rounded-sm px-2 py-1 text-[11px] text-ink-500 hover:bg-ink-800 hover:text-danger"
            >
              Remove
            </button>
          </div>
        ))}
      </div>
      {error && <p className="mt-1.5 text-[11px] text-danger">{error}</p>}
      <button onClick={addPasskey} disabled={loading} className="auth-btn-ghost mt-2">
        {loading ? "…" : "+ Add passkey"}
      </button>
    </div>
  );
}

// ── TOTP (authenticator app) ──────────────────────────────────
function TotpManager({
  enabled: initialEnabled,
  prompt,
}: {
  enabled: boolean;
  prompt: (opts: {
    title: string;
    message?: string;
    placeholder?: string;
    password?: boolean;
    required?: boolean;
  }) => Promise<string | null>;
}) {
  const [enabled, setEnabled] = useState(initialEnabled);
  const [setupUri, setSetupUri] = useState<string | null>(null);
  const [pendingBackupCodes, setPendingBackupCodes] = useState<string[] | null>(null);
  const [shownBackupCodes, setShownBackupCodes] = useState<string[] | null>(null);
  const [code, setCode] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Sync when the session loads the real value.
  useEffect(() => {
    setEnabled(initialEnabled);
  }, [initialEnabled]);

  // Enable flow: enable({ password }) → { totpURI, backupCodes } → show QR →
  // verifyTotp({ code }) → 2FA active → show backup codes.
  async function startEnable() {
    const password = await prompt({
      title: "Confirm password",
      message: "Enter your password to start TOTP setup.",
      password: true,
      required: true,
    });
    if (password === null) return;
    setError(null);
    setLoading(true);
    const { data, error } = await authClient.twoFactor.enable({ password });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Failed to start TOTP setup.");
      return;
    }
    const d = data as any;
    const uri = d?.totpURI ?? d?.uri;
    if (uri) {
      setSetupUri(uri);
      if (Array.isArray(d?.backupCodes)) setPendingBackupCodes(d.backupCodes);
    } else {
      setError("No TOTP URI returned.");
    }
  }

  async function verifyAndEnable(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    const { error } = await authClient.twoFactor.verifyTotp({ code });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Invalid code.");
      return;
    }
    setEnabled(true);
    setSetupUri(null);
    setCode("");
    if (pendingBackupCodes) setShownBackupCodes(pendingBackupCodes);
    setPendingBackupCodes(null);
  }

  async function disable() {
    const password = await prompt({
      title: "Disable 2FA",
      message: "Enter your password to disable two-factor authentication.",
      password: true,
      required: true,
    });
    if (password === null) return;
    setError(null);
    setLoading(true);
    const { error } = await authClient.twoFactor.disable({ password });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Failed to disable 2FA.");
      return;
    }
    setEnabled(false);
    setShownBackupCodes(null);
  }

  async function regenerateBackupCodes() {
    const password = await prompt({
      title: "Regenerate backup codes",
      message: "Enter your password to regenerate backup codes. Old codes stop working.",
      password: true,
      required: true,
    });
    if (password === null) return;
    setError(null);
    setLoading(true);
    const { data, error } = await authClient.twoFactor.generateBackupCodes({ password });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Failed to generate backup codes.");
      return;
    }
    const codes = (data as any)?.backupCodes ?? (data as any)?.codes;
    if (Array.isArray(codes)) setShownBackupCodes(codes);
  }

  // ── Backup codes display ──
  if (shownBackupCodes) {
    return (
      <div>
        <SectionLabel>Backup codes</SectionLabel>
        <div className="rounded-sm border border-ink-800 bg-ink-900 p-4">
          <p className="mb-2 text-[12px] text-ink-400">
            Save these one-time codes somewhere safe. Each can substitute for a TOTP code once.
          </p>
          <div className="grid grid-cols-2 gap-1 font-mono text-[12px] text-ink-200">
            {shownBackupCodes.map((c, i) => (
              <div key={i} className="rounded-sm bg-ink-950 px-2 py-1">{c}</div>
            ))}
          </div>
          <button onClick={() => setShownBackupCodes(null)} className="auth-btn-primary mt-3">
            I&apos;ve saved them
          </button>
        </div>
      </div>
    );
  }

  // ── Setup flow (QR + verify) ──
  if (setupUri) {
    return (
      <div>
        <SectionLabel>Two-factor (TOTP)</SectionLabel>
        <div className="rounded-sm border border-ink-800 bg-ink-900 p-4">
          <p className="mb-3 text-[12px] text-ink-400">
            Scan this QR with your authenticator app (Google Authenticator, Authy, 1Password…),
            then enter the 6-digit code it shows.
          </p>
          <div className="mb-3 flex justify-center rounded-sm bg-white p-3">
            <QRCodeSVG value={setupUri} size={160} />
          </div>
          <form onSubmit={verifyAndEnable} className="space-y-2">
            <input
              type="text"
              inputMode="numeric"
              autoFocus
              required
              value={code}
              onChange={(e) => setCode(e.target.value)}
              className="auth-input text-center text-lg tracking-[0.4em]"
              placeholder="000000"
              autoComplete="one-time-code"
            />
            {error && <p className="text-[11px] text-danger">{error}</p>}
            <button type="submit" disabled={loading} className="auth-btn-primary">
              {loading ? "Verifying…" : "Enable"}
            </button>
            <button
              type="button"
              onClick={() => {
                setSetupUri(null);
                setError(null);
              }}
              className="auth-btn-ghost"
            >
              Cancel
            </button>
          </form>
        </div>
      </div>
    );
  }

  // ── Status + actions ──
  return (
    <div>
      <SectionLabel>Two-factor (TOTP)</SectionLabel>
      <div className="rounded-sm border border-ink-800 bg-ink-900 px-4 py-3">
        <div className="flex items-center gap-2">
          <span className={`h-1.5 w-1.5 rounded-none ${enabled ? "bg-success" : "bg-ink-600"}`} />
          <span className="text-[12px] font-medium text-ink-100">
            {enabled ? "Enabled" : "Not enabled"}
          </span>
        </div>
        <p className="mt-1 text-[11px] text-ink-500">
          {enabled
            ? "An authenticator app code is required at sign-in."
            : "Use an authenticator app as a second factor."}
        </p>
        <div className="mt-2.5 flex gap-2">
          {enabled ? (
            <>
              <button onClick={regenerateBackupCodes} disabled={loading} className="auth-btn-ghost">
                New backup codes
              </button>
              <button onClick={disable} disabled={loading} className="auth-btn-ghost text-danger">
                Disable
              </button>
            </>
          ) : (
            <button onClick={startEnable} disabled={loading} className="auth-btn-primary">
              {loading ? "…" : "Enable TOTP"}
            </button>
          )}
        </div>
        {error && <p className="mt-1.5 text-[11px] text-danger">{error}</p>}
      </div>
    </div>
  );
}

// ── Change password ──────────────────────────────────────────
function ChangePassword() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setDone(false);
    if (next.length < 8) {
      setError("New password must be at least 8 characters.");
      return;
    }
    if (next !== confirm) {
      setError("Passwords do not match.");
      return;
    }
    setLoading(true);
    const { error } = await authClient.changePassword({
      currentPassword: current,
      newPassword: next,
    });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Failed to change password.");
      return;
    }
    setDone(true);
    setCurrent("");
    setNext("");
    setConfirm("");
  }

  return (
    <div>
      <SectionLabel>Change password</SectionLabel>
      <form onSubmit={onSubmit} className="space-y-2 rounded-sm border border-ink-800 bg-ink-900 p-3">
        <input
          type="password"
          required
          value={current}
          onChange={(e) => setCurrent(e.target.value)}
          className="auth-input"
          placeholder="Current password"
          autoComplete="current-password"
        />
        <input
          type="password"
          required
          value={next}
          onChange={(e) => setNext(e.target.value)}
          className="auth-input"
          placeholder="New password"
          autoComplete="new-password"
        />
        <input
          type="password"
          required
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          className="auth-input"
          placeholder="Confirm new password"
          autoComplete="new-password"
        />
        {error && <p className="text-[11px] text-danger">{error}</p>}
        {done && <p className="text-[11px] text-success">Password updated.</p>}
        <button type="submit" disabled={loading} className="auth-btn-primary">
          {loading ? "Updating…" : "Update password"}
        </button>
      </form>
    </div>
  );
}

// ── Sign out ──────────────────────────────────────────────────
function SignOut({ onDone }: { onDone: () => void }) {
  const [loading, setLoading] = useState(false);
  return (
    <button
      onClick={async () => {
        setLoading(true);
        await authClient.signOut();
        onDone();
      }}
      disabled={loading}
      className="auth-btn-ghost text-danger"
    >
      {loading ? "Signing out…" : "Sign out"}
    </button>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-2 font-mono text-[10px] uppercase tracking-wider text-ink-500">
      {children}
    </div>
  );
}
