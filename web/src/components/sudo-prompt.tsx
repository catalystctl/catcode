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
 *  /dev/tty and garbles the TUI) or declines (Esc / Decline button). The
 *  prompt auto-closes after 30s. */
export function SudoPrompt({ prompt, onApprove, onDecline }: Props) {
  const [password, setPassword] = useState("");
  const [remaining, setRemaining] = useState(30);
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus the password field when the prompt opens.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // 30-second auto-close countdown. Each tick decrements; at 0 it declines.
  useEffect(() => {
    if (remaining <= 0) {
      onDecline();
      return;
    }
    const t = setTimeout(() => setRemaining((r) => r - 1), 1000);
    return () => clearTimeout(t);
  }, [remaining, onDecline]);

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
    <div className="rounded-xl border border-amber-500/50 bg-ink-900/95 p-4 shadow-lg backdrop-blur">
      <div className="mb-2 flex items-center gap-2">
        <span className="text-base">🔐</span>
        <span className="text-[13px] font-semibold text-amber-400">
          Sudo command requested
        </span>
      </div>
      <p className="mb-3 text-[12px] text-ink-400">
        The agent wants to run a command that needs sudo. Enter your password to
        approve, or decline (the command will <strong>not</strong> run).
      </p>
      <div className="mb-3 rounded-lg border border-ink-700 bg-ink-950 px-3 py-2">
        <code className="break-all text-[12px] text-ink-200">
          {prompt.command}
        </code>
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
          className="w-full rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-amber-500/50 focus:outline-none focus:shadow-[0_0_0_2px_rgba(245,158,11,0.15)]"
        />
      </div>
      <div className="flex items-center justify-between gap-2">
        <div className="flex gap-2">
          <button
            onClick={() => password.trim() && onApprove(password)}
            disabled={!password.trim()}
            className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-4 py-1.5 text-[13px] font-medium text-amber-400 transition-colors hover:bg-amber-500/20 disabled:opacity-40"
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
        <span className="text-[11px] text-ink-500">
          ⏱ auto-close in {remaining}s
        </span>
      </div>
    </div>
  );
}
