"use client";

// Shared draggable splitter for the IDE shell. Supports both horizontal drags
// (resizes a column width — orientation "x") and vertical drags (resizes a row
// height — orientation "y"). `invert` flips the growth direction: by default a
// positive drag (right/down) grows the size; with `invert` a negative drag
// (left/up) grows it (used for the right-hand copilot dock and the bottom panel,
// whose resize handles are on their inner edges).
//
// The handle captures the pointer start + the current `size` at mousedown and
// computes the new absolute size from those on every mousemove, so it stays
// correct even if the parent re-renders mid-drag (the active move handler reads
// refs, not props). The parent's setter applies its own clamp too (defense in
// depth).

import { useCallback, useRef } from "react";

export interface ResizeHandleProps {
  /** "x" = drag horizontally (resizes width); "y" = drag vertically (resizes height). */
  orientation: "x" | "y";
  /** When true, moving the pointer toward 0 (left/up) grows the size. */
  invert?: boolean;
  /** Current size in px (captured at drag start). */
  size: number;
  /** Called with the new absolute size in px as the pointer moves. */
  onResize: (px: number) => void;
  /** Clamp bounds applied here so the handle is self-contained. */
  min?: number;
  max?: number;
}

export function ResizeHandle({
  orientation,
  invert = false,
  size,
  onResize,
  min = 0,
  max = Infinity,
}: ResizeHandleProps) {
  const startPosRef = useRef(0);
  const startSizeRef = useRef(0);

  const onMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      startPosRef.current = orientation === "x" ? e.clientX : e.clientY;
      startSizeRef.current = size;
      const prevCursor = document.body.style.cursor;
      const prevSelect = document.body.style.userSelect;
      document.body.style.cursor = orientation === "x" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";

      const onMove = (ev: MouseEvent) => {
        const cur = orientation === "x" ? ev.clientX : ev.clientY;
        const delta = invert ? startPosRef.current - cur : cur - startPosRef.current;
        const next = startSizeRef.current + delta;
        onResize(Math.max(min, Math.min(max, next)));
      };
      const onUp = () => {
        document.body.style.cursor = prevCursor;
        document.body.style.userSelect = prevSelect;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [orientation, invert, size, onResize, min, max],
  );

  const isX = orientation === "x";
  return (
    <div
      role="separator"
      aria-orientation={isX ? "vertical" : "horizontal"}
      onMouseDown={onMouseDown}
      className={`group relative z-10 shrink-0 bg-ink-800 transition-colors hover:bg-accent/60 ${
        isX ? "w-px cursor-col-resize" : "h-px cursor-row-resize"
      }`}
    >
      {/* Wider hit area without visible thickness. */}
      <span
        className={`absolute ${
          isX ? "inset-y-0 -left-1 -right-1" : "inset-x-0 -top-1 -bottom-1"
        }`}
      />
    </div>
  );
}
