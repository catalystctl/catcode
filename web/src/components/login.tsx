"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { authClient, setTwoFactorHandler } from "@/lib/auth-client";
import { AuthScreen } from "@/components/auth-screen";

type Step = "credentials" | "two-factor";

export function LoginForm() {
  const router = useRouter();
  const [step, setStep] = useState<Step>("credentials");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Register the 2FA handler so signIn.email can flip us to the TOTP step.
  useEffect(() => {
    setTwoFactorHandler(() => {
      setStep("two-factor");
      setError(null);
      setCode("");
    });
    return () => setTwoFactorHandler(null);
  }, []);

  async function onPassword(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    const { error } = await authClient.signIn.email({ email, password });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Sign in failed.");
      return;
    }
    // If 2FA is required, the onTwoFactorRedirect handler flips the step.
    // Otherwise we're signed in.
    router.replace("/");
    router.refresh();
  }

  async function onPasskey() {
    setError(null);
    setLoading(true);
    const { error } = await authClient.signIn.passkey();
    setLoading(false);
    if (error) {
      setError(error.message ?? "Passkey sign-in failed.");
      return;
    }
    router.replace("/");
    router.refresh();
  }

  async function onVerifyTotp(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    const { error } = await authClient.twoFactor.verifyTotp({ code });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Invalid code.");
      return;
    }
    router.replace("/");
    router.refresh();
  }

  async function onUseBackupCode() {
    const bc = window.prompt("Enter a backup code:");
    if (!bc) return;
    setError(null);
    setLoading(true);
    const { error } = await authClient.twoFactor.verifyBackupCode({ code: bc });
    setLoading(false);
    if (error) {
      setError(error.message ?? "Invalid backup code.");
      return;
    }
    router.replace("/");
    router.refresh();
  }

  if (step === "two-factor") {
    return (
      <AuthScreen title="Two-factor verification" subtitle="Enter the code from your authenticator app.">
        <form onSubmit={onVerifyTotp} className="space-y-4">
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
          {error && <p className="text-sm text-danger">{error}</p>}
          <button type="submit" disabled={loading} className="auth-btn-primary">
            {loading ? "Verifying…" : "Verify"}
          </button>
          <button
            type="button"
            onClick={onUseBackupCode}
            className="auth-btn-ghost"
          >
            Use a backup code
          </button>
          <button
            type="button"
            onClick={() => {
              setStep("credentials");
              setError(null);
            }}
            className="auth-btn-ghost"
          >
            ← Back
          </button>
        </form>
      </AuthScreen>
    );
  }

  return (
    <AuthScreen title="Welcome back" subtitle="Sign in to your account.">
      <form onSubmit={onPassword} className="space-y-4">
        <label className="block">
          <span className="mb-1.5 block text-xs font-medium text-ink-400">Email</span>
          <input
            type="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="auth-input"
            placeholder="you@example.com"
            autoComplete="email"
          />
        </label>
        <label className="block">
          <span className="mb-1.5 block text-xs font-medium text-ink-400">Password</span>
          <input
            type="password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="auth-input"
            placeholder="••••••••"
            autoComplete="current-password"
          />
        </label>
        {error && <p className="text-sm text-danger">{error}</p>}
        <button type="submit" disabled={loading} className="auth-btn-primary">
          {loading ? "Signing in…" : "Sign in"}
        </button>
      </form>
      <div className="my-4 flex items-center gap-3 text-xs text-ink-600">
        <div className="h-px flex-1 bg-ink-800" />
        OR
        <div className="h-px flex-1 bg-ink-800" />
      </div>
      <button onClick={onPasskey} disabled={loading} className="auth-btn-ghost w-full">
        🔑 Sign in with passkey
      </button>
    </AuthScreen>
  );
}
