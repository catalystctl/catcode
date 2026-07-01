"use client";

// Composer — the prompt input. Auto-growing textarea, Enter to send,
// Shift+Enter for newline. Features:
//   • Slash-command flyout: typing "/" at the start shows a filterable command
//     list (arrow keys to navigate, Enter/Tab to complete-or-run).
//   • @-mention file flyout: typing "@" triggers a debounced workspace file
//     search; selecting a file inserts its path into the prompt.
//   • Draft persistence: unsent text + images survive a reload (sessionStorage).
//   • Steer vs. send: while the agent is streaming, Enter steers (redirects).

import { useCallback, useEffect, useRef, useState } from "react";
import { SendIcon, StopIcon, BoltIcon } from "./icons";
import { ImageAttach, fileToDataUrl } from "./attach";
import { Flyout, type FlyoutItem } from "./flyout";
import { COMMANDS, filterCommands } from "@/lib/commands";
import type { FileEntry } from "@/lib/types";

interface Props {
  streaming: boolean;
  connected: boolean;
  canSend: boolean;
  thinkingLevel: string;
  modelLabel: string;
  images: string[];
  onAddImage: (url: string) => void;
  onRemoveImage: (i: number) => void;
  onPrompt: (text: string, images?: string[]) => void;
  onSteer: (text: string) => void;
  onAbort: () => void;
  onCommand: (name: string) => void;
}

const DRAFT_KEY = "umans:draft";
const DRAFT_IMG_KEY = "umans:draft-images";

function lsGet(k: string): string | null {
  try {
    return typeof sessionStorage !== "undefined" ? sessionStorage.getItem(k) : null;
  } catch {
    return null;
  }
}
function lsSet(k: string, v: string): void {
  try {
    if (typeof sessionStorage !== "undefined") sessionStorage.setItem(k, v);
  } catch {
    /* ignore */
  }
}

/** Detect a @-mention trigger at the caret: returns the @ index + query text,
 *  or null if the caret isn't inside a mention. */
function detectMention(text: string, caret: number): { start: number; query: string } | null {
  let i = caret;
  while (i > 0) {
    const ch = text[i - 1];
    if (ch === "@") {
      if (i - 1 === 0 || /\s/.test(text[i - 2])) {
        return { start: i - 1, query: text.slice(i, caret) };
      }
      return null;
    }
    if (/\s/.test(ch)) return null;
    i--;
  }
  return null;
}

/** True when the text is a single-line slash-command search (for the flyout). */
function isCommandSearch(text: string): boolean {
  return text.startsWith("/") && !text.includes("\n") && text.length <= 32;
}

export function Composer({
  streaming,
  connected,
  canSend,
  thinkingLevel,
  modelLabel,
  images,
  onAddImage,
  onRemoveImage,
  onPrompt,
  onSteer,
  onAbort,
  onCommand,
}: Props) {
  const [text, setText] = useState("");
  const [imagesState, setImagesState] = useState<string[]>(images);
  const ref = useRef<HTMLTextAreaElement>(null);

  // ── Flyout state ──
  const [cmdItems, setCmdItems] = useState<FlyoutItem[]>([]);
  const [cmdIndex, setCmdIndex] = useState(0);
  const [cmdOpen, setCmdOpen] = useState(false);

  const [fileItems, setFileItems] = useState<FlyoutItem[]>([]);
  const [fileIndex, setFileIndex] = useState(0);
  const [fileOpen, setFileOpen] = useState(false);
  const mentionRef = useRef<{ start: number; query: string } | null>(null);
  const fetchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Sync external images prop into local state (for draft persistence).
  useEffect(() => {
    setImagesState(images);
  }, [images]);

  // ── Draft persistence: restore on mount, save on change ──
  useEffect(() => {
    const saved = lsGet(DRAFT_KEY);
    if (saved) setText(saved);
    const savedImgs = lsGet(DRAFT_IMG_KEY);
    if (savedImgs) {
      try {
        const arr = JSON.parse(savedImgs);
        if (Array.isArray(arr) && arr.length) onAddImage(arr[0]); // restore first
      } catch {
        /* ignore */
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    lsSet(DRAFT_KEY, text);
  }, [text]);

  useEffect(() => {
    lsSet(DRAFT_IMG_KEY, JSON.stringify(imagesState));
  }, [imagesState]);

  // Auto-grow.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 240) + "px";
  }, [text]);

  // ── Command flyout: filter locally as the user types ──
  useEffect(() => {
    if (!isCommandSearch(text)) {
      setCmdOpen(false);
      return;
    }
    const filtered = filterCommands(text).map((c) => ({
      id: c.label,
      label: c.label,
      desc: c.desc,
    }));
    setCmdItems(filtered);
    setCmdIndex(0);
    setCmdOpen(filtered.length > 0);
  }, [text]);

  // ── File-mention flyout: debounced fetch from /api/files ──
  useEffect(() => {
    const el = ref.current;
    const caret = el ? el.selectionStart : 0;
    const mention = detectMention(text, caret);
    mentionRef.current = mention;

    if (!mention) {
      setFileOpen(false);
      return;
    }
    if (fetchTimer.current) clearTimeout(fetchTimer.current);
    fetchTimer.current = setTimeout(async () => {
      try {
        const res = await fetch(`/api/files?q=${encodeURIComponent(mention.query)}`);
        if (!res.ok) return;
        const data = (await res.json()) as { files: FileEntry[] };
        // Re-check the caret is still in a mention (user may have moved).
        const el2 = ref.current;
        const caret2 = el2 ? el2.selectionStart : 0;
        const still = detectMention(text, caret2);
        if (!still) return;
        mentionRef.current = still;
        setFileItems(
          data.files.map((f) => ({
            id: f.path,
            label: f.path,
            badge: f.dir ? "dir" : undefined,
          })),
        );
        setFileIndex(0);
        setFileOpen(data.files.length > 0);
      } catch {
        setFileOpen(false);
      }
    }, 180);
    return () => {
      if (fetchTimer.current) clearTimeout(fetchTimer.current);
    };
  }, [text]);

  const closeFlyouts = useCallback(() => {
    setCmdOpen(false);
    setFileOpen(false);
  }, []);

  const submit = useCallback(() => {
    const t = text.trim();
    if (!t && imagesState.length === 0) return;
    // Exact slash-command match → execute (legacy fast path).
    const exact = COMMANDS.find((c) => c.label === t);
    if (exact) {
      onCommand(exact.action);
      setText("");
      closeFlyouts();
      return;
    }
    const imgs = imagesState.length ? imagesState : undefined;
    if (streaming) {
      onSteer(t);
    } else {
      onPrompt(t, imgs);
    }
    setText("");
    setImagesState([]);
    closeFlyouts();
  }, [text, imagesState, streaming, onCommand, onSteer, onPrompt, closeFlyouts]);

  // Run a command from the flyout by action key.
  const runCommand = useCallback(
    (index: number) => {
      const item = cmdItems[index];
      if (!item) return;
      const def = COMMANDS.find((c) => c.label === item.id);
      if (def) {
        onCommand(def.action);
        setText("");
        closeFlyouts();
      }
    },
    [cmdItems, onCommand, closeFlyouts],
  );

  // Insert a file mention: replace @query with the file path.
  const insertFile = useCallback(
    (index: number) => {
      const item = fileItems[index];
      if (!item) return;
      const el = ref.current;
      const caret = el ? el.selectionStart : text.length;
      const mention = mentionRef.current ?? detectMention(text, caret);
      if (!mention) return;
      const before = text.slice(0, mention.start);
      const after = text.slice(caret);
      const inserted = `${before}@${item.label} ${after}`;
      setText(inserted);
      setFileOpen(false);
      // Restore caret after the inserted path + space.
      requestAnimationFrame(() => {
        const e = ref.current;
        if (!e) return;
        const pos = before.length + item.label.length + 2; // @path<space>
        e.setSelectionRange(pos, pos);
        e.focus();
      });
    },
    [fileItems, text],
  );

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    let added = false;
    for (const it of items) {
      if (it.type.startsWith("image/")) {
        const file = it.getAsFile();
        if (file) {
          added = true;
          fileToDataUrl(file).then(onAddImage).catch(() => {});
        }
      }
    }
    if (added) e.preventDefault();
  };

  const onKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // ── Flyout keyboard navigation (takes priority over submit) ──
    if (cmdOpen && cmdItems.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setCmdIndex((i) => (i + 1) % cmdItems.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setCmdIndex((i) => (i - 1 + cmdItems.length) % cmdItems.length);
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        runCommand(cmdIndex);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setCmdOpen(false);
        return;
      }
    }
    if (fileOpen && fileItems.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setFileIndex((i) => (i + 1) % fileItems.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setFileIndex((i) => (i - 1 + fileItems.length) % fileItems.length);
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        insertFile(fileIndex);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setFileOpen(false);
        return;
      }
    }
    // ── Normal submit ──
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  const disabled = !connected || !canSend;
  const flyoutOpen = cmdOpen || fileOpen;

  return (
    <div className="relative border-t border-ink-800/80 bg-ink-950/80 px-4 pb-4 pt-2 backdrop-blur sm:px-6">
      <div className="mx-auto max-w-3xl">
        <div className="relative">
          {/* Flyout (positioned above the input box) */}
          {cmdOpen && (
            <Flyout
              items={cmdItems}
              selectedIndex={cmdIndex}
              onSelect={runCommand}
              onHover={setCmdIndex}
              emptyHint="No commands match"
            />
          )}
          {fileOpen && (
            <Flyout
              items={fileItems}
              selectedIndex={fileIndex}
              onSelect={insertFile}
              onHover={setFileIndex}
              emptyHint="No files match"
            />
          )}

          <div className="flex items-end gap-2 rounded-2xl border border-ink-700/70 bg-ink-900/80 p-2 shadow-lg shadow-black/20 transition-colors focus-within:border-accent/50 focus-within:shadow-glow">
            <ImageAttach images={images} onAdd={onAddImage} onRemove={onRemoveImage} />
            <textarea
              ref={ref}
              rows={1}
              value={text}
              aria-label="Message the agent"
              disabled={!connected}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={onKey}
              onPaste={onPaste}
              onBlur={() => {
                // Delay so click on flyout registers first.
                setTimeout(() => closeFlyouts(), 150);
              }}
              placeholder={
                connected
                  ? streaming
                    ? "Redirect the agent… (Enter to steer)"
                    : "Message the agent…  (/ for commands, @ for files)"
                  : "Connecting to umans-core…"
              }
              className="max-h-60 flex-1 resize-none bg-transparent px-2 py-1.5 text-[14px] leading-relaxed text-ink-100 placeholder:text-ink-500 focus:outline-none disabled:opacity-50"
            />
            {streaming ? (
              <button
                onClick={onAbort}
                className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl border border-rose-500/40 bg-rose-500/10 px-3.5 text-[13px] font-medium text-rose-300 transition-colors hover:bg-rose-500/20"
              >
                <StopIcon width={14} height={14} /> Stop
              </button>
            ) : (
              <button
                onClick={submit}
                disabled={disabled || (!text.trim() && images.length === 0)}
                className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl bg-accent px-3.5 text-[13px] font-semibold text-white transition-all hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
              >
                <SendIcon width={14} height={14} /> Send
              </button>
            )}
          </div>
        </div>
        <div className="mt-1.5 flex items-center justify-between px-1 text-[11px] text-ink-500">
          <span className="flex items-center gap-1.5">
            <BoltIcon width={11} height={11} className="text-accent-soft" />
            <span className="font-mono">{modelLabel}</span>
            <span className="text-ink-600">·</span>
            <span>think: {thinkingLevel}</span>
            {flyoutOpen && (
              <>
                <span className="text-ink-600">·</span>
                <span className="text-accent-soft">
                  <kbd className="rounded bg-ink-800 px-1 font-mono text-[10px]">↑↓</kbd> select{" "}
                  <kbd className="ml-1 rounded bg-ink-800 px-1 font-mono text-[10px]">↵</kbd> confirm
                </span>
              </>
            )}
          </span>
          <span className="hidden sm:inline">
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Enter</kbd> send ·{" "}
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Shift+↵</kbd> newline ·{" "}
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">/</kbd>{" "}
            <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">@</kbd>
          </span>
        </div>
      </div>
    </div>
  );
}
