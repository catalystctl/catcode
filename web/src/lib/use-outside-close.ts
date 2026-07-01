"use client";

// useOutsideClose — shared hook for dropdowns/modals: close on outside click
// or Escape. Extracted from the copy-pasted version in header/intercom/
// memory/plugins/settings so there is one implementation.

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

export function useOutsideClose(onClose: () => void, enabled = true) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!enabled) return;
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", h);
    document.addEventListener("keydown", k);
    return () => {
      document.removeEventListener("mousedown", h);
      document.removeEventListener("keydown", k);
    };
  }, [onClose, enabled]);
  return ref;
}
