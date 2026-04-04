import { Inbox, LayoutGrid, Settings } from "lucide-react";
import { useUIStore, type ActiveView } from "../stores/ui.store";

interface NavItem {
  id: ActiveView;
  label: string;
  icon: React.ReactNode;
}

const NAV_ITEMS: NavItem[] = [
  { id: "inbox", label: "Inbox", icon: <Inbox size={18} /> },
  { id: "kanban", label: "Kanban", icon: <LayoutGrid size={18} /> },
  { id: "settings", label: "Settings", icon: <Settings size={18} /> },
];

export default function Sidebar() {
  const { activeView, setActiveView, sidebarCollapsed } = useUIStore();

  return (
    <aside
      className="flex flex-col border-r h-full"
      style={{
        width: sidebarCollapsed ? "48px" : "200px",
        backgroundColor: "var(--color-sidebar-bg)",
        borderColor: "var(--color-border)",
        transition: "width 150ms ease",
      }}
    >
      <nav className="flex flex-col gap-0.5 p-2 mt-1">
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            onClick={() => setActiveView(item.id)}
            className="flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm"
            style={{
              backgroundColor:
                activeView === item.id
                  ? "var(--color-sidebar-active)"
                  : "transparent",
              color: "var(--color-text-primary)",
            }}
            onMouseEnter={(e) => {
              if (activeView !== item.id)
                e.currentTarget.style.backgroundColor =
                  "var(--color-sidebar-hover)";
            }}
            onMouseLeave={(e) => {
              if (activeView !== item.id)
                e.currentTarget.style.backgroundColor = "transparent";
            }}
          >
            {item.icon}
            {!sidebarCollapsed && <span>{item.label}</span>}
          </button>
        ))}
      </nav>
    </aside>
  );
}
