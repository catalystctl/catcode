"use client";

import { useEffect, useState } from "react";

/** Subscribe to a CSS media query. Returns `false` during SSR / first paint. */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false);

  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia(query);
    const update = () => setMatches(mq.matches);
    update();
    mq.addEventListener("change", update);
    return () => mq.removeEventListener("change", update);
  }, [query]);

  return matches;
}

/** True below Tailwind's `lg` breakpoint (1024px). Multi-pane IDE chrome
 *  needs roughly that width; tablet portrait uses the mobile shell. */
export function useIsMobile(): boolean {
  return useMediaQuery("(max-width: 1023px)");
}
