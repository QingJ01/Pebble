import { accountOptionLabel } from "@/lib/accountIdentity";
import { useEffect, useMemo, useState } from "react";
import {
  Inbox,
  Archive,
  Trash2,
  LayoutGrid,
  Settings,
  Search,
  Clock,
  Star,
  ContactRound,
  Layers,
  ChevronDown,
  ChevronRight,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../stores/ui.store";
import { isComposeDirty, useComposeStore } from "../stores/compose.store";
import { useConfirmStore } from "../stores/confirm.store";
import { useMailStore } from "../stores/mail.store";
import { useAccountsQuery, useFoldersForAccountsQuery } from "../hooks/queries";
import { useFolderUnreadCountsForAccounts } from "../hooks/queries/useFolderUnreadCounts";
import {
  allAccountsFolderId,
  buildAllAccountsFolders,
  unreadCountForFolder,
} from "../lib/folderAggregation";
import { profileLocalStorage } from "../lib/profileStorage";
import type { Account, Folder as FolderType } from "../lib/api";

const EMPTY_ACCOUNTS: Account[] = [];
const EMPTY_FOLDERS: FolderType[] = [];

const ACCOUNT_GROUP_ROLES = ["inbox", "archive", "trash"] as const;
type AccountGroupRole = (typeof ACCOUNT_GROUP_ROLES)[number];

const ROLE_ICONS: Record<AccountGroupRole, React.ReactNode> = {
  inbox: <Inbox size={16} />,
  archive: <Archive size={16} />,
  trash: <Trash2 size={16} />,
};

const COLLAPSED_ACCOUNTS_KEY = "pebble-sidebar-collapsed-accounts";

function readCollapsedAccountIds(): Set<string> {
  try {
    const stored = profileLocalStorage.getItem(COLLAPSED_ACCOUNTS_KEY);
    const parsed = stored ? JSON.parse(stored) : [];
    return new Set(
      Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string") : [],
    );
  } catch {
    return new Set();
  }
}

export default function SidebarGrouped() {
  const { t } = useTranslation();
  const activeView = useUIStore((s) => s.activeView);
  const setActiveView = useUIStore((s) => s.setActiveView);
  const sidebarCollapsed = useUIStore((s) => s.sidebarCollapsed);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const setActiveAccountId = useMailStore((s) => s.setActiveAccountId);
  const setActiveFolderId = useMailStore((s) => s.setActiveFolderId);
  const [collapsedAccountIds, setCollapsedAccountIds] = useState<Set<string>>(readCollapsedAccountIds);

  const showUnread = useUIStore((s) => s.showFolderUnreadCount);
  const { data: accounts = EMPTY_ACCOUNTS } = useAccountsQuery();
  // The sidebar always renders every account group, so it needs all folders.
  const accountIds = useMemo(() => accounts.map((account) => account.id), [accounts]);
  const { data: folders = EMPTY_FOLDERS, isFetched: foldersFetched } = useFoldersForAccountsQuery(accountIds);
  const { data: unreadCounts = {} } = useFolderUnreadCountsForAccounts(accountIds);

  const ROLE_LABELS: Record<AccountGroupRole, string> = {
    inbox: t("sidebar.inbox"),
    archive: t("sidebar.archive"),
    trash: t("sidebar.trash"),
  };

  // First folder per role, mirroring the single-account sidebar order.
  const roleFoldersByAccount = useMemo(() => {
    const byAccount = new Map<string, Map<AccountGroupRole, FolderType>>();
    for (const folder of folders) {
      const role = folder.role as AccountGroupRole | null;
      if (!role || !ACCOUNT_GROUP_ROLES.includes(role)) continue;
      const roles = byAccount.get(folder.account_id) ?? new Map<AccountGroupRole, FolderType>();
      if (!roles.has(role)) roles.set(role, folder);
      byAccount.set(folder.account_id, roles);
    }
    return byAccount;
  }, [folders]);

  // Default to the merged all-accounts inbox (the "Overview" entry).
  useEffect(() => {
    if (activeFolderId || accounts.length === 0) return;
    const merged = buildAllAccountsFolders(folders);
    const preferred = merged.find((folder) => folder.role === "inbox") ?? merged[0];
    if (preferred) setActiveFolderId(preferred.id);
  }, [accounts.length, folders, activeFolderId, setActiveFolderId]);

  async function confirmDiscardDraft() {
    if (isComposeDirty()) {
      const confirmed = await useConfirmStore.getState().confirm({
        title: t("compose.discardDraft", "Discard draft"),
        message: t("compose.discardDraftConfirm", "You have an unsaved draft. Discard and leave?"),
        destructive: true,
      });
      return confirmed;
    }
    return true;
  }

  async function safeSetActiveView(view: Parameters<typeof setActiveView>[0]) {
    if (isComposeDirty()) {
      const confirmed = await confirmDiscardDraft();
      if (!confirmed) return;
      useComposeStore.getState().discardComposeAndSetActiveView(view);
      return;
    }
    setActiveView(view);
  }

  async function handleFolderSelect(accountId: string | null, folderId: string) {
    if (isComposeDirty()) {
      const confirmed = await confirmDiscardDraft();
      if (!confirmed) return;
      setActiveAccountId(accountId);
      setActiveFolderId(folderId);
      useComposeStore.getState().discardComposeAndSetActiveView("inbox");
      return;
    }
    setActiveAccountId(accountId);
    setActiveFolderId(folderId);
    setActiveView("inbox");
  }

  function toggleAccountGroup(accountId: string) {
    setCollapsedAccountIds((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) next.delete(accountId);
      else next.add(accountId);
      profileLocalStorage.setItem(COLLAPSED_ACCOUNTS_KEY, JSON.stringify([...next]));
      return next;
    });
  }

  const isOverviewActive = activeView === "inbox" && !activeAccountId;

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
  const childButtonBase: React.CSSProperties = sidebarCollapsed
    ? buttonBase
    : { ...buttonBase, paddingLeft: "26px" };

  const groupHeaderBase: React.CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: "5px",
    borderRadius: "6px",
    padding: "7px 8px",
    marginTop: "4px",
    width: "100%",
    border: "none",
    backgroundColor: "transparent",
    cursor: "pointer",
    fontSize: "11px",
    fontWeight: 600,
    color: "var(--color-text-secondary)",
    letterSpacing: "0.3px",
    textAlign: "left",
  };

  return (
    <aside
      aria-label={t("sidebar.navigation", "Sidebar")}
      style={{
        width: sidebarCollapsed ? "48px" : "200px",
        flexShrink: 0,
        backgroundColor: "var(--color-sidebar-bg)",
        borderRight: "1px solid var(--color-border)",
        transition: "width 150ms ease",
        display: "flex",
        flexDirection: "column",
        height: "100%",
        overflow: "hidden",
      }}
    >
      {/* Search button */}
      <nav aria-label={t("sidebar.search", "Search")} style={{ padding: "8px 6px 0", display: "flex", flexDirection: "column", gap: "1px" }}>
        <SidebarButton
          icon={<Search size={16} />}
          label={t("search.title", "Search")}
          isActive={activeView === "search"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => safeSetActiveView("search")}
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

      {/* Overview + per-account folder groups */}
      <nav
        className="scroll-region sidebar-folder-scroll"
        aria-label={t("sidebar.mailFolders", "Mail folders")}
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "0 6px",
          display: "flex",
          flexDirection: "column",
          gap: "1px",
        }}
      >
        {accounts.length > 0 && (
          <>
            <SidebarButton
              icon={<Layers size={16} />}
              label={t("sidebar.overview", "Overview")}
              badge={showUnread ? unreadCountForFolder(allAccountsFolderId("inbox"), folders, unreadCounts) : undefined}
              isActive={isOverviewActive}
              collapsed={sidebarCollapsed}
              style={buttonBase}
              onClick={() => handleFolderSelect(null, allAccountsFolderId("inbox"))}
            />
            <SidebarButton
              icon={<Star size={16} />}
              label={t("sidebar.starred", "Starred")}
              isActive={activeView === "starred"}
              collapsed={sidebarCollapsed}
              style={buttonBase}
              onClick={() => safeSetActiveView("starred")}
            />
            {accounts.map((account) => {
              const accountName = accountOptionLabel(account);
              const entries = ACCOUNT_GROUP_ROLES.flatMap((role) => {
                const folder = roleFoldersByAccount.get(account.id)?.get(role);
                return folder ? [{ role, folder }] : [];
              });
              const groupCollapsed = !sidebarCollapsed && collapsedAccountIds.has(account.id);
              return (
                <div
                  key={account.id}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "1px",
                    marginTop: sidebarCollapsed ? "4px" : undefined,
                  }}
                >
                  {!sidebarCollapsed && (
                    <button
                      type="button"
                      aria-expanded={!groupCollapsed}
                      title={accountName}
                      style={groupHeaderBase}
                      onClick={() => toggleAccountGroup(account.id)}
                      onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = "var(--color-sidebar-hover)"; }}
                      onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = "transparent"; }}
                    >
                      {groupCollapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
                        {accountName}
                      </span>
                    </button>
                  )}
                  {!groupCollapsed && (entries.length > 0
                    ? entries.map(({ role, folder }) => (
                        <SidebarButton
                          key={folder.id}
                          icon={ROLE_ICONS[role]}
                          label={sidebarCollapsed ? `${accountName} · ${ROLE_LABELS[role]}` : ROLE_LABELS[role]}
                          badge={showUnread ? (unreadCounts[folder.id] ?? 0) : undefined}
                          isActive={activeView === "inbox" && activeAccountId === account.id && activeFolderId === folder.id}
                          collapsed={sidebarCollapsed}
                          style={childButtonBase}
                          onClick={() => handleFolderSelect(account.id, folder.id)}
                        />
                      ))
                    : !sidebarCollapsed && foldersFetched && (
                        <div style={{ padding: "4px 8px", fontSize: "12px", color: "var(--color-text-secondary)" }}>
                          {t("sidebar.noFolders", "No folders yet")}
                        </div>
                      ))}
                </div>
              );
            })}
          </>
        )}
      </nav>

      {/* Divider */}
      <div
        style={{
          height: "1px",
          backgroundColor: "var(--color-border)",
          margin: "0 6px",
        }}
      />

      {/* Bottom nav: Contacts + Snoozed + Kanban + Settings */}
      <nav
        aria-label={t("sidebar.tools", "Tools")}
        style={{
          padding: "6px 6px 8px",
          display: "flex",
          flexDirection: "column",
          gap: "1px",
        }}
      >
        <SidebarButton
          icon={<ContactRound size={16} />}
          label={t("sidebar.contacts", "Contacts")}
          isActive={activeView === "contacts"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => safeSetActiveView("contacts")}
        />
        <SidebarButton
          icon={<Clock size={16} />}
          label={t("sidebar.snoozed", "Snoozed")}
          isActive={activeView === "snoozed"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => safeSetActiveView("snoozed")}
        />
        <SidebarButton
          icon={<LayoutGrid size={16} />}
          label={t("sidebar.kanban", "Kanban")}
          isActive={activeView === "kanban"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => safeSetActiveView("kanban")}
        />
        <SidebarButton
          icon={<Settings size={16} />}
          label={t("sidebar.settings", "Settings")}
          isActive={activeView === "settings"}
          collapsed={sidebarCollapsed}
          style={buttonBase}
          onClick={() => safeSetActiveView("settings")}
        />
      </nav>
    </aside>
  );
}

// Reusable sidebar button to avoid repetitive hover logic
function SidebarButton({
  icon, label, badge, isActive, collapsed, style, disabled, onClick,
}: {
  icon: React.ReactNode;
  label: string;
  badge?: number;
  isActive: boolean;
  collapsed: boolean;
  style: React.CSSProperties;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={collapsed ? label : undefined}
      aria-current={isActive ? "page" : undefined}
      title={collapsed ? label : undefined}
      disabled={disabled}
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
        <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
          {label}
        </span>
      )}
      {!collapsed && badge != null && badge > 0 && (
        <span style={{
          fontSize: "11px",
          fontWeight: 600,
          color: "var(--color-accent)",
          minWidth: "18px",
          textAlign: "right",
        }}>
          {badge}
        </span>
      )}
    </button>
  );
}
