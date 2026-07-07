"use client";

// Markdown — react-markdown + GFM + syntax highlighting (rehype-highlight).
// Code blocks get a header bar with the language + a copy button.

import { memo, useRef, useState, type ComponentPropsWithoutRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { CopyIcon, CheckIcon } from "./icons";

function CodeBlock({ className, children }: { className?: string; children?: React.ReactNode }) {
  const lang = /language-(\w+)/.exec(className || "")?.[1] ?? "text";
  const [copied, setCopied] = useState(false);
  const codeRef = useRef<HTMLElement>(null);
  const copy = () => {
    // Read the rendered text from the DOM so the copied text is exactly what's
    // shown — including highlighted code, where `children` is an array of
    // rehype-highlight <span> tokens (so String(children) would be "[object
    // Object],…"). Reading textContent at click time is robust to any tree.
    const code = (codeRef.current?.textContent ?? "").replace(/\n$/, "");
    navigator.clipboard?.writeText(code).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1400);
      },
      () => {},
    );
  };
  return (
    <div className="group/code my-3 overflow-hidden rounded-xl border border-ink-800 bg-ink-950">
      <div className="flex items-center justify-between border-b border-ink-800/80 bg-ink-925/60 px-3 py-1.5">
        <span className="font-mono text-[11px] uppercase tracking-wider text-ink-400">{lang}</span>
        <button
          onClick={copy}
          className="flex items-center gap-1 rounded-md px-1.5 py-1 text-[11px] text-ink-400 transition-colors hover:bg-ink-800 hover:text-ink-100"
        >
          {copied ? <CheckIcon width={12} height={12} /> : <CopyIcon width={12} height={12} />}
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="!my-0 !border-0 !rounded-none">
        <code ref={codeRef} className={className}>
          {children}
        </code>
      </pre>
    </div>
  );
}

const components: ComponentPropsWithoutRef<typeof ReactMarkdown>["components"] = {
  pre({ children }) {
    // The CodeBlock wrapper replaces the default <pre>; react-markdown already
    // rendered <pre><code class="language-x">...</code></pre>.
    const codeEl = (Array.isArray(children) ? children[0] : children) as
      | React.ReactElement<{ className?: string; children?: React.ReactNode }>
      | undefined;
    const props = codeEl?.props ?? {};
    return <CodeBlock className={props.className}>{props.children}</CodeBlock>;
  },
  a({ children, ...props }) {
    return (
      <a target="_blank" rel="noopener noreferrer" {...props}>
        {children}
      </a>
    );
  },
};

export const Markdown = memo(function Markdown({ children }: { children: string }) {
  return (
    <div className="prose-catalyst">
      <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeHighlight]} components={components}>
        {children}
      </ReactMarkdown>
    </div>
  );
});
