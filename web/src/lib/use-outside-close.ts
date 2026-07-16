"use client";

// useOutsideClose — shared hook for dropdowns/modals: close on outside click
// or Escape. Escape uses a LIFO stack so only the topmost enabled overlay
// closes (nested AppDialog / Settings / header menus don't all dismiss at once).

import { useEffect, useRef } from "react";

/** Merge multiple refs into one callback ref (for combining useOutsideClose +
 *  useFocusTrap on the same element). */
export function mergeRefs<T>(
  ...refs: Array<React.MutableRefObject<T | null> | React.RefCallback<T> | null | undefined>
) {
  return (el: T | null) => {
    for (const r of refs) {
      if (!r) continue;
      if (typeof r === "function") r(el);
      else r.current = el;
    }
  };
}

type EscapeHandler = () => void;
const escapeStack: EscapeHandler[] = [];

export function useOutsideClose(onClose: () => void, enabled = true) {
  const ref = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!enabled) return;
    const close = () => onCloseRef.current();
    escapeStack.push(close);

    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const k = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (e.defaultPrevented) return;
      // Only the topmost registered overlay handles Escape.
      if (escapeStack[escapeStack.length - 1] !== close) return;
      e.preventDefault();
      close();
    };
    document.addEventListener("mousedown", h);
    document.addEventListener("keydown", k);
    return () => {
      const idx = escapeStack.lastIndexOf(close);
      if (idx >= 0) escapeStack.splice(idx, 1);
      document.removeEventListener("mousedown", h);
      document.removeEventListener("keydown", k);
    };
  }, [enabled]);
  return ref;
}
