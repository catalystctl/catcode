"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { authClient } from "@/lib/auth-client";
import { AuthScreen } from "@/components/auth-screen";

export function SetupForm() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [name, setName] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (password.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    if (password !== confirm) {
      setError("Passwords do not match.");
      return;
    }
    setLoading(true);
    try {
      const { error } = await authClient.signUp.email({
        email,
        password,
        name: name || email.split("@")[0],
      });
      if (error) {
        setError(error.message ?? "Failed to create account.");
        return;
      }
      router.replace("/");
      router.refresh();
    } finally {
      setLoading(false);
    }
  }

  return (
    <AuthScreen
      title="Create your account"
      subtitle="This is the only account this instance will ever have."
      footer="Catalyst Code · self-hosted"
    >
      <form onSubmit={onSubmit} className="space-y-4">
        <Field label="Display name (optional)">
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="auth-input"
            placeholder="admin"
            autoComplete="name"
          />
        </Field>
        <Field label="Email">
          <input
            type="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="auth-input"
            placeholder="you@example.com"
            autoComplete="email"
          />
        </Field>
        <Field label="Password">
          <input
            type="password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="auth-input"
            placeholder="••••••••"
            autoComplete="new-password"
          />
        </Field>
        <Field label="Confirm password">
          <input
            type="password"
            required
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            className="auth-input"
            placeholder="••••••••"
            autoComplete="new-password"
          />
        </Field>
        {error && <p className="text-sm text-danger">{error}</p>}
        <button
          type="submit"
          disabled={loading}
          className="auth-btn-primary"
        >
          {loading ? "Creating…" : "Create account"}
        </button>
      </form>
    </AuthScreen>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-xs font-medium text-ink-400">{label}</span>
      {children}
    </label>
  );
}
