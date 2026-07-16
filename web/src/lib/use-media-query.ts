"use client";

import { useCallback, useSyncExternalStore } from "react";

function subscribeMediaQuery(query: string, onStoreChange: () => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const mq = window.matchMedia(query);
  mq.addEventListener("change", onStoreChange);
  return () => mq.removeEventListener("change", onStoreChange);
}

/** Subscribe to a CSS media query. SSR / first server snapshot is `false`. */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onStoreChange: () => void) => subscribeMediaQuery(query, onStoreChange),
    [query],
  );
  const getSnapshot = useCallback(() => {
    if (typeof window === "undefined" || !window.matchMedia) return false;
    return window.matchMedia(query).matches;
  }, [query]);
  const getServerSnapshot = useCallback(() => false, []);
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

/** True below Tailwind's `lg` breakpoint (1024px). Multi-pane IDE chrome
 *  needs roughly that width; tablet portrait uses the mobile shell. */
export function useIsMobile(): boolean {
  return useMediaQuery("(max-width: 1023px)");
}
