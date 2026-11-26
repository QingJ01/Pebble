import { useEffect } from "react";
import { useUIStore } from "../stores/ui.store";
import { useNarrowWindow } from "../lib/narrowWindow";
import SidebarClassic from "./SidebarClassic";
import SidebarGrouped from "./SidebarGrouped";

export default function Sidebar() {
  const sidebarStyle = useUIStore((s) => s.sidebarStyle);
  const userCollapsed = useUIStore((s) => s.sidebarCollapsed);
  const narrowExpanded = useUIStore((s) => s.sidebarNarrowExpanded);
  const narrow = useNarrowWindow();
  // Narrow windows get the icon rail unless the user expands it there with
  // "Toggle sidebar"; that override ends when the window widens again, and the
  // user's own preference returns.
  useEffect(() => {
    if (!narrow && narrowExpanded) useUIStore.setState({ sidebarNarrowExpanded: false });
  }, [narrow, narrowExpanded]);
  const collapsed = narrow ? !narrowExpanded : userCollapsed;
  return sidebarStyle === "classic"
    ? <SidebarClassic collapsed={collapsed} />
    : <SidebarGrouped collapsed={collapsed} />;
}
