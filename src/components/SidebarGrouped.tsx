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
  FolderCog,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import SidebarButton from "./SidebarButton";
import SidebarRail, { RailMenuButton } from "./SidebarRail";
import AccountBadge, { accountShortName } from "./ui/AccountBadge";
import { useResolvedAccounts } from "../hooks/useAccountBadges";
import { useUIStore } from "../stores/ui.store";
import { isComposeDirty, useComposeStore } from "../stores/compose.store";
import { useMailStore } from "../stores/mail.store";
import { useFoldersForAccountsQuery } from "../hooks/queries";
import { useFolderUnreadCountsForAccounts } from "../hooks/queries/useFolderUnreadCounts";
import {
  allAccountsFolderId,
  buildAllAccountsFolders,
  sortFoldersForSidebar,
  unreadCountForFolder,
} from "../lib/folderAggregation";
import { profileLocalStorage } from "../lib/profileStorage";
import { ancestorKeys, buildFolderTree, totalForNode, type FolderTreeNode } from "../lib/folderTree";
import { useSidebarFolderManagement } from "../hooks/useSidebarFolderManagement";
import type { Folder as FolderType } from "../lib/api";

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

const EXPANDED_FOLDERS_KEY = "pebble-sidebar-expanded-folders";

function readExpandedFolderKeys(): Set<string> {
  try {
    const stored = profileLocalStorage.getItem(EXPANDED_FOLDERS_KEY);
    const parsed = stored ? JSON.parse(stored) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((key): key is string => typeof key === "string") : []);
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
  const accounts = useResolvedAccounts();
  // The sidebar always renders every account group, so it needs all folders.
  const accountIds = useMemo(() => accounts.map((account) => account.id), [accounts]);
  const { data: folders = EMPTY_FOLDERS, isFetched: foldersFetched } = useFoldersForAccountsQuery(accountIds);
  const { data: unreadCounts = {} } = useFolderUnreadCountsForAccounts(accountIds);
  const { handleContextMenu: handleSidebarContextMenu, overlay: folderManagementOverlay, openManager } = useSidebarFolderManagement({ accounts, folders, accountId: activeAccountId });

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

  const [expandedFolderKeys, setExpandedFolderKeys] = useState<Set<string>>(readExpandedFolderKeys);
  function toggleFolderExpanded(stateKey: string) {
    setExpandedFolderKeys((prev) => {
      const next = new Set(prev);
      if (next.has(stateKey)) next.delete(stateKey);
      else next.add(stateKey);
      try {
        profileLocalStorage.setItem(EXPANDED_FOLDERS_KEY, JSON.stringify([...next]));
      } catch {
        // storage unavailable: keep the in-memory state
      }
      return next;
    });
  }

  const isOverviewActive = activeView === "inbox" && !activeAccountId;

  function folderLabel(folder: FolderType) {
    if (folder.remote_id === "__local_outbox__") return t("sidebar.outbox", "Outbox");
    return folder.role ? ROLE_LABELS[folder.role] : folder.name;
  }

  function folderIconFor(folder: FolderType) {
    return folder.role ? ROLE_ICONS[folder.role] : <Folder size={16} />;
  }

  const toolsButtons = (
    <>
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
    </>
  );

  const searchButton = (
    <SidebarButton
      icon={<Search size={16} />}
      label={t("search.title", "Search")}
      isActive={activeView === "search"}
      collapsed={sidebarCollapsed}
      onClick={() => safeSetActiveView("search")}
    />
  );

  const overviewAndStarred = accounts.length > 0 && (
    <>
      <SidebarButton
        icon={<Layers size={16} />}
        label={t("sidebar.overview", "Overview")}
        badge={showUnread ? unreadCountForFolder(allAccountsFolderId("inbox"), folders, unreadCounts) : undefined}
        isActive={isOverviewActive}
        collapsed={sidebarCollapsed}
        onClick={() => handleFolderSelect(null, allAccountsFolderId("inbox"))}
        dropFolderId={allAccountsFolderId("inbox")}
      />
      <SidebarButton
        icon={<Star size={16} />}
        label={t("sidebar.starred", "Starred")}
        isActive={activeView === "starred"}
        collapsed={sidebarCollapsed}
        onClick={() => safeSetActiveView("starred")}
      />
    </>
  );

  if (sidebarCollapsed) {
    return (
      <SidebarRail
        label={t("sidebar.navigation", "Sidebar")}
        topLabel={t("sidebar.mail", "Mail")}
        top={<>{searchButton}{overviewAndStarred}</>}
        middleLabel={t("sidebar.accounts", "Accounts")}
        middle={accounts.map((account) => {
          const accountFolders = foldersByAccount.get(account.id) ?? EMPTY_FOLDERS;
          const entries = sortFoldersForSidebar(accountFolders);
          const inbox = entries.find((folder) => folder.role === "inbox");
          // The dot and label follow "show folder unread counts" like the
          // expanded sidebar (the filter bar may fetch counts regardless).
          const inboxUnread = showUnread && inbox ? unreadCountForFolder(inbox.id, accountFolders, unreadCounts) : 0;
          const name = accountShortName(account);
          const buttonLabel = inboxUnread > 0
            ? t("sidebar.accountUnread", "{{name}} · {{count}} unread", { name, count: inboxUnread })
            : name;
          const isAccountActive = activeView === "inbox" && activeAccountId === account.id;
          return (
            <RailMenuButton
              key={account.id}
              className="sidebar-btn--account"
              face={<AccountBadge account={account} size="md" selected={isAccountActive} unread={inboxUnread > 0} />}
              label={buttonLabel}
              menuLabel={name}
              menuHeader={<span className="sidebar-rail-menu__header-text" title={accountOptionLabel(account)}>{accountOptionLabel(account)}</span>}
              emptyNote={foldersFetched ? t("sidebar.noFolders", "No folders yet") : undefined}
              items={entries.map((folder) => ({
                id: folder.id,
                icon: folderIconFor(folder),
                label: folderLabel(folder),
                count: showUnread ? unreadCountForFolder(folder.id, accountFolders, unreadCounts) : undefined,
                current: isAccountActive && activeFolderId === folder.id,
                onSelect: () => handleFolderSelect(account.id, folder.id),
                dropFolderId: folder.id,
              }))}
              footerItems={[{
                id: "__manage__",
                icon: <FolderCog size={16} />,
                label: t("folderManagement.manage", "Manage folders"),
                onSelect: () => openManager(account.id),
              }]}
            />
          );
        })}
        bottomLabel={t("sidebar.tools", "Tools")}
        bottom={toolsButtons}
        onContextMenu={handleSidebarContextMenu}
      >
        {folderManagementOverlay}
      </SidebarRail>
    );
  }


  return (
    <aside
      className="app-sidebar"
      aria-label={t("sidebar.navigation", "Sidebar")}
      onContextMenu={handleSidebarContextMenu}
      style={{ width: "200px", flexShrink: 0 }}
    >
      <nav aria-label={t("sidebar.search", "Search")} className="sidebar-nav sidebar-nav--top">
        {searchButton}
      </nav>

      <div className="sidebar-section-label">
        {t("sidebar.mail", "Mail")}
      </div>

      {/* Overview + per-account folder groups */}
      <nav
        className="scroll-region sidebar-folder-scroll sidebar-nav sidebar-nav--folders"
        aria-label={t("sidebar.mailFolders", "Mail folders")}
      >
        {overviewAndStarred}
        {accounts.map((account) => {
          const accountName = accountOptionLabel(account);
          const accountFolders = foldersByAccount.get(account.id) ?? EMPTY_FOLDERS;
          const entries = sortFoldersForSidebar(accountFolders);
          const groupCollapsed = collapsedAccountIds.has(account.id);
          return (
            <div key={account.id} className="sidebar-group">
              <button
                type="button"
                aria-expanded={!groupCollapsed}
                title={accountName}
                className="sidebar-group-header"
                onClick={() => toggleAccountGroup(account.id)}
              >
                {groupCollapsed ? <ChevronRight size={12} aria-hidden="true" /> : <ChevronDown size={12} aria-hidden="true" />}
                <AccountBadge account={account} size="xs" />
                <span className="sidebar-btn__label">
                  {accountName}
                </span>
              </button>
              {!groupCollapsed && (entries.length > 0
                ? (() => {
                    const tree = buildFolderTree(entries);
                    const openAncestors = new Set(activeAccountId === account.id ? ancestorKeys(tree, activeFolderId) : []);
                    const unreadFor = (f: FolderType) => unreadCountForFolder(f.id, accountFolders, unreadCounts);
                    const renderNode = (node: FolderTreeNode, depth: number): React.ReactNode => {
                      const stateKey = `${account.id}:${node.key}`;
                      const hasChildren = node.children.length > 0;
                      const expanded = hasChildren && (expandedFolderKeys.has(stateKey) || openAncestors.has(node.key));
                      const folder = node.folder;
                      const label = folder && (folder.role || folder.remote_id === "__local_outbox__") ? folderLabel(folder) : node.label;
                      // A collapsed parent shows the unread total of everything inside it
                      const badge = !showUnread ? undefined
                        : hasChildren && !expanded ? totalForNode(node, unreadFor)
                        : folder ? unreadFor(folder) : undefined;
                      return (
                        <div key={node.key} role="treeitem" aria-expanded={hasChildren ? expanded : undefined}>
                          <div className="sidebar-tree-row" style={{ "--tree-depth": depth } as React.CSSProperties}>
                            {hasChildren && (
                              <button
                                type="button"
                                className="sidebar-tree-toggle"
                                aria-label={expanded
                                  ? t("sidebar.collapseFolder", { name: label, defaultValue: "Collapse {{name}}" })
                                  : t("sidebar.expandFolder", { name: label, defaultValue: "Expand {{name}}" })}
                                onClick={(e) => { e.stopPropagation(); toggleFolderExpanded(stateKey); }}
                              >
                                {expanded ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
                              </button>
                            )}
                            <SidebarButton
                              icon={folder ? folderIconFor(folder) : <Folder size={16} />}
                              label={label}
                              tooltip={folder && !folder.role && folder.name !== label ? folder.name : undefined}
                              badge={badge}
                              isActive={!!folder && activeView === "inbox" && activeAccountId === account.id && activeFolderId === folder.id}
                              collapsed={false}
                              indent
                              onClick={() => folder ? handleFolderSelect(account.id, folder.id) : toggleFolderExpanded(stateKey)}
                              dropFolderId={folder?.id}
                            />
                          </div>
                          {expanded && (
                            <div role="group">
                              {node.children.map((child) => renderNode(child, depth + 1))}
                            </div>
                          )}
                        </div>
                      );
                    };
                    return <div role="tree" aria-label={accountName}>{tree.map((node) => renderNode(node, 0))}</div>;
                  })()
                : foldersFetched && (
                    <div className="sidebar-empty-note">
                      {t("sidebar.noFolders", "No folders yet")}
                    </div>
                  ))}
            </div>
          );
        })}
      </nav>

      <div className="sidebar-divider" />

      {/* Bottom nav: Contacts + Snoozed + Kanban + Settings */}
      <nav
        aria-label={t("sidebar.tools", "Tools")}
        className="sidebar-nav sidebar-nav--bottom"
      >
        {toolsButtons}
      </nav>
      {folderManagementOverlay}
    </aside>
  );
}
