"use client";

// Recursive renderer for the hub split-tree. Leaves are rendered by the
// caller's renderLeaf; splits are flex rows/columns with a draggable divider.
// Ratio drags report a fraction of the container via onRatioChange — the hub
// owns the state so persistence and React keys stay centralized.

import { useRef, type ReactNode } from "react";
import type { LayoutNode, SplitNode } from "@/lib/hub-layout";

export interface SplitViewProps {
  node: LayoutNode;
  onRatioChange: (splitId: string, ratio: number) => void;
  renderLeaf: (leafId: string) => ReactNode;
}

export function SplitView({ node, onRatioChange, renderLeaf }: SplitViewProps) {
  if (node.kind === "leaf") return <>{renderLeaf(node.id)}</>;
  return <SplitInner node={node} onRatioChange={onRatioChange} renderLeaf={renderLeaf} />;
}

function SplitInner({ node, onRatioChange, renderLeaf }: SplitViewProps & { node: SplitNode }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const horizontal = node.dir === "h";

  const startDrag = (event: React.PointerEvent) => {
    const container = containerRef.current;
    if (!container) return;
    event.preventDefault();
    const move = (ev: PointerEvent) => {
      const rect = container.getBoundingClientRect();
      const frac = horizontal
        ? (ev.clientX - rect.left) / Math.max(1, rect.width)
        : (ev.clientY - rect.top) / Math.max(1, rect.height);
      onRatioChange(node.id, frac);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const firstSize = `calc(${(node.ratio * 100).toFixed(3)}% - 3px)`;

  return (
    <div
      ref={containerRef}
      className={`flex h-full w-full min-h-0 min-w-0 ${horizontal ? "flex-row" : "flex-col"}`}
    >
      <div
        key={node.children[0].kind === "split" ? node.children[0].id : undefined}
        className="min-h-0 min-w-0"
        style={horizontal ? { width: firstSize } : { height: firstSize }}
      >
        <SplitView node={node.children[0]} onRatioChange={onRatioChange} renderLeaf={renderLeaf} />
      </div>
      <div
        role="separator"
        aria-orientation={horizontal ? "vertical" : "horizontal"}
        onPointerDown={startDrag}
        className={`shrink-0 bg-ink-800/70 transition-colors hover:bg-accent/60 ${
          horizontal ? "w-1.5 cursor-col-resize" : "h-1.5 cursor-row-resize"
        }`}
      />
      <div className="min-h-0 min-w-0 flex-1">
        <SplitView node={node.children[1]} onRatioChange={onRatioChange} renderLeaf={renderLeaf} />
      </div>
    </div>
  );
}
