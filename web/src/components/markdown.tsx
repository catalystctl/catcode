"use client";

// Compact Markdown — react-markdown + GFM + syntax highlighting.
// Code blocks get a header bar with the language + a copy button.

import { memo, useMemo, useRef, useState, type ComponentPropsWithoutRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { CopyIcon, CheckIcon } from "./icons";

const DEFAULT_REHYPE_PLUGINS: ComponentPropsWithoutRef<typeof ReactMarkdown>["rehypePlugins"] = [
  rehypeHighlight,
];

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
    <div className="group/code my-3 overflow-hidden rounded-sm border border-ink-800 bg-ink-925">
      <div className="flex items-center justify-between border-b border-ink-800 bg-ink-900 px-2 py-1">
        <span className="font-mono text-[10px] uppercase tracking-wider text-ink-500">{lang}</span>
        <button
          onClick={copy}
          className={`flex items-center gap-1 rounded-sm px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider transition-colors ${
            copied ? "text-success" : "text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          }`}
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

const baseComponents: ComponentPropsWithoutRef<typeof ReactMarkdown>["components"] = {
  pre({ children }) {
    // The CodeBlock wrapper replaces the default <pre>; react-markdown already
    // rendered <pre><code class="language-x">...</code></pre>.
    const codeEl = (Array.isArray(children) ? children[0] : children) as
      | React.ReactElement<{ className?: string; children?: React.ReactNode }>
      | undefined;
    const props = codeEl?.props ?? {};
    return <CodeBlock className={props.className}>{props.children}</CodeBlock>;
  },
  table({ children, ...props }) {
    return (
      <div className="markdown-table-scroll" role="region" aria-label="Scrollable table" tabIndex={0}>
        <table {...props}>{children}</table>
      </div>
    );
  },
};

export const Markdown = memo(function Markdown({
  children,
  variant = "compact",
  resolveImageUrl,
  rehypePlugins = DEFAULT_REHYPE_PLUGINS,
}: {
  children: string;
  variant?: "compact" | "document";
  resolveImageUrl?: (source: string) => string;
  rehypePlugins?: ComponentPropsWithoutRef<typeof ReactMarkdown>["rehypePlugins"];
}) {
  const components = useMemo<ComponentPropsWithoutRef<typeof ReactMarkdown>["components"]>(
    () => ({
      ...baseComponents,
      a({ children: linkChildren, href, ...props }) {
        const fragment = typeof href === "string" && href.startsWith("#");
        const safeHref = fragment && !href.startsWith("#user-content-")
          ? `#user-content-${href.slice(1)}`
          : href;
        return (
          <a
            {...props}
            href={safeHref}
            target={fragment ? undefined : "_blank"}
            rel={fragment ? undefined : "noopener noreferrer"}
          >
            {linkChildren}
          </a>
        );
      },
      img({ src, alt, ...props }) {
        const source = typeof src === "string" ? src : "";
        // eslint-disable-next-line @next/next/no-img-element -- Markdown images may be workspace API URLs.
        return <img {...props} src={resolveImageUrl?.(source) ?? source} alt={alt ?? ""} loading="lazy" decoding="async" />;
      },
    }),
    [resolveImageUrl],
  );

  return (
    <div className={`prose-catalyst ${variant === "document" ? "prose-catalyst-document" : ""}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={rehypePlugins}
        components={components}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
});
