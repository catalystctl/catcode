"use client";

// Diff — a shared, theme-aware unified-diff renderer. Used by ToolCallCard
// (tool results with a diff) and Approval (pre-write diff preview). Lines are
// coloured add/del/hunk; backgrounds use CSS variables so light theme works.

export function Diff({ diff, className }: { diff: string; className?: string }) {
  const lines = diff.split("\n");
  return (
    <pre
      className={`overflow-x-auto rounded-none border border-ink-800 bg-ink-950 p-3 font-mono text-[12px] leading-relaxed ${
        className ?? ""
      }`}
    >
      {lines.map((l, i) => {
        const cls =
          l.startsWith("+") && !l.startsWith("+++")
            ? "diff-line-add"
            : l.startsWith("-") && !l.startsWith("---")
              ? "diff-line-del"
              : l.startsWith("@@")
                ? "diff-line-hunk"
                : "";
        return (
          <div key={i} className={`${cls} px-1`}>
            {l || " "}
          </div>
        );
      })}
    </pre>
  );
}
