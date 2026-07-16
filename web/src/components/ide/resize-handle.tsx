"use client";

// Shared draggable splitter for the IDE shell. Supports both horizontal drags
// (resizes a column width — orientation "x") and vertical drags (resizes a row
// height — orientation "y"). `invert` flips the growth direction: by default a
// positive drag (right/down) grows the size; with `invert` a negative drag
// (left/up) grows it (used for the right-hand copilot dock and the bottom panel,
// whose resize handles are on their inner edges).
//
// The handle captures the pointer start + the current `size` at pointerdown and
// computes the new absolute size from those on every pointermove, so it stays
// correct even if the parent re-renders mid-drag (the active move handler reads
// refs, not props). The parent's setter applies its own clamp too (defense in
// depth).
//
// `setPointerCapture` keeps receiving events when the cursor crosses iframes
// (preview/screen) or WebGL canvases (Ghostty) — without it, resize is hit-or-
// miss depending on what sits under the pointer path.

import { useCallback, useEffect, useRef } from "react";

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
  const draggingRef = useRef(false);
  const cleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    return () => {
      cleanupRef.current?.();
    };
  }, []);

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (e.button !== 0 && e.pointerType === "mouse") return;
      if (draggingRef.current) return;
      e.preventDefault();

      const el = e.currentTarget;
      const pointerId = e.pointerId;
      try {
        el.setPointerCapture(pointerId);
      } catch {
        // Capture can fail on some environments; window listeners still help.
      }

      draggingRef.current = true;
      startPosRef.current = orientation === "x" ? e.clientX : e.clientY;
      startSizeRef.current = size;
      const prevCursor = document.body.style.cursor;
      const prevSelect = document.body.style.userSelect;
      document.body.style.cursor = orientation === "x" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";
      document.body.classList.add("catalyst-resizing");

      const onMove = (ev: PointerEvent) => {
        if (ev.pointerId !== pointerId) return;
        const cur = orientation === "x" ? ev.clientX : ev.clientY;
        const delta = invert ? startPosRef.current - cur : cur - startPosRef.current;
        const next = startSizeRef.current + delta;
        onResize(Math.max(min, Math.min(max, next)));
      };

      const cleanup = () => {
        if (!draggingRef.current) return;
        draggingRef.current = false;
        document.body.style.cursor = prevCursor;
        document.body.style.userSelect = prevSelect;
        document.body.classList.remove("catalyst-resizing");
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
        el.removeEventListener("lostpointercapture", onLostCapture);
        try {
          if (el.hasPointerCapture(pointerId)) el.releasePointerCapture(pointerId);
        } catch {
          /* ignore */
        }
        cleanupRef.current = null;
      };

      const onUp = (ev: PointerEvent) => {
        if (ev.pointerId !== pointerId) return;
        cleanup();
      };

      const onLostCapture = (ev: Event) => {
        const pev = ev as PointerEvent;
        if (pev.pointerId !== pointerId) return;
        cleanup();
      };

      cleanupRef.current = cleanup;
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
      el.addEventListener("lostpointercapture", onLostCapture);
    },
    [orientation, invert, size, onResize, min, max],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const step = e.shiftKey ? 20 : 10;
      let delta = 0;
      if (orientation === "x") {
        if (e.key === "ArrowLeft") delta = invert ? step : -step;
        else if (e.key === "ArrowRight") delta = invert ? -step : step;
      } else {
        if (e.key === "ArrowUp") delta = invert ? step : -step;
        else if (e.key === "ArrowDown") delta = invert ? -step : step;
      }
      if (!delta) return;
      e.preventDefault();
      onResize(Math.max(min, Math.min(max, size + delta)));
    },
    [orientation, invert, size, onResize, min, max],
  );

  const isX = orientation === "x";
  return (
    <div
      role="separator"
      tabIndex={0}
      aria-orientation={isX ? "vertical" : "horizontal"}
      aria-valuemin={min === 0 ? undefined : min}
      aria-valuemax={max === Infinity ? undefined : max}
      aria-valuenow={Math.round(size)}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      className={`group relative z-20 shrink-0 touch-none select-none bg-ink-800 transition-colors hover:bg-accent/60 focus:bg-accent/60 focus:outline-none ${
        isX ? "w-px cursor-col-resize" : "h-px cursor-row-resize"
      }`}
    >
      {/* Wider hit slug; receives the same pointerdown via bubbling to this node. */}
      <span
        aria-hidden
        className={`absolute z-20 ${
          isX
            ? "inset-y-0 -left-1.5 -right-1.5 cursor-col-resize"
            : "inset-x-0 -top-1.5 -bottom-1.5 cursor-row-resize"
        }`}
      />
    </div>
  );
}
