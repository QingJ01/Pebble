import { useSyncExternalStore } from "react";

/** Below this window width the sidebar collapses to its 48px icon rail. */
export const SIDEBAR_RAIL_BREAKPOINT = 1000;
const NARROW_QUERY = `(max-width: ${SIDEBAR_RAIL_BREAKPOINT - 1}px)`;

function subscribeNarrow(onChange: () => void) {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const mql = window.matchMedia(NARROW_QUERY);
  mql.addEventListener("change", onChange);
  return () => mql.removeEventListener("change", onChange);
}

export function isNarrowWindow() {
  return typeof window !== "undefined" && !!window.matchMedia && window.matchMedia(NARROW_QUERY).matches;
}

/** True while the window is narrower than SIDEBAR_RAIL_BREAKPOINT. */
export function useNarrowWindow(): boolean {
  return useSyncExternalStore(subscribeNarrow, isNarrowWindow, () => false);
}
