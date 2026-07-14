"use client";

// Full document Markdown is kept separate from chat Markdown so raw-HTML
// parsing/sanitizing remains in the lazy Preview chunk rather than the initial
// application bundle.

import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import rehypeSlug from "rehype-slug";
import type { PluggableList } from "unified";
import { Markdown } from "./markdown";

const DOCUMENT_PLUGINS: PluggableList = [
  rehypeRaw,
  rehypeSlug,
  // Sanitize after IDs are generated so DOM-clobbering names are prefixed too;
  // highlighting runs afterward and only adds trusted token classes.
  [rehypeSanitize, defaultSchema],
  rehypeHighlight,
];

export function MarkdownDocument({
  children,
  resolveImageUrl,
}: {
  children: string;
  resolveImageUrl?: (source: string) => string;
}) {
  return (
    <Markdown
      variant="document"
      resolveImageUrl={resolveImageUrl}
      rehypePlugins={DOCUMENT_PLUGINS}
    >
      {children}
    </Markdown>
  );
}
