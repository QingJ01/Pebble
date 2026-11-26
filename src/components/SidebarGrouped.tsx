import { accountOptionLabel } from "@/lib/accountIdentity";
import { useEffect, useMemo, useState } from "react";
import {
  Inbox,
  Send,
  FileEdit,
  AlertTriangle,
  Folder,
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
import SidebarButton from "./SidebarButton";
import { useUIStore } from "../stores/ui.store";
import { isComposeDirty, useComposeStore } from "../stores/compose.store";
import { useMailStore } from "../stores/mail.store";
import { useAccountsQuery, useFoldersForAccountsQuery } from "../hooks/queries";
import { useFolderUnreadCountsForAccounts } from "../hooks/queries/useFolderUnreadCounts";
import {
  allAccountsFolderId,
  buildAllAccountsFolders,
  sortFoldersForSidebar,
  unreadCountForFolder,
} from "../lib/folderAggregation";
import { profileLocalStorage } from "../lib/profileStorage";
import { useSidebarFolderManagement } from "../hooks/useSidebarFolderManagement";
import type { Account, Folder as FolderType } from "../lib/api";

const EMPTY_ACCOUNTS: Account[] = [];
const EMPTY_FOLDERS: FolderType[] = [];

type AccountGroupRole = NonNullable<FolderType["role"]>;

const ROLE_ICONS: Record<AccountGroupRole, React.ReactNode> = {
  inbox: <Inbox size={16} />,
  sent: <Send size={16} />,
  drafts: <FileEdit size={16} />,
  spam: <AlertTriangle size={16} />,
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

export default function SidebarGrouped({ collapsed: sidebarCollapsed }: { collapsed: boolean }) {
  const { t } = useTranslation();
  const activeView = useUIStore((s) => s.activeView);
  const setActiveView = useUIStore((s) => s.setActiveView);
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
  const { handleContextMenu: handleSidebarContextMenu, overlay: folderManagementOverlay } = useSidebarFolderManagement({ accounts, folders, accountId: activeAccountId });

  const ROLE_LABELS: Record<AccountGroupRole, string> = {
    inbox: t("sidebar.inbox"),
    sent: t("sidebar.sent"),
    drafts: t("sidebar.drafts"),
    spam: t("sidebar.spam"),
    archive: t("sidebar.archive"),
    trash: t("sidebar.trash"),
  };

  const foldersByAccount = useMemo(() => {
    const byAccount = new Map<string, FolderType[]>();
    for (const folder of folders) {
      const accountFolders = byAccount.get(folder.account_id) ?? [];
      accountFolders.push(folder);
      byAccount.set(folder.account_id, accountFolders);
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

  async function safeSetActiveView(view: Parameters<typeof setActiveView>[0]) {
    setActiveView(view);
  }

  async function handleFolderSelect(accountId: string | null, folderId: string) {
    if (isComposeDirty()) {
      useComposeStore.setState({
        showComposeLeaveConfirm: true, pendingView: "inbox",
        pendingNavigation: () => { setActiveAccountId(accountId); setActiveFolderId(folderId); },
      });
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


  return (
    <aside
      className="app-sidebar"
      aria-label={t("sidebar.navigation", "Sidebar")}
      onContextMenu={handleSidebarContextMenu}
      style={{ width: sidebarCollapsed ? "48px" : "200px", flexShrink: 0 }}
    >
      {/* Search button */}
      <nav aria-label={t("sidebar.search", "Search")} className="sidebar-nav sidebar-nav--top">
        <SidebarButton
          icon={<Search size={16} />}
          label={t("search.title", "Search")}
          isActive={activeView === "search"}
          collapsed={sidebarCollapsed}
          onClick={() => safeSetActiveView("search")}
        />
      </nav>

      {/* Section label */}
      {!sidebarCollapsed && (
        <div className="sidebar-section-label">
          {t("sidebar.mail", "Mail")}
        </div>
      )}

      {/* Overview + per-account folder groups */}
      <nav
        className="scroll-region sidebar-folder-scroll sidebar-nav sidebar-nav--folders"
        aria-label={t("sidebar.mailFolders", "Mail folders")}
      >
        {accounts.length > 0 && (
          <>
            <SidebarButton
              icon={<Layers size={16} />}
              label={t("sidebar.overview", "Overview")}
              badge={showUnread ? unreadCountForFolder(allAccountsFolderId("inbox"), folders, unreadCounts) : undefined}
              isActive={isOverviewActive}
              collapsed={sidebarCollapsed}
              onClick={() => handleFolderSelect(null, allAccountsFolderId("inbox"))}
            />
            <SidebarButton
              icon={<Star size={16} />}
              label={t("sidebar.starred", "Starred")}
              isActive={activeView === "starred"}
              collapsed={sidebarCollapsed}
              onClick={() => safeSetActiveView("starred")}
            />
            {accounts.map((account) => {
              const accountName = accountOptionLabel(account);
              const accountFolders = foldersByAccount.get(account.id) ?? EMPTY_FOLDERS;
              const entries = sortFoldersForSidebar(accountFolders);
              const groupCollapsed = !sidebarCollapsed && collapsedAccountIds.has(account.id);
              return (
                <div
                  key={account.id}
                  className={sidebarCollapsed ? "sidebar-group sidebar-group--rail" : "sidebar-group"}
                >
                  {!sidebarCollapsed && (
                    <button
                      type="button"
                      aria-expanded={!groupCollapsed}
                      title={accountName}
                      className="sidebar-group-header"
                      onClick={() => toggleAccountGroup(account.id)}
                    >
                      {groupCollapsed ? <ChevronRight size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}
                      <span className="sidebar-btn__label">
                        {accountName}
                      </span>
                    </button>
                  )}
                  {!groupCollapsed && (entries.length > 0
                    ? entries.map((folder) => {
                        const label = folder.remote_id === "__local_outbox__" ? t("sidebar.outbox", "Outbox")
                          : folder.role ? ROLE_LABELS[folder.role] : folder.name;
                        return (
                          <SidebarButton
                            key={folder.id}
                            icon={folder.role ? ROLE_ICONS[folder.role] : <Folder size={16} />}
                            label={sidebarCollapsed ? `${accountName} · ${label}` : label}
                            badge={showUnread ? unreadCountForFolder(folder.id, accountFolders, unreadCounts) : undefined}
                            isActive={activeView === "inbox" && activeAccountId === account.id && activeFolderId === folder.id}
                            collapsed={sidebarCollapsed}
                            indent
                            onClick={() => handleFolderSelect(account.id, folder.id)}
                          />
                        );
                      })
                    : !sidebarCollapsed && foldersFetched && (
                        <div className="sidebar-empty-note">
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
      <div className="sidebar-divider" />

      {/* Bottom nav: Contacts + Snoozed + Kanban + Settings */}
      <nav
        aria-label={t("sidebar.tools", "Tools")}
        className="sidebar-nav sidebar-nav--bottom"
      >
        <SidebarButton
          icon={<ContactRound size={16} />}
          label={t("sidebar.contacts", "Contacts")}
          isActive={activeView === "contacts"}
          collapsed={sidebarCollapsed}
          onClick={() => safeSetActiveView("contacts")}
        />
        <SidebarButton
          icon={<Clock size={16} />}
          label={t("sidebar.snoozed", "Snoozed")}
          isActive={activeView === "snoozed"}
          collapsed={sidebarCollapsed}
          onClick={() => safeSetActiveView("snoozed")}
        />
        <SidebarButton
          icon={<LayoutGrid size={16} />}
          label={t("sidebar.kanban", "Kanban")}
          isActive={activeView === "kanban"}
          collapsed={sidebarCollapsed}
          onClick={() => safeSetActiveView("kanban")}
        />
        <SidebarButton
          icon={<Settings size={16} />}
          label={t("sidebar.settings", "Settings")}
          isActive={activeView === "settings"}
          collapsed={sidebarCollapsed}
          onClick={() => safeSetActiveView("settings")}
        />
      </nav>
      {folderManagementOverlay}
    </aside>
  );
}
