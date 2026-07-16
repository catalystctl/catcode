"use client";

// Message — renders a single transcript entry.
//   user      → right-aligned bubble
//   assistant → markdown text + collapsible reasoning + tool-call cards + metrics
//   tool      → standalone fallback card (when no matching tool call was found)

import { memo, useState, useEffect, useRef } from "react";
import type { ComponentPropsWithoutRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { AssistantMsg, BashMsg, GoalMsg, ToolMsg, UserMsg, UIMessage } from "@/lib/types";
import { formatTokens } from "@/lib/format";
import { Markdown } from "./markdown";
import { Thinking } from "./thinking";
import { ToolCallCard } from "./tool-call";
import { DotIcon, CopyIcon, CheckIcon, PencilIcon, RefreshIcon, BrandMark } from "./icons";

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
  compact,
}: {
  m: UserMsg;
  canEdit?: boolean;
  onEdit?: (text: string, images?: string[]) => void;
  compact?: boolean;
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
    <div className={`group chat-msg-enter flex justify-end py-2.5 ${compact ? "px-2" : "px-4 sm:px-6"}`}>
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
            <div className="whitespace-pre-wrap break-words rounded-2xl rounded-tr-sm border border-ink-700/40 bg-ink-850/50 px-3.5 py-2 text-[14px] leading-relaxed text-ink-100">
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
        <div
          className={
            compact
              ? "mt-1 flex justify-end gap-0.5 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100"
              : "absolute -left-9 top-1.5 flex gap-0.5 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100"
          }
        >
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
  compact,
}: {
  m: AssistantMsg;
  canRegenerate?: boolean;
  onRegenerate?: () => void;
  compact?: boolean;
}) {
  return (
    <div className={`group chat-msg-enter py-2.5 ${compact ? "px-2" : "px-4 sm:px-6"}`}>
      <div className="flex items-center gap-2 pb-1.5">
        <BrandMark size={compact ? 16 : 18} />
        <span className="text-[11px] font-medium tracking-wide text-ink-400">assistant</span>
        {m.model && <span className="font-mono text-[10px] text-ink-600">{m.model}</span>}
        {m.streaming && (
          <span className="flex items-center gap-1 text-[10px] text-accent-soft">
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
          <div className="mt-1.5 space-y-0">
            {m.toolCalls.map((tc) => (
              <ToolCallCard key={tc.id} tc={tc} />
            ))}
          </div>
        )}
        {m.usage && (
          <div className="mt-2 flex items-center gap-3 font-mono text-[10px] text-ink-600">
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
      <div className="ml-0 rounded-md border border-ink-800/50 bg-ink-925/25 px-2.5 py-1.5 sm:ml-7">
        <div className="font-mono text-[10px] uppercase tracking-wide text-ink-500">
          {m.toolName || "tool"} · {m.ok ? "ok" : "error"}
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
      <div className="ml-0 rounded-md border border-ink-800/50 bg-ink-925/25 px-2.5 py-1.5 sm:ml-7">
        <div className="font-mono text-[10px] uppercase tracking-wide text-ink-500">
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

function GoalMessage({ m }: { m: GoalMsg }) {
  const tone =
    m.ok === false || m.status === "failed"
      ? "border-danger/40 text-danger"
      : m.status === "skipped"
        ? "border-ink-700 text-ink-400"
        : m.kind === "phase"
          ? "border-accent/30 text-accent-soft"
          : "border-success/40 text-success";
  const kindLabel =
    m.kind === "step_complete"
      ? "step"
      : m.kind === "completion_summary"
        ? "summary"
        : m.kind === "verdict"
          ? "verdict"
          : "phase";
  return (
    <div className="px-4 py-1.5 sm:px-6">
      <div className={`ml-0 rounded-md border bg-ink-925/30 px-2.5 py-1.5 sm:ml-7 ${tone}`}>
        <div className="flex items-center gap-2 font-mono text-[10px] uppercase tracking-wide">
          <span>goal · {kindLabel}</span>
          {m.status && <span className="text-ink-500">{m.status}</span>}
        </div>
        <div className="mt-0.5 text-[13px] font-medium text-ink-100">{m.title}</div>
        {m.text && (
          <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words text-[12px] leading-relaxed text-ink-300">
            {m.text}
          </pre>
        )}
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
  compact,
}: {
  m: UIMessage;
  onEditUser?: (text: string, images?: string[]) => void;
  onRegenerate?: () => void;
  canEdit?: boolean;
  canRegenerate?: boolean;
  compact?: boolean;
}) {
  if (m.role === "user")
    return <UserMessage m={m} canEdit={canEdit} onEdit={onEditUser} compact={compact} />;
  if (m.role === "assistant")
    return (
      <AssistantMessage
        m={m}
        canRegenerate={canRegenerate}
        onRegenerate={onRegenerate}
        compact={compact}
      />
    );
  if (m.role === "bash") return <BashMessage m={m} />;
  if (m.role === "goal") return <GoalMessage m={m} />;
  return <ToolMessage m={m} />;
});
