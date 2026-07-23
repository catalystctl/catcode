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
      className="rounded-sm border border-ink-700 border-l-2 border-l-warning bg-ink-925 p-4"
    >
      <div className="mb-2 flex items-center gap-2">
        <span className="text-[10px] font-mono uppercase tracking-wider text-ink-400">
          Sudo command requested
        </span>
      </div>
      <p className="mb-3 text-[12px] text-ink-200">
        The agent wants to run a command that needs sudo. Enter your password to approve, or
        decline (the command will <strong>not</strong> run).
      </p>
      <div className="mb-3 rounded-sm border border-ink-800 bg-ink-950 px-2.5 py-1.5">
        <code className="break-all font-mono text-[11px] text-ink-200">{prompt.command}</code>
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
          className="w-full rounded-sm border border-ink-700 bg-ink-950 px-3 py-1.5 text-[12px] text-ink-100 placeholder:text-ink-600 transition-colors focus:border-warning/50 focus:outline-none"
        />
      </div>
      <div className="flex items-center justify-between gap-2">
        <div className="flex gap-2">
          <button
            onClick={() => password.trim() && onApprove(password)}
            disabled={!password.trim()}
            className="rounded-sm bg-accent px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
          >
            Approve
          </button>
          <button
            onClick={onDecline}
            className="rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 transition-colors hover:bg-ink-800"
          >
            Decline
          </button>
        </div>
        <span className="font-mono text-[10px] text-ink-500">auto-close in {remaining}s</span>
      </div>
    </div>
  );
}
