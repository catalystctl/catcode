"use client";

// Message — renders a single transcript entry.
//   user      → right-aligned bubble
//   assistant → markdown text + collapsible reasoning + tool-call cards + metrics
//   tool      → standalone fallback card (when no matching tool call was found)

import { memo, useState } from "react";
import type { ComponentPropsWithoutRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { AssistantMsg, ToolMsg, UserMsg } from "@/lib/types";
import { formatTokens } from "@/lib/format";
import { Markdown } from "./markdown";
import { Thinking } from "./thinking";
import { ToolCallCard } from "./tool-call";
import { DotIcon, CopyIcon, CheckIcon } from "./icons";

// Lightweight streaming markdown: while the assistant is still producing tokens
// we render markdown WITHOUT rehype-highlight (re-highlighting every code block
// on each delta is expensive). The full Markdown (with syntax highlighting) is
// used once the message is done streaming.
function StreamingMarkdown({ children }: { children: string }) {
  return (
    <div className="prose-umans stream-caret">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={streamComponents}>
        {children}
      </ReactMarkdown>
    </div>
  );
}

const streamComponents: ComponentPropsWithoutRef<typeof ReactMarkdown>["components"] = {
  pre({ children }) {
    return <pre className="!my-2 !border !border-ink-800 !rounded-lg !bg-[#0a0a0c]">{children}</pre>;
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

function UserMessage({ m }: { m: UserMsg }) {
  return (
    <div className="group flex justify-end px-4 py-2 sm:px-6">
      <div className="relative max-w-[85%]">
        <div className="whitespace-pre-wrap break-words rounded-2xl rounded-tr-sm border border-ink-700/60 bg-ink-800/70 px-4 py-2.5 text-[14px] leading-relaxed text-ink-100">
          {m.text}
        </div>
        <div className="absolute -left-9 top-1.5 opacity-0 transition-opacity group-hover:opacity-100">
          <CopyBtn text={m.text} />
        </div>
      </div>
    </div>
  );
}

function AssistantMessage({ m }: { m: AssistantMsg }) {
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
        <span className="ml-auto opacity-0 transition-opacity group-hover:opacity-100">
          <CopyBtn text={m.text} />
        </span>
      </div>
      <div className="pl-7">
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
      <div className="ml-7 rounded-lg border border-ink-800 bg-ink-925/40 px-3 py-2">
        <div className="font-mono text-[11px] text-ink-400">
          {m.toolName || "tool"} result · {m.ok ? "ok" : "error"}
        </div>
        <pre
          className={`mt-1 max-h-60 overflow-auto whitespace-pre-wrap break-words text-[12px] ${
            m.ok ? "text-ink-200" : "text-rose-300"
          }`}
        >
          {m.output}
        </pre>
      </div>
    </div>
  );
}

export const Message = memo(function Message({ m }: { m: UserMsg | AssistantMsg | ToolMsg }) {
  if (m.role === "user") return <UserMessage m={m} />;
  if (m.role === "assistant") return <AssistantMessage m={m} />;
  return <ToolMessage m={m} />;
});
