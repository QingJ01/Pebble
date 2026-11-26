import { useSyncExternalStore } from "react";
import { useUIStore } from "../stores/ui.store";
import SidebarClassic from "./SidebarClassic";
import SidebarGrouped from "./SidebarGrouped";

/** Below this window width the sidebar collapses to its 48px icon rail. */
export const SIDEBAR_RAIL_BREAKPOINT = 1000;
const NARROW_QUERY = `(max-width: ${SIDEBAR_RAIL_BREAKPOINT - 1}px)`;

function subscribeNarrow(onChange: () => void) {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const mql = window.matchMedia(NARROW_QUERY);
  mql.addEventListener("change", onChange);
  return () => mql.removeEventListener("change", onChange);
}

function isNarrowWindow() {
  return typeof window !== "undefined" && !!window.matchMedia && window.matchMedia(NARROW_QUERY).matches;
}

/** True while the window is narrower than SIDEBAR_RAIL_BREAKPOINT. */
export function useNarrowWindow(): boolean {
  return useSyncExternalStore(subscribeNarrow, isNarrowWindow, () => false);
}

export default function Sidebar() {
  const sidebarStyle = useUIStore((s) => s.sidebarStyle);
  const userCollapsed = useUIStore((s) => s.sidebarCollapsed);
  const narrow = useNarrowWindow();
  // Narrow windows always get the icon rail; the user's own preference returns
  // as soon as there is room again.
  const collapsed = userCollapsed || narrow;
  return sidebarStyle === "classic"
    ? <SidebarClassic collapsed={collapsed} />
    : <SidebarGrouped collapsed={collapsed} />;
}
