"use client";

import { useState, useEffect, useRef } from "react";
import type { SudoPrompt as SudoPromptData } from "@/lib/types";

interface Props {
  prompt: SudoPromptData;
  onApprove: (password: string) => void;
  onDecline: () => void;
}

/** A bash command that invokes `sudo` needs interactive approval. The user
 *  enters their sudo password (fed to `sudo -S` on stdin so sudo never touches
 *  /dev/tty) or declines (Esc / Decline button). The prompt auto-closes after 30s. */
export function SudoPrompt({ prompt, onApprove, onDecline }: Props) {
  const [password, setPassword] = useState("");
  const [remaining, setRemaining] = useState(30);
  const inputRef = useRef<HTMLInputElement>(null);
  // Keep decline stable across parent re-renders so the countdown doesn't reset.
  const onDeclineRef = useRef(onDecline);
  useEffect(() => {
    onDeclineRef.current = onDecline;
  }, [onDecline]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    if (remaining <= 0) {
      onDeclineRef.current();
      return;
    }
    const t = setTimeout(() => setRemaining((r) => r - 1), 1000);
    return () => clearTimeout(t);
  }, [remaining]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (password.trim()) onApprove(password);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onDecline();
    }
  };

  return (
    <div
      role="alertdialog"
      aria-modal="true"
      aria-label="Sudo command requested"
      className="rounded-xl border border-warning/50 bg-ink-900/95 p-4 shadow-lg backdrop-blur"
    >
      <div className="mb-2 flex items-center gap-2">
        <span className="text-[13px] font-semibold text-warning">Sudo command requested</span>
      </div>
      <p className="mb-3 text-[12px] text-ink-400">
        The agent wants to run a command that needs sudo. Enter your password to approve, or
        decline (the command will <strong>not</strong> run).
      </p>
      <div className="mb-3 rounded-lg border border-ink-700 bg-ink-950 px-3 py-2">
        <code className="break-all text-[12px] text-ink-200">{prompt.command}</code>
      </div>
      <div className="mb-3">
        <input
          ref={inputRef}
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Enter your sudo password…"
          autoComplete="off"
          className="w-full rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-warning/50 focus:outline-none"
        />
      </div>
      <div className="flex items-center justify-between gap-2">
        <div className="flex gap-2">
          <button
            onClick={() => password.trim() && onApprove(password)}
            disabled={!password.trim()}
            className="rounded-lg border border-warning/40 bg-warning/10 px-4 py-1.5 text-[13px] font-medium text-warning transition-colors hover:bg-warning/20 disabled:opacity-40"
          >
            Approve
          </button>
          <button
            onClick={onDecline}
            className="rounded-lg border border-ink-700 bg-ink-800 px-4 py-1.5 text-[13px] font-medium text-ink-300 transition-colors hover:bg-ink-700"
          >
            Decline
          </button>
        </div>
        <span className="text-[11px] text-ink-500">auto-close in {remaining}s</span>
      </div>
    </div>
  );
}
