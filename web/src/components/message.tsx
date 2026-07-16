"use client";

// Message — renders a single transcript entry.
//   user      → right-aligned bubble
//   assistant → markdown text + collapsible reasoning + tool-call cards + metrics
//   tool      → standalone fallback card (when no matching tool call was found)

import { memo, useState, useEffect, useRef } from "react";
import type { ComponentPropsWithoutRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { AssistantMsg, BashMsg, ToolMsg, UserMsg, UIMessage } from "@/lib/types";
import { formatTokens } from "@/lib/format";
import { Markdown } from "./markdown";
import { Thinking } from "./thinking";
import { ToolCallCard } from "./tool-call";
import { DotIcon, CopyIcon, CheckIcon, PencilIcon, RefreshIcon } from "./icons";

// Lightweight streaming markdown: while the assistant is still producing tokens
// we render markdown WITHOUT rehype-highlight (re-highlighting every code block
// on each delta is expensive). The full Markdown (with syntax highlighting) is
// used once the message is done streaming.
function StreamingMarkdown({ children }: { children: string }) {
  return (
    <div className="prose-catalyst stream-caret">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={streamComponents}>
        {children}
      </ReactMarkdown>
    </div>
  );
}

const streamComponents: ComponentPropsWithoutRef<typeof ReactMarkdown>["components"] = {
  pre({ children }) {
    return <pre className="!my-2 !border !border-ink-800 !rounded-lg !bg-ink-950">{children}</pre>;
  },
};

function CopyBtn({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    navigator.clipboard?.writeText(text).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1400);
      },
      () => {},
    );
  };
  return (
    <button
      onClick={copy}
      className="flex items-center gap-1 rounded-md px-1.5 py-1 text-[11px] text-ink-500 transition-colors hover:bg-ink-800 hover:text-ink-100"
      aria-label="Copy message"
    >
      {copied ? <CheckIcon width={12} height={12} /> : <CopyIcon width={12} height={12} />}
    </button>
  );
}

function UserMessage({
  m,
  canEdit,
  onEdit,
}: {
  m: UserMsg;
  canEdit?: boolean;
  onEdit?: (text: string, images?: string[]) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(m.text);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const cancelledRef = useRef(false);
  useEffect(() => {
    if (editing) {
      setDraft(m.text);
      cancelledRef.current = false;
      requestAnimationFrame(() => {
        const el = inputRef.current;
        if (el) {
          el.focus();
          el.setSelectionRange(el.value.length, el.value.length);
        }
      });
    }
  }, [editing, m.text]);

  const commit = () => {
    const t = draft.trim();
    if (t && t !== m.text && onEdit) onEdit(t, m.images);
    setEditing(false);
  };

  return (
    <div className="group flex justify-end px-4 py-2 sm:px-6">
      <div className="relative max-w-[min(85%,28rem)] sm:max-w-[85%]">
        {editing ? (
          <div className="rounded-2xl rounded-tr-sm border border-accent/40 bg-ink-800/70 p-2">
            <textarea
              ref={inputRef}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  commit();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  cancelledRef.current = true;
                  setEditing(false);
                }
              }}
              onBlur={() => {
                if (cancelledRef.current) {
                  cancelledRef.current = false;
                  return;
                }
                commit();
              }}
              rows={Math.min(8, draft.split("\n").length + 1)}
              className="w-full resize-none bg-transparent px-2 py-1 text-[14px] leading-relaxed text-ink-100 focus:outline-none"
            />
            <div className="flex justify-end gap-1.5 px-2 pb-1 text-[10px] text-ink-500">
              <kbd className="rounded bg-ink-900 px-1 font-mono">↵</kbd> save{" "}
              <kbd className="rounded bg-ink-900 px-1 font-mono">Esc</kbd> cancel
            </div>
          </div>
        ) : (
          <>
            <div className="whitespace-pre-wrap break-words rounded-2xl rounded-tr-sm border border-ink-700/60 bg-ink-800/70 px-4 py-2.5 text-[14px] leading-relaxed text-ink-100">
              {m.images && m.images.length > 0 && (
                <div className="mb-2 flex flex-wrap gap-1.5">
                  {m.images.map((src, i) => (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      key={i}
                      src={src}
                      alt={`attachment ${i + 1}`}
                      className="h-16 w-16 rounded-md border border-ink-700/80 object-cover"
                    />
                  ))}
                </div>
              )}
              {m.text}
            </div>
            {m.steer && (
              <span className="absolute -top-2 right-2 rounded-full border border-warning/40 bg-ink-950 px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wide text-warning">
                ↳ steer
              </span>
            )}
          </>
        )}
        <div className="absolute -left-9 top-1.5 flex gap-0.5 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100">
          <CopyBtn text={m.text} />
          {canEdit && onEdit && !editing && (
            <button
              onClick={() => setEditing(true)}
              className="flex items-center rounded-md px-1.5 py-1 text-[11px] text-ink-500 transition-colors hover:bg-ink-800 hover:text-ink-100"
              aria-label="Edit message"
              title="Edit & resend"
            >
              <PencilIcon width={12} height={12} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function AssistantMessage({
  m,
  canRegenerate,
  onRegenerate,
}: {
  m: AssistantMsg;
  canRegenerate?: boolean;
  onRegenerate?: () => void;
}) {
  return (
    <div className="group px-4 py-2 sm:px-6">
      <div className="flex items-center gap-2 pb-1">
        <span className="flex h-5 w-5 items-center justify-center rounded-md bg-accent/15 text-[11px] font-semibold text-accent-soft">
          u
        </span>
        <span className="text-[12px] font-medium text-ink-300">assistant</span>
        {m.model && <span className="font-mono text-[11px] text-ink-500">{m.model}</span>}
        {m.streaming && (
          <span className="flex items-center gap-1 text-[11px] text-accent-soft">
            <DotIcon className="animate-pulse" /> streaming
          </span>
        )}
        <span className="ml-auto flex items-center gap-0.5 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100">
          {canRegenerate && onRegenerate && !m.streaming && (
            <button
              onClick={onRegenerate}
              className="flex items-center gap-1 rounded-md px-1.5 py-1 text-[11px] text-ink-500 transition-colors hover:bg-ink-800 hover:text-ink-100"
              aria-label="Regenerate response"
              title="Regenerate"
            >
              <RefreshIcon width={12} height={12} />
            </button>
          )}
          <CopyBtn text={m.text} />
        </span>
      </div>
      <div className="pl-0 sm:pl-7">
        <Thinking text={m.thinking} active={m.streaming} />
        {m.text ? (
          m.streaming ? (
            <StreamingMarkdown>{m.text}</StreamingMarkdown>
          ) : (
            <Markdown>{m.text}</Markdown>
          )
        ) : (
          !m.thinking &&
          m.toolCalls.length === 0 &&
          m.streaming && (
            <div className="flex items-center gap-1.5 py-1 text-[13px] text-ink-500">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent-soft" />
              <span>thinking</span>
            </div>
          )
        )}
        {m.toolCalls.length > 0 && (
          <div className="mt-1 space-y-0.5">
            {m.toolCalls.map((tc) => (
              <ToolCallCard key={tc.id} tc={tc} />
            ))}
          </div>
        )}
        {m.usage && (
          <div className="mt-1.5 flex items-center gap-3 font-mono text-[10px] text-ink-500">
            {m.usage.prompt_tokens != null && <span>↑ {formatTokens(m.usage.prompt_tokens)}</span>}
            {m.usage.tokens_out != null && <span>↓ {formatTokens(m.usage.tokens_out)}</span>}
            {m.usage.cached_tokens ? <span>cached {formatTokens(m.usage.cached_tokens)}</span> : null}
          </div>
        )}
      </div>
    </div>
  );
}

function ToolMessage({ m }: { m: ToolMsg }) {
  return (
    <div className="px-4 py-1 sm:px-6">
      <div className="ml-0 rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2 sm:ml-7">
        <div className="font-mono text-[11px] text-ink-400">
          {m.toolName || "tool"} result · {m.ok ? "ok" : "error"}
        </div>
        <pre
          className={`mt-1 max-h-60 overflow-auto whitespace-pre-wrap break-words text-[12px] ${
            m.ok ? "text-ink-200" : "text-danger"
          }`}
        >
          {m.output}
        </pre>
      </div>
    </div>
  );
}

function BashMessage({ m }: { m: BashMsg }) {
  const prefix = m.excludeFromContext ? "!!" : "!";
  return (
    <div className="px-4 py-1 sm:px-6">
      <div className="ml-0 rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2 sm:ml-7">
        <div className="font-mono text-[11px] text-ink-400">
          {prefix} bash · {m.ok ? "ok" : "error"}
          {m.excludeFromContext ? " · no context" : ""}
        </div>
        <div className="mt-0.5 font-mono text-[12px] text-accent-soft">{m.command}</div>
        <pre
          className={`mt-1 max-h-60 overflow-auto whitespace-pre-wrap break-words text-[12px] ${
            m.ok ? "text-ink-200" : "text-danger"
          }`}
        >
          {m.output}
        </pre>
      </div>
    </div>
  );
}

export const Message = memo(function Message({
  m,
  onEditUser,
  onRegenerate,
  canEdit,
  canRegenerate,
}: {
  m: UIMessage;
  onEditUser?: (text: string, images?: string[]) => void;
  onRegenerate?: () => void;
  canEdit?: boolean;
  canRegenerate?: boolean;
}) {
  if (m.role === "user")
    return <UserMessage m={m} canEdit={canEdit} onEdit={onEditUser} />;
  if (m.role === "assistant")
    return (
      <AssistantMessage
        m={m}
        canRegenerate={canRegenerate}
        onRegenerate={onRegenerate}
      />
    );
  if (m.role === "bash") return <BashMessage m={m} />;
  return <ToolMessage m={m} />;
});
