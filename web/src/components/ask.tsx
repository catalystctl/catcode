"use client";

// AskFlyout — surfaces the `ask` tool's questions to the user as a blocking
// flyout. Each question is either a multiple-choice selection (option chips,
// optionally allowing a custom typed answer) or a free-text box. Submit sends
// the answers back to the model; Skip dismisses without answering. Without this
// the model's `ask` call would hang forever waiting for `ask_reply`.

import { useEffect, useMemo, useRef, useState } from "react";
import type { AskPrompt, AskQuestion } from "@/lib/types";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { CheckIcon, HelpIcon, SendIcon, XIcon, PencilIcon } from "./icons";

interface Props {
  prompt: AskPrompt;
  onSubmit: (answers: Record<string, string>) => void;
  onSkip: () => void;
}

export function AskFlyout({ prompt, onSubmit, onSkip }: Props) {
  // Default answers: select questions start on their first option; text start empty.
  const initial = useMemo(() => {
    const a: Record<string, string> = {};
    for (const q of prompt.questions) {
      if (q.type === "select" && q.options && q.options.length > 0) {
        a[q.id] = q.options[0];
      } else {
        a[q.id] = "";
      }
    }
    return a;
  }, [prompt]);

  const [answers, setAnswers] = useState<Record<string, string>>(initial);
  // Per select-question: whether the user switched to custom free-text entry.
  const [custom, setCustom] = useState<Record<string, boolean>>({});
  const firstTextRef = useRef<HTMLInputElement | null>(null);
  const trapRef = useFocusTrap<HTMLDivElement>();

  useEffect(() => {
    // Focus the first text-capable field so typing works immediately.
    const idx = prompt.questions.findIndex(
      (q) => q.type === "text" || q.allowCustom,
    );
    if (idx >= 0 && firstTextRef.current) {
      firstTextRef.current.focus();
    }
  }, [prompt]);

  const set = (id: string, v: string) =>
    setAnswers((prev) => ({ ...prev, [id]: v }));

  const pickOption = (q: AskQuestion, opt: string) => {
    setAnswers((prev) => ({ ...prev, [q.id]: opt }));
    setCustom((prev) => ({ ...prev, [q.id]: false }));
  };

  const toggleCustom = (q: AskQuestion) => {
    setCustom((prev) => {
      const next = { ...prev, [q.id]: !prev[q.id] };
      if (next[q.id]) {
        // entering custom mode: clear the picked option so the typed value is used
        setAnswers((a) => ({ ...a, [q.id]: "" }));
      } else if (q.options && q.options.length > 0) {
        // leaving custom mode: restore the first option
        const first = q.options[0];
        setAnswers((a) => ({ ...a, [q.id]: first }));
      }
      return next;
    });
  };

  const submit = () => {
    // Validate required questions have a non-empty answer.
    const missing = prompt.questions.filter(
      (q) => (q.required ?? true) && !(answers[q.id]?.trim()),
    );
    if (missing.length > 0) {
      const id = missing[0].id;
      // Prefer the actual input; fall back to the question container (tabIndex=-1).
      const input = document.getElementById(`ask-input-${id}`);
      const el = input ?? document.getElementById(`ask-${id}`);
      el?.focus();
      el?.scrollIntoView({ block: "center" });
      return;
    }
    // Only include non-empty answers (empty optionals are omitted → "(skipped)").
    const out: Record<string, string> = {};
    for (const q of prompt.questions) {
      const v = (answers[q.id] ?? "").trim();
      if (v) out[q.id] = v;
    }
    onSubmit(out);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onSkip();
    }
  };

  let textFocusedAssigned = false;
  const hasTextField = prompt.questions.some(
    (q) => q.type === "text" || custom[q.id],
  );

  return (
    <div
      ref={trapRef}
      className="my-3 overflow-hidden rounded-xl border border-accent/25 bg-accent/[0.03]"
      role="dialog"
      aria-modal="true"
      aria-label="Agent questions"
    >
      <div className="flex items-center gap-2 border-b border-accent/15 px-4 py-2.5">
        <HelpIcon width={15} height={15} className="shrink-0 text-accent-soft" aria-hidden />
        <span className="text-sm font-semibold text-ink-100">
          {prompt.questions.length === 1
            ? "The agent has a question"
            : `The agent has ${prompt.questions.length} questions`}
        </span>
        <button
          onClick={onSkip}
          className="ml-auto rounded-md p-1 text-ink-500 transition-colors hover:bg-ink-800 hover:text-ink-100"
          aria-label="Skip"
        >
          <XIcon width={16} height={16} />
        </button>
      </div>

      <div className="max-h-[60vh] overflow-y-auto px-4 py-3">
        <div className="space-y-4" onKeyDown={onKey}>
          {prompt.questions.map((q) => {
            const req = q.required ?? true;
            const isCustom = custom[q.id];
            const isText = q.type === "text" || isCustom;
            const inputId = `ask-input-${q.id}`;
            const assignRef =
              isText && !textFocusedAssigned ? (textFocusedAssigned = true) : false;
            return (
              <div
                key={q.id}
                id={`ask-${q.id}`}
                tabIndex={-1}
                className="space-y-1.5 outline-none"
              >
                <label
                  htmlFor={isText ? inputId : undefined}
                  className="flex items-start gap-1 text-[13px] font-medium text-ink-100"
                >
                  <span className="flex-1">{q.prompt}</span>
                  {req && <span className="text-danger">*</span>}
                  {!req && (
                    <span className="text-[10px] uppercase tracking-wide text-ink-600">
                      optional
                    </span>
                  )}
                </label>

                {q.type === "select" && q.options && (
                  <div className="flex flex-wrap gap-1.5">
                    {q.options.map((opt) => {
                      const selected = !isCustom && answers[q.id] === opt;
                      return (
                        <button
                          key={opt}
                          type="button"
                          onClick={() => pickOption(q, opt)}
                          className={`rounded-lg border px-2.5 py-1 text-[12px] font-medium transition-colors ${
                            selected
                              ? "border-accent bg-accent/15 text-accent-soft"
                              : "border-ink-700 bg-ink-950 text-ink-300 hover:border-accent/40 hover:text-ink-100"
                          }`}
                        >
                          {selected && <CheckIcon width={11} height={11} className="mr-1 inline" />}
                          {opt}
                        </button>
                      );
                    })}
                    {q.allowCustom && (
                      <button
                        type="button"
                        onClick={() => toggleCustom(q)}
                        className={`flex items-center gap-1 rounded-lg border px-2.5 py-1 text-[12px] font-medium transition-colors ${
                          isCustom
                            ? "border-accent bg-accent/15 text-accent-soft"
                            : "border-ink-700 bg-ink-950 text-ink-400 hover:border-accent/40 hover:text-ink-100"
                        }`}
                      >
                        <PencilIcon width={11} height={11} />
                        Custom
                      </button>
                    )}
                  </div>
                )}

                {isText && (
                  <input
                    id={inputId}
                    ref={assignRef ? firstTextRef : undefined}
                    type="text"
                    value={answers[q.id] ?? ""}
                    onChange={(e) => set(q.id, e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        submit();
                      } else if (e.key === "Escape") {
                        e.preventDefault();
                        onSkip();
                      }
                    }}
                    placeholder={q.placeholder ?? (isCustom ? "Type a custom answer…" : "Type your answer…")}
                    className="w-full rounded-lg border border-ink-700 bg-ink-950 px-3 py-1.5 text-[13px] text-ink-100 placeholder:text-ink-500 focus:border-accent/50 focus:outline-none focus:shadow-glow"
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 border-t border-ink-800/80 px-4 py-2.5">
        <button
          onClick={submit}
          className="flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-1.5 text-[13px] font-semibold text-white transition-colors hover:bg-accent-soft"
        >
          <SendIcon width={14} height={14} /> Submit
        </button>
        <button
          onClick={onSkip}
          className="flex items-center gap-1.5 rounded-lg border border-ink-700 px-3.5 py-1.5 text-[13px] font-medium text-ink-300 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
        >
          <XIcon width={14} height={14} /> Skip
        </button>
        <span className="ml-auto hidden text-[11px] text-ink-600 sm:inline">
          {hasTextField ? (
            <>
              <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Enter</kbd>{" "}
              submit ·{" "}
            </>
          ) : (
            <>
              <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Ctrl+↵</kbd>{" "}
              submit ·{" "}
            </>
          )}
          <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Esc</kbd>{" "}
          skip
        </span>
      </div>
    </div>
  );
}
