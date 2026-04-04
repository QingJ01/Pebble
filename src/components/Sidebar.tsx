import { useEffect } from "react";
import {
  Inbox,
  Send,
  FileEdit,
  Trash2,
  Archive,
  AlertTriangle,
  Folder,
  LayoutGrid,
  Settings,
  Search,
  PenLine,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../stores/ui.store";
import { useMailStore } from "../stores/mail.store";
import { useAccountsQuery, useFoldersQuery } from "../hooks/queries";
import type { Folder as FolderType } from "../lib/api";

const ROLE_ICONS: Record<string, React.ReactNode> = {
  inbox: <Inbox size={16} />,
  sent: <Send size={16} />,
  drafts: <FileEdit size={16} />,
  trash: <Trash2 size={16} />,
  archive: <Archive size={16} />,
  spam: <AlertTriangle size={16} />,
};

function folderIcon(role: FolderType["role"]): React.ReactNode {
  return (role && ROLE_ICONS[role]) || <Folder size={16} />;
}

// Default folders shown when no account is configured
const DEFAULT_FOLDERS: { role: string; labelKey: string }[] = [
  { role: "inbox", labelKey: "sidebar.inbox" },
  { role: "sent", labelKey: "sidebar.sent" },
  { role: "drafts", labelKey: "sidebar.drafts" },
  { role: "trash", labelKey: "sidebar.trash" },
  { role: "archive", labelKey: "sidebar.archive" },
  { role: "spam", labelKey: "sidebar.spam" },
];

export default function Sidebar() {
  const { t } = useTranslation();
  const { activeView, setActiveView, sidebarCollapsed } = useUIStore();
  const openCompose = useUIStore((s) => s.openCompose);
  const {
    activeFolderId,
    activeAccountId,
    setActiveAccountId,
    setActiveFolderId,
  } = useMailStore();

  const { data: accounts = [] } = useAccountsQuery();
  const { data: folders = [] } = useFoldersQuery(activeAccountId);

  const hasRealFolders = folders.length > 0;

  // Auto-select first account
  useEffect(() => {
    if (accounts.length > 0 && !activeAccountId) {
      setActiveAccountId(accounts[0].id);
    }
  }, [accounts, activeAccountId, setActiveAccountId]);

  // Auto-select inbox folder when folders load
  useEffect(() => {
    if (folders.length > 0 && !activeFolderId) {
      const inbox = folders.find((f) => f.role === "inbox");
      if (inbox) {
        setActiveFolderId(inbox.id);
      }
    }
  }, [folders, activeFolderId, setActiveFolderId]);

  function handleFolderClick(folderId: string) {
    setActiveView("inbox");
    setActiveFolderId(folderId);
  }

  const buttonBase: React.CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: "8px",
    borderRadius: "6px",
    padding: sidebarCollapsed ? "7px" : "6px 10px",
    width: "100%",
    border: "none",
    cursor: "pointer",
    fontSize: "13px",
    textAlign: "left",
    justifyContent: sidebarCollapsed ? "center" : "flex-start",
  };

  return (
    <aside
      style={{
        width: sidebarCollapsed ? "48px" : "200px",
        backgroundColor: "var(--color-sidebar-bg)",
        borderRight: "1px solid var(--color-border)",
        transition: "width 150ms ease",
        display: "flex",
        flexDirection: "column",
        height: "100%",
        overflow: "hidden",
      }}
    >
      {/* Compose + Search buttons */}
      <nav style={{ padding: "8px 6px 0", display: "flex", flexDirection: "column", gap: "1px" }}>
        <SidebarButton
          icon={<PenLine size={16} />}
          label={t("sidebar.compose", "Compose")}
          isActive={false}
          collapsed={sidebarCollapsed}
          style={{
            ...buttonBase,
            backgroundColor: "var(--color-accent)",
            color: "#fff",
            marginBottom: "4px",
          }}
          onClick={() => openCompose("new")}
        />
        <SidebarButton
          icon={<Search size={16} />}
          label={t("search.title", "Search")}
          isActive={activeView === "search"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => setActiveView("search")}
        />
      </nav>

      {/* Section label */}
      {!sidebarCollapsed && (
        <div style={{
          padding: "12px 10px 4px 10px",
          fontSize: "11px",
          fontWeight: 600,
          color: "var(--color-text-secondary)",
          textTransform: "uppercase",
          letterSpacing: "0.5px",
        }}>
          {t("sidebar.mail", "Mail")}
        </div>
      )}

      {/* Folders section */}
      <nav
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "0 6px",
          display: "flex",
          flexDirection: "column",
          gap: "1px",
        }}
      >
        {hasRealFolders
          ? folders.map((folder) => {
              const isActive = folder.id === activeFolderId && activeView === "inbox";
              return (
                <SidebarButton
                  key={folder.id}
                  icon={folderIcon(folder.role)}
                  label={folder.name}
                  isActive={isActive}
                  collapsed={sidebarCollapsed}
                  style={buttonBase}
                  onClick={() => handleFolderClick(folder.id)}
                />
              );
            })
          : DEFAULT_FOLDERS.map((df, index) => (
              <SidebarButton
                key={df.role}
                icon={ROLE_ICONS[df.role] || <Folder size={16} />}
                label={t(df.labelKey)}
                isActive={index === 0 && activeView === "inbox"}
                collapsed={sidebarCollapsed}
                style={buttonBase}
                onClick={() => setActiveView("inbox")}
              />
            ))}
      </nav>

      {/* Divider */}
      <div
        style={{
          height: "1px",
          backgroundColor: "var(--color-border)",
          margin: "0 6px",
        }}
      />

      {/* Bottom nav: Kanban + Settings */}
      <nav
        style={{
          padding: "6px 6px 8px",
          display: "flex",
          flexDirection: "column",
          gap: "1px",
        }}
      >
        <SidebarButton
          icon={<LayoutGrid size={16} />}
          label={t("sidebar.kanban", "Kanban")}
          isActive={activeView === "kanban"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => setActiveView("kanban")}
        />
        <SidebarButton
          icon={<Settings size={16} />}
          label={t("sidebar.settings", "Settings")}
          isActive={activeView === "settings"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => setActiveView("settings")}
        />
      </nav>
    </aside>
  );
}

// Reusable sidebar button to avoid repetitive hover logic
function SidebarButton({
  icon, label, isActive, collapsed, style, disabled, onClick,
}: {
  icon: React.ReactNode;
  label: string;
  isActive: boolean;
  collapsed: boolean;
  style: React.CSSProperties;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      title={collapsed ? label : undefined}
      style={{
        ...style,
        backgroundColor: isActive
          ? "var(--color-sidebar-active)"
          : style.backgroundColor ?? "transparent",
        color: style.color ?? "var(--color-text-primary)",
        opacity: disabled ? 0.45 : 1,
        cursor: disabled ? "default" : "pointer",
        transition: "background-color 0.15s ease, opacity 0.15s ease",
      }}
      onMouseEnter={(e) => {
        if (!isActive && !style.backgroundColor)
          e.currentTarget.style.backgroundColor = "var(--color-sidebar-hover)";
      }}
      onMouseLeave={(e) => {
        if (!isActive && !style.backgroundColor)
          e.currentTarget.style.backgroundColor = "transparent";
      }}
    >
      {icon}
      {!collapsed && (
        <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {label}
        </span>
      )}
    </button>
  );
}
