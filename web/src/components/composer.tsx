"use client";

// Composer — the prompt input. Auto-growing textarea, Enter to send,
// Shift+Enter for newline. Features:
//   • Slash-command flyout: typing "/" at the start shows a filterable command
//     list (arrow keys to navigate, Enter/Tab to complete-or-run).
//   • @-mention file flyout: typing "@" triggers a debounced workspace file
//     search; selecting a file inserts its path into the prompt.
//   • Draft persistence: unsent text + images survive a reload (sessionStorage).
//   • Mid-turn input: while streaming, Enter queues a follow-up (core one-deep
//     buffer); Ctrl+Enter steers. Esc clears a queued follow-up, else aborts.
//
// Exposes an imperative handle ({ focus, insert, openAttach }) so the shell can
// drive it from slash commands (/steer focuses, /run/parallel/chain insert a
// template, /attach opens the image picker).

import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
import { SendIcon, StopIcon, BoltIcon } from "./icons";
import { ImageAttach, fileToDataUrl, type ImageAttachHandle } from "./attach";
import { Flyout, type FlyoutItem } from "./flyout";
import { COMMANDS, filterCommands } from "@/lib/commands";
import type { FileEntry, SkillInfo } from "@/lib/types";

interface Props {
  compact?: boolean;
  streaming: boolean;
  followUpQueued?: boolean;
  /** When a HITL banner owns Esc (approve/ask/sudo/intercom), composer must not abort. */
  hitlOpen?: boolean;
  connected: boolean;
  canSend: boolean;
  thinkingLevel: string;
  modelLabel: string;
  images: string[];
  workspace: string;
  onAddImage: (url: string) => void;
  onRemoveImage: (i: number) => void;
  onPrompt: (text: string, images?: string[]) => void;
  onSteer: (text: string) => void;
  onAbort: () => void;
  onClearQueue?: () => void;
  onCommand: (name: string, args?: string) => void;
  /** Discoverable skills (drives the /skill:<name> autocomplete entries). */
  skills: SkillInfo[];
  /** Invoke a skill by name with an optional follow-up task. */
  onSkill: (name: string, task?: string) => void;
  /** PI-compatible bang bash: `!cmd` / `!!cmd`. */
  onBash?: (command: string, excludeFromContext: boolean) => void;
}

/** Imperative handle so Chat can drive the composer from slash commands. */
export interface ComposerHandle {
  /** Focus the textarea (and place the caret at the end). */
  focus: () => void;
  /** Replace the textarea value with `text` and focus it (for /run etc.). */
  insert: (text: string) => void;
  /** Open the image-attachment file picker. */
  openAttach: () => void;
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
  return text.startsWith("/") && !text.includes("\n") && text.length <= 64;
}

export const Composer = forwardRef<ComposerHandle, Props>(function Composer(
  {
    compact = false,
    streaming,
    followUpQueued = false,
    hitlOpen = false,
    connected,
    canSend,
    thinkingLevel,
    modelLabel,
    images,
    workspace,
    onAddImage,
    onRemoveImage,
    onPrompt,
    onSteer,
    onAbort,
    onClearQueue,
    onCommand,
    skills,
    onSkill,
    onBash,
  },
  ref,
) {
  const [text, setText] = useState("");
  const taRef = useRef<HTMLTextAreaElement>(null);
  const attachRef = useRef<ImageAttachHandle>(null);

  // Bumped on caret-only movement (clicks / arrow keys) so the @-mention flyout
  // re-evaluates even when `text` hasn't changed. Text changes already re-run
  // the effect via the `text` dep; this covers cursor moves within existing text.
  const [caretTick, setCaretTick] = useState(0);
  useEffect(() => {
    const handler = () => {
      const el = taRef.current;
      if (el && document.activeElement === el) setCaretTick((t) => t + 1);
    };
    document.addEventListener("selectionchange", handler);
    return () => document.removeEventListener("selectionchange", handler);
  }, []);

  // ── Flyout state ──
  const [cmdItems, setCmdItems] = useState<FlyoutItem[]>([]);
  const [cmdIndex, setCmdIndex] = useState(0);
  const [cmdOpen, setCmdOpen] = useState(false);

  const [fileItems, setFileItems] = useState<FlyoutItem[]>([]);
  const [fileIndex, setFileIndex] = useState(0);
  const [fileOpen, setFileOpen] = useState(false);
  const mentionRef = useRef<{ start: number; query: string } | null>(null);
  const fetchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Imperative handle (stable — only refs/setState are touched) ──
  useImperativeHandle(
    ref,
    () => ({
      focus: () => {
        const el = taRef.current;
        if (!el) return;
        el.focus();
        const end = el.value.length;
        el.setSelectionRange(end, end);
      },
      insert: (t: string) => {
        setText(t);
        requestAnimationFrame(() => {
          const el = taRef.current;
          if (!el) return;
          el.focus();
          const end = el.value.length;
          el.setSelectionRange(end, end);
        });
      },
      openAttach: () => attachRef.current?.pick(),
    }),
    [],
  );

  // ── Draft persistence: restore on mount, save on change ──
  // Text + images are owned by the parent (props); we restore by feeding the
  // saved values back through the parent callbacks so there is a single source
  // of truth (no divergent local image state).
  useEffect(() => {
    const saved = lsGet(DRAFT_KEY);
    if (saved) setText(saved);
    const savedImgs = lsGet(DRAFT_IMG_KEY);
    if (savedImgs) {
      try {
        const arr = JSON.parse(savedImgs);
        if (Array.isArray(arr)) {
          // Restore ALL saved images (not just the first).
          for (const url of arr) {
            if (typeof url === "string" && url) onAddImage(url);
          }
        }
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
    lsSet(DRAFT_IMG_KEY, JSON.stringify(images));
  }, [images]);

  // Auto-grow.
  useEffect(() => {
    const el = taRef.current;
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
    // Strip the leading "/" so a partial like "/frontend" matches
    // "/skill:frontend-design" (the label Contains "frontend"), mirroring
    // filterCommands' own slash-stripping for built-in commands. Without this
    // the skill filter required typing the full "/skill:<name>" prefix.
    const q = text.toLowerCase().replace(/^\//, "");
    const cmdFiltered = filterCommands(text).map((c) => ({
      id: c.label,
      label: c.label,
      desc: c.desc,
    }));
    // Append /skill:<name> entries for discoverable skills so they autocomplete
    // like the built-in commands. Match on the full "/skill:<name>" label or
    // the skill description.
    const skillFiltered = skills
      .map((s) => ({
        id: `/skill:${s.name}`,
        label: `/skill:${s.name}`,
        desc: s.description || "apply skill",
      }))
      .filter((s) => s.label.toLowerCase().includes(q) || s.desc.toLowerCase().includes(q));
    const filtered = [...cmdFiltered, ...skillFiltered];
    setCmdItems(filtered);
    setCmdIndex(0);
    setCmdOpen(filtered.length > 0);
  }, [text, skills]);

  // ── File-mention flyout: debounced fetch from /api/files ──
  useEffect(() => {
    const el = taRef.current;
    const caret = el ? el.selectionStart : 0;
    const mention = detectMention(text, caret);
    mentionRef.current = mention;

    if (!mention) {
      setFileOpen(false);
      return;
    }
    if (fetchTimer.current) clearTimeout(fetchTimer.current);
    const ac = new AbortController();
    fetchTimer.current = setTimeout(async () => {
      try {
        const res = await fetch(
          `/api/files?q=${encodeURIComponent(mention.query)}&workspace=${encodeURIComponent(workspace)}`,
          { signal: ac.signal },
        );
        if (ac.signal.aborted) return;
        if (!res.ok) return;
        const data = (await res.json()) as { files: FileEntry[] };
        if (ac.signal.aborted) return;
        // Re-check the caret is still in a mention (user may have moved).
        const el2 = taRef.current;
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
        if (ac.signal.aborted) return;
        setFileOpen(false);
      }
    }, 180);
    return () => {
      if (fetchTimer.current) clearTimeout(fetchTimer.current);
      ac.abort();
    };
  }, [text, workspace, caretTick]);

  const closeFlyouts = useCallback(() => {
    setCmdOpen(false);
    setFileOpen(false);
  }, []);

  const submit = useCallback(() => {
    const t = text.trim();
    if (!t && images.length === 0) return;
    if (!connected) return;
    // Slash commands (incl. /login) must work without a selected model.
    if (t.startsWith("/")) {
      let matched: (typeof COMMANDS)[number] | undefined;
      for (const c of COMMANDS) {
        if (t === c.label || t.startsWith(c.label + " ")) {
          if (!matched || c.label.length > matched.label.length) matched = c;
        }
      }
      if (matched) {
        const args = t.slice(matched.label.length).trim();
        onCommand(matched.action, args || undefined);
        setText("");
        closeFlyouts();
        return;
      }
    }
    // Match Send button: prompts/skills need a model.
    if (!canSend) return;
    // "/skill:<name> [task]" — invoke a discoverable skill. Handles both the
    // bare token (selected from the flyout) and a typed invocation with an
    // optional follow-up task appended after the skill name.
    if (t.startsWith("/skill:")) {
      const rest = t.slice("/skill:".length).trim();
      if (rest) {
        const sp = rest.indexOf(" ");
        const name = sp === -1 ? rest : rest.slice(0, sp);
        const task = sp === -1 ? undefined : rest.slice(sp + 1).trim();
        if (name) {
          onSkill(name, task || undefined);
          setText("");
          closeFlyouts();
          return;
        }
      }
    }
    // PI-compatible bang bash: `!cmd` (include in context) / `!!cmd` (exclude).
    if (onBash && t.startsWith("!")) {
      const exclude = t.startsWith("!!");
      const command = (exclude ? t.slice(2) : t.slice(1)).trim();
      if (command) {
        onBash(command, exclude);
        setText("");
        closeFlyouts();
        return;
      }
    }
    const imgs = images.length ? images : undefined;
    // Mid-turn Enter queues a follow-up via core; Ctrl+Enter steers (submitSteer).
    onPrompt(t, imgs);
    setText("");
    closeFlyouts();
  }, [text, images, connected, canSend, onCommand, onPrompt, onSkill, onBash, closeFlyouts]);

  const submitSteer = useCallback(() => {
    const t = text.trim();
    if (!t || !streaming) return;
    onSteer(t);
    setText("");
    closeFlyouts();
  }, [text, streaming, onSteer, closeFlyouts]);

  // Run a command from the flyout by action key.
  const runCommand = useCallback(
    (index: number) => {
      const item = cmdItems[index];
      if (!item) return;
      // /skill:<name> — insert into the composer (with a trailing space) instead
      // of dispatching immediately, so the user can append a task message and
      // send them as one turn. Press Enter again to run the bare skill with no
      // task. (The submit() path parses "/skill:<name> [task]" on Enter, so the
      // bare token and token+task both work once it's in the input.)
      if (item.id.startsWith("/skill:")) {
        const inserted = `/skill:${item.id.slice("/skill:".length)} `;
        setText(inserted);
        closeFlyouts();
        requestAnimationFrame(() => {
          const e = taRef.current;
          if (!e) return;
          e.setSelectionRange(inserted.length, inserted.length);
          e.focus();
        });
        return;
      }
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
      const el = taRef.current;
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
        const e = taRef.current;
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
    for (const it of Array.from(items)) {
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
    // ── Normal submit / mid-turn queue vs steer ──
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (streaming && (e.ctrlKey || e.metaKey)) {
        submitSteer();
      } else {
        submit();
      }
      return;
    }
    if (e.key === "Escape" && !cmdOpen && !fileOpen) {
      e.preventDefault();
      // HITL banners own Esc (deny/skip); don't also abort/clear-queue.
      if (hitlOpen) return;
      if (followUpQueued && onClearQueue) onClearQueue();
      else if (streaming) onAbort();
      return;
    }
  };

  const disabled = !connected || !canSend;
  const flyoutOpen = cmdOpen || fileOpen;

  return (
    <div className={`relative border-t border-ink-800/80 bg-ink-950/80 pb-3 pt-2 backdrop-blur ${compact ? "px-2" : "px-4 sm:px-6 sm:pb-4"}`}>
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

          <div
            className={
              "flex items-end gap-2 rounded-2xl p-2 shadow-lg shadow-black/20 transition-all duration-200 " +
              (streaming
                ? "composer-inflight"
                : "border border-ink-700/70 bg-ink-900/80 focus-within:border-accent/50 focus-within:shadow-glow")
            }
          >
            <ImageAttach ref={attachRef} images={images} onAdd={onAddImage} onRemove={onRemoveImage} />
            <textarea
              ref={taRef}
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
                    ? "Queue a follow-up… (Enter) · Ctrl+Enter to steer"
                    : "Message the agent…  (/ for commands, @ for files)"
                  : "Connecting to catcode-core…"
              }
              className="max-h-60 flex-1 resize-none bg-transparent px-2 py-1.5 text-[14px] leading-relaxed text-ink-100 placeholder:text-ink-500 focus:outline-none disabled:opacity-50"
            />
            {streaming ? (
              <>
                {text.trim() && (
                  <>
                    <button
                      onClick={submit}
                      disabled={!connected || !canSend}
                      className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl border border-accent/40 bg-accent/10 px-3.5 text-[13px] font-medium text-accent-soft transition-colors hover:bg-accent/20 disabled:opacity-40"
                      title="Queue follow-up (Enter)"
                    >
                      <SendIcon width={14} height={14} /> <span className={compact ? "hidden" : ""}>Queue</span>
                    </button>
                    <button
                      onClick={submitSteer}
                      disabled={!connected || !canSend}
                      className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl border border-warning/40 bg-warning/10 px-3.5 text-[13px] font-medium text-warning transition-colors hover:bg-warning/20 disabled:opacity-40"
                      title="Steer in-flight turn (Ctrl+Enter)"
                    >
                      {compact && <span aria-hidden="true">↗</span>}
                      <span className={compact ? "sr-only" : ""}>Steer</span>
                    </button>
                  </>
                )}
                <button
                  onClick={onAbort}
                  className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl border border-danger/40 bg-danger/10 px-3.5 text-[13px] font-medium text-danger transition-colors hover:bg-danger/20"
                >
                  <StopIcon width={14} height={14} /> <span className={compact ? "hidden" : ""}>Stop</span>
                </button>
              </>
            ) : (
              <button
                onClick={submit}
                disabled={disabled || (!text.trim() && images.length === 0)}
                className="flex h-9 shrink-0 items-center gap-1.5 rounded-xl bg-accent px-3.5 text-[13px] font-semibold text-white transition-all hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
              >
                <SendIcon width={14} height={14} /> <span className={compact ? "hidden" : ""}>Send</span>
              </button>
            )}
          </div>
        </div>
        <div className={`mt-1.5 items-center justify-between px-1 text-[11px] text-ink-500 ${compact ? "hidden" : "flex"}`}>
          <span className="flex items-center gap-1.5">
            <BoltIcon width={11} height={11} className="text-accent-soft" />
            <span className="font-mono">{modelLabel}</span>
            <span className="text-ink-600">·</span>
            <span>think: {thinkingLevel}</span>
            {followUpQueued && (
              <>
                <span className="text-ink-600">·</span>
                <span
                  className="inline-flex items-center gap-1 rounded bg-accent/15 px-1.5 py-0.5 font-medium text-accent-soft"
                  title="A follow-up is queued for after this turn"
                >
                  queued
                  {onClearQueue && (
                    <button
                      type="button"
                      onClick={onClearQueue}
                      className="ml-0.5 text-ink-400 hover:text-ink-100"
                      title="Clear queue (Esc)"
                    >
                      ×
                    </button>
                  )}
                </span>
              </>
            )}
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
            {streaming ? (
              <>
                <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Enter</kbd> queue ·{" "}
                <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Ctrl+↵</kbd> steer ·{" "}
                <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Esc</kbd>{" "}
                {followUpQueued ? "clear queue" : "stop"}
              </>
            ) : (
              <>
                <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Enter</kbd> send ·{" "}
                <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">Shift+↵</kbd> newline ·{" "}
                <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">/</kbd>{" "}
                <kbd className="rounded bg-ink-800 px-1 py-0.5 font-mono text-[10px]">@</kbd>
              </>
            )}
          </span>
        </div>
      </div>
    </div>
  );
});
