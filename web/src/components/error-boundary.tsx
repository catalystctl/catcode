"use client";

// ErrorBoundary — catches render errors in the message list (or any subtree)
// so a single malformed message / component crash doesn't white-screen the
// whole app. Shows a compact error card with a retry button.

import { Component, Fragment, type ErrorInfo, type ReactNode } from "react";
import { WarningIcon } from "./icons";

type Props = {
  children: ReactNode;
  /** Optional label shown in the fallback (e.g. "message list"). */
  label?: string;
};

type State = { error: Error | null; resetKey: number };

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, resetKey: 0 };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[ErrorBoundary]", this.props.label ?? "", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="mx-auto my-6 max-w-xl px-4">
          <div className="rounded-xl border border-danger/30 bg-danger/[0.06] p-4">
            <div className="flex items-center gap-2 text-[13px] font-semibold text-danger">
              <WarningIcon width={14} height={14} className="shrink-0" />
              <span>Something broke rendering{this.props.label ? ` the ${this.props.label}` : ""}.</span>
            </div>
            <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-ink-950 p-2 text-[11px] text-ink-400">
              {this.state.error.message}
            </pre>
            <button
              onClick={() =>
                this.setState((s) => ({ error: null, resetKey: s.resetKey + 1 }))
              }
              className="mt-2 rounded-lg border border-ink-700 bg-ink-900 px-3 py-1.5 text-[12px] font-medium text-ink-200 transition-colors hover:bg-ink-850"
            >
              Retry
            </button>
          </div>
        </div>
      );
    }
    return <Fragment key={this.state.resetKey}>{this.props.children}</Fragment>;
  }
}
