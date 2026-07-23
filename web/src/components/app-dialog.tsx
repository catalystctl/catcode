"use client";

// Promise-based confirm / prompt dialogs that replace window.confirm / window.prompt.
// Each call site mounts <AppDialogHost dialog={…} /> and awaits confirm()/prompt().

import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { XIcon } from "./icons";

export type ConfirmOpts = {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
};

export type PromptOpts = {
  title: string;
  message?: string;
  placeholder?: string;
  defaultValue?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  password?: boolean;
  multiline?: boolean;
  required?: boolean;
};

type DialogState =
  | ({ kind: "confirm" } & ConfirmOpts & { resolve: (v: boolean) => void })
  | ({ kind: "prompt" } & PromptOpts & { resolve: (v: string | null) => void })
  | null;

export type AppDialogApi = {
  confirm: (opts: ConfirmOpts) => Promise<boolean>;
  prompt: (opts: PromptOpts) => Promise<string | null>;
  dialog: DialogState;
};

export function useAppDialog(): AppDialogApi {
  const [dialog, setDialog] = useState<DialogState>(null);

  const confirm = useCallback((opts: ConfirmOpts) => {
    return new Promise<boolean>((resolve) => {
      setDialog({
        kind: "confirm",
        ...opts,
        resolve: (v) => {
          setDialog(null);
          resolve(v);
        },
      });
    });
  }, []);

  const prompt = useCallback((opts: PromptOpts) => {
    return new Promise<string | null>((resolve) => {
      setDialog({
        kind: "prompt",
        ...opts,
        resolve: (v) => {
          setDialog(null);
          resolve(v);
        },
      });
    });
  }, []);

  return { confirm, prompt, dialog };
}

export function AppDialogHost({ dialog }: { dialog: DialogState }) {
  if (!dialog) return null;
  const content =
    dialog.kind === "confirm" ? (
      <ConfirmView
        title={dialog.title}
        message={dialog.message}
        confirmLabel={dialog.confirmLabel}
        cancelLabel={dialog.cancelLabel}
        danger={dialog.danger}
        onConfirm={() => {
          dialog.resolve(true);
        }}
        onCancel={() => {
          dialog.resolve(false);
        }}
      />
    ) : (
      <PromptView
        title={dialog.title}
        message={dialog.message}
        placeholder={dialog.placeholder}
        defaultValue={dialog.defaultValue}
        confirmLabel={dialog.confirmLabel}
        cancelLabel={dialog.cancelLabel}
        password={dialog.password}
        multiline={dialog.multiline}
        required={dialog.required}
        onSubmit={(v) => dialog.resolve(v)}
        onCancel={() => dialog.resolve(null)}
      />
    );
  if (typeof document === "undefined") return content;
  return createPortal(content, document.body);
}

function ConfirmView({
  title,
  message,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  danger,
  onConfirm,
  onCancel,
}: ConfirmOpts & { onConfirm: () => void; onCancel: () => void }) {
  const done = useRef(false);
  const finish = (ok: boolean) => {
    if (done.current) return;
    done.current = true;
    if (ok) onConfirm();
    else onCancel();
  };
  const closeRef = useOutsideClose(() => finish(false));
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();

  return (
    <div
      className="modal-backdrop z-[60]"
      onMouseDown={(e) => {
        e.stopPropagation();
        finish(false);
      }}
    >
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet modal-sheet-auto max-w-md"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="app-dialog-title"
        aria-describedby="app-dialog-desc"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-3 border-b border-ink-800 px-5 py-3.5">
          <h2 id="app-dialog-title" className="text-[15px] font-semibold text-ink-100">
            {title}
          </h2>
          <button
            onClick={() => finish(false)}
            className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Cancel"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>
        <p id="app-dialog-desc" className="px-5 py-4 text-[13px] leading-relaxed text-ink-300">
          {message}
        </p>
        <div className="flex justify-end gap-2 border-t border-ink-800 px-5 py-3">
          <button
            autoFocus={!!danger}
            onClick={() => finish(false)}
            className="rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 hover:bg-ink-800"
          >
            {cancelLabel}
          </button>
          <button
            autoFocus={!danger}
            onClick={() => finish(true)}
            className={`rounded-sm px-2.5 py-1 text-[11px] font-medium text-white ${
              danger
                ? "bg-danger/90 hover:bg-danger"
                : "bg-accent hover:bg-accent-soft"
            }`}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

function PromptView({
  title,
  message,
  placeholder,
  defaultValue = "",
  confirmLabel = "OK",
  cancelLabel = "Cancel",
  password,
  multiline,
  required,
  onSubmit,
  onCancel,
}: PromptOpts & { onSubmit: (v: string | null) => void; onCancel: () => void }) {
  const [value, setValue] = useState(defaultValue);
  const done = useRef(false);
  const inputRef = useRef<HTMLInputElement | HTMLTextAreaElement>(null);
  const finish = (v: string | null) => {
    if (done.current) return;
    done.current = true;
    if (v === null) onCancel();
    else onSubmit(v);
  };
  const closeRef = useOutsideClose(() => finish(null));
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select?.();
  }, []);

  const submit = () => {
    const v = value;
    if (required && !v.trim()) return;
    finish(v);
  };

  return (
    <div
      className="modal-backdrop z-[60]"
      onMouseDown={(e) => {
        e.stopPropagation();
        finish(null);
      }}
    >
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet modal-sheet-auto max-w-md"
        role="dialog"
        aria-modal="true"
        aria-labelledby="app-prompt-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-3 border-b border-ink-800 px-5 py-3.5">
          <h2 id="app-prompt-title" className="text-[15px] font-semibold text-ink-100">
            {title}
          </h2>
          <button
            onClick={() => finish(null)}
            className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-400 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Cancel"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>
        <div className="space-y-3 px-5 py-4">
          {message && <p className="text-[13px] leading-relaxed text-ink-300">{message}</p>}
          {multiline ? (
            <textarea
              ref={inputRef as React.RefObject<HTMLTextAreaElement>}
              rows={4}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder={placeholder}
              className="w-full resize-y rounded-sm border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/60 focus:outline-none"
            />
          ) : (
            <input
              ref={inputRef as React.RefObject<HTMLInputElement>}
              type={password ? "password" : "text"}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  submit();
                }
              }}
              placeholder={placeholder}
              className="w-full rounded-sm border border-ink-700 bg-ink-950 px-3 py-2 text-[13px] text-ink-100 placeholder:text-ink-600 focus:border-accent/60 focus:outline-none"
            />
          )}
        </div>
        <div className="flex justify-end gap-2 border-t border-ink-800 px-5 py-3">
          <button
            onClick={() => finish(null)}
            className="rounded-sm border border-ink-700 px-2.5 py-1 text-[11px] text-ink-300 hover:bg-ink-800"
          >
            {cancelLabel}
          </button>
          <button
            onClick={submit}
            disabled={required && !value.trim()}
            className="rounded-sm bg-accent px-2.5 py-1 text-[11px] font-medium text-white hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
