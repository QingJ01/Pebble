import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../stores/ui.store";
import { useMailStore } from "../stores/mail.store";
import { useNarrowWindow } from "../lib/narrowWindow";
import { useOverlayLayer } from "../hooks/useOverlayLayer";
import { useFocusScope } from "../hooks/useFocusScope";
import SidebarClassic from "./SidebarClassic";
import SidebarGrouped from "./SidebarGrouped";

function closeNarrowSidebar() {
  useUIStore.setState({ sidebarNarrowExpanded: false });
}

/**
 * Narrow window + "Toggle sidebar": the full sidebar floats over the content
 * as a drawer (a rail-wide chrome strip keeps its place in the layout), so the
 * list and reader keep their width. A click outside, Escape or navigating closes it.
 */
function NarrowSidebarDrawer({ classic }: { classic: boolean }) {
  const { t } = useTranslation();
  const panelRef = useRef<HTMLDivElement>(null);
  useOverlayLayer({ open: true, modal: false, onEscape: closeNarrowSidebar, containerRef: panelRef });
  // Opened from the keyboard (command palette): put focus in the drawer, and
  // hand it back when the drawer closes.
  useFocusScope({ active: true, containerRef: panelRef, initialFocus: "first", restoreFocus: true });

  // Picking a folder or view is the end of the trip: close the drawer.
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const activeView = useUIStore((s) => s.activeView);
  const initial = useRef({ activeFolderId, activeAccountId, activeView });
  useEffect(() => {
    const start = initial.current;
    if (activeFolderId !== start.activeFolderId || activeAccountId !== start.activeAccountId || activeView !== start.activeView) {
      closeNarrowSidebar();
    }
  }, [activeFolderId, activeAccountId, activeView]);

  return (
    <div className="sidebar-drawer-host">
      <div
        className="sidebar-drawer-backdrop"
        aria-hidden="true"
        onMouseDown={closeNarrowSidebar}
      />
      <div ref={panelRef} className="sidebar-drawer" role="dialog" aria-label={t("sidebar.navigation", "Sidebar")}>
        {classic ? <SidebarClassic collapsed={false} /> : <SidebarGrouped collapsed={false} />}
      </div>
    </div>
  );
}

export default function Sidebar() {
  const sidebarStyle = useUIStore((s) => s.sidebarStyle);
  const userCollapsed = useUIStore((s) => s.sidebarCollapsed);
  const narrowExpanded = useUIStore((s) => s.sidebarNarrowExpanded);
  const narrow = useNarrowWindow();
  // Narrow windows get the icon rail unless the user expands it there with
  // "Toggle sidebar"; that override ends when the window widens again, and the
  // user's own preference returns.
  useEffect(() => {
    if (!narrow && narrowExpanded) closeNarrowSidebar();
  }, [narrow, narrowExpanded]);
  const classic = sidebarStyle === "classic";
  if (narrow && narrowExpanded) return <NarrowSidebarDrawer classic={classic} />;
  const collapsed = narrow || userCollapsed;
  return classic
    ? <SidebarClassic collapsed={collapsed} />
    : <SidebarGrouped collapsed={collapsed} />;
}
