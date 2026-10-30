import { useUIStore } from "../stores/ui.store";
import SidebarClassic from "./SidebarClassic";
import SidebarGrouped from "./SidebarGrouped";

export default function Sidebar() {
  const sidebarStyle = useUIStore((s) => s.sidebarStyle);
  return sidebarStyle === "classic" ? <SidebarClassic /> : <SidebarGrouped />;
}
