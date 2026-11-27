import { accountOptionLabel } from "@/lib/accountIdentity";
import { useEffect, useMemo } from "react";
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
  Clock,
  Star,
  ContactRound,
  Layers,
  FolderOpen,
  FolderCog,
  Users,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import SidebarButton from "./SidebarButton";
import SidebarRail, { RailMenuButton, type RailMenuItem } from "./SidebarRail";
import AccountBadge, { accountShortName } from "./ui/AccountBadge";
import { useResolvedAccounts } from "../hooks/useAccountBadges";
import { useUIStore } from "../stores/ui.store";
import { isComposeDirty, useComposeStore } from "../stores/compose.store";
import { useMailStore } from "../stores/mail.store";
import { useFoldersForAccountsQuery } from "../hooks/queries";
import { useFolderUnreadCountsForAccounts } from "../hooks/queries/useFolderUnreadCounts";
import {
  ALL_ACCOUNTS_SELECT_VALUE,
  allAccountsFolderId,
  buildAllAccountsFolders,
  sortFoldersForSidebar,
  unreadCountForFolder,
} from "../lib/folderAggregation";
import { useSidebarFolderManagement } from "../hooks/useSidebarFolderManagement";
import type { Folder as FolderType } from "../lib/api";

const EMPTY_FOLDERS: FolderType[] = [];

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
  { role: "archive", labelKey: "sidebar.archive" },
  { role: "drafts", labelKey: "sidebar.drafts" },
  { role: "trash", labelKey: "sidebar.trash" },
  { role: "spam", labelKey: "sidebar.spam" },
];

export default function SidebarClassic({ collapsed: sidebarCollapsed }: { collapsed: boolean }) {
  const { t } = useTranslation();
  const activeView = useUIStore((s) => s.activeView);
  const setActiveView = useUIStore((s) => s.setActiveView);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const setActiveAccountId = useMailStore((s) => s.setActiveAccountId);
  const setActiveFolderId = useMailStore((s) => s.setActiveFolderId);

  const showUnread = useUIStore((s) => s.showFolderUnreadCount);
  const accounts = useResolvedAccounts();
  const allAccountsMode = accounts.length > 1 && !activeAccountId;
  const folderAccountIds = useMemo(
    () => activeAccountId ? [activeAccountId] : accounts.map((account) => account.id),
    [accounts, activeAccountId],
  );
  const { data: folders = EMPTY_FOLDERS, isFetched: foldersFetched } = useFoldersForAccountsQuery(folderAccountIds);
  const { data: unreadCounts = {} } = useFolderUnreadCountsForAccounts(folderAccountIds);
  const { handleContextMenu: handleSidebarContextMenu, overlay: folderManagementOverlay, openManager } = useSidebarFolderManagement({ accounts, folders, accountId: activeAccountId });
  const ROLE_LABELS: Record<string, string> = {
    inbox: t("sidebar.inbox"),
    sent: t("sidebar.sent"),
    drafts: t("sidebar.drafts"),
    trash: t("sidebar.trash"),
    archive: t("sidebar.archive"),
    spam: t("sidebar.spam"),
  };
  const folderLabel = (folder: FolderType) => folder.remote_id === "__local_outbox__"
    ? t("sidebar.outbox", "Outbox") : (folder.role && ROLE_LABELS[folder.role]) || folder.name;

  const displayedFolders = useMemo(
    () => allAccountsMode ? buildAllAccountsFolders(folders) : folders,
    [allAccountsMode, folders],
  );
  const hasRealFolders = displayedFolders.length > 0;

  // Keep system folders stable across all-account and single-account views.
  const dedupedFolders = useMemo(() => {
    return sortFoldersForSidebar(displayedFolders);
  }, [displayedFolders]);

  // Auto-select the only account. With multiple accounts, null means the
  // combined "all accounts" mailbox.
  useEffect(() => {
    if (accounts.length === 1 && !activeAccountId) {
      setActiveAccountId(accounts[0].id);
    }
  }, [accounts, activeAccountId, setActiveAccountId]);

  // Auto-select inbox folder when folders load.
  // If the selected account has no folders, try the next account.
  useEffect(() => {
    if (displayedFolders.length > 0 && !activeFolderId) {
      const inbox = displayedFolders.find((f) => f.role === "inbox");
      setActiveFolderId((inbox ?? displayedFolders[0]).id);
    } else if (!allAccountsMode && foldersFetched && displayedFolders.length === 0 && activeAccountId && accounts.length > 1) {
      const idx = accounts.findIndex((a) => a.id === activeAccountId);
      const next = accounts[idx + 1] ?? accounts.find((a) => a.id !== activeAccountId);
      if (next) {
        setActiveAccountId(next.id);
      }
    }
  }, [displayedFolders, foldersFetched, activeFolderId, setActiveFolderId, accounts, activeAccountId, setActiveAccountId, allAccountsMode]);

  async function safeSetActiveView(view: Parameters<typeof setActiveView>[0]) {
    setActiveView(view);
  }

  function switchAccount(accountId: string | null) {
    setActiveAccountId(accountId);
    setActiveFolderId(null);
  }

  async function handleFolderClick(folderId: string) {
    if (isComposeDirty()) {
      useComposeStore.setState({
        showComposeLeaveConfirm: true, pendingView: "inbox",
        pendingNavigation: () => setActiveFolderId(folderId),
      });
      return;
    }
    setActiveView("inbox");
    setActiveFolderId(folderId);
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

  const starredButton = (
    <SidebarButton
      key="__starred__"
      icon={<Star size={16} />}
      label={t("sidebar.starred", "Starred")}
      isActive={activeView === "starred"}
      collapsed={sidebarCollapsed}
      onClick={() => safeSetActiveView("starred")}
    />
  );

  if (sidebarCollapsed) {
    const activeAccount = accounts.find((account) => account.id === activeAccountId) ?? null;
    const allAccountsLabel = t("sidebar.allAccounts", "All accounts");
    const currentName = activeAccount ? accountShortName(activeAccount) : allAccountsLabel;
    // Only the current account's system folders sit on the rail; custom
    // folders go behind one "More folders" menu so the rail stays short.
    const mainFolders = dedupedFolders.filter((folder) => folder.role);
    const otherFolders = dedupedFolders.filter((folder) => !folder.role);
    const folderItem = (folder: FolderType): RailMenuItem => ({
      id: folder.id,
      icon: folderIcon(folder.role),
      label: folderLabel(folder),
      count: showUnread ? unreadCountForFolder(folder.id, folders, unreadCounts) : undefined,
      current: folder.id === activeFolderId && activeView === "inbox",
      onSelect: () => handleFolderClick(folder.id),
      dropFolderId: folder.id,
    });
    const switcherItems: RailMenuItem[] = accounts.map((account) => ({
      id: account.id,
      icon: <AccountBadge account={account} size="xs" />,
      label: accountOptionLabel(account),
      current: account.id === activeAccountId,
      onSelect: () => switchAccount(account.id),
    }));
    // "Overview" is the classic sidebar's all-accounts mailbox.
    const overviewFolderId = allAccountsFolderId("inbox");
    const overviewButton = accounts.length > 1 && (
      <SidebarButton
        icon={<Layers size={16} />}
        label={t("sidebar.overview", "Overview")}
        isActive={!activeAccountId && activeView === "inbox" && activeFolderId === overviewFolderId}
        collapsed
        onClick={() => {
          // Overview is always the all-accounts inbox, even when another
          // all-accounts folder (Sent, Archive…) is open.
          const go = () => {
            setActiveAccountId(null);
            setActiveFolderId(overviewFolderId);
            setActiveView("inbox");
          };
          if (isComposeDirty()) {
            useComposeStore.setState({ showComposeLeaveConfirm: true, pendingView: "inbox", pendingNavigation: go });
            return;
          }
          go();
        }}
      />
    );
    return (
      <SidebarRail
        label={t("sidebar.navigation", "Sidebar")}
        topLabel={t("sidebar.mail", "Mail")}
        top={<>{searchButton}{overviewButton}{starredButton}</>}
        middleLabel={t("sidebar.mailFolders", "Mail folders")}
        middle={accounts.length > 0 && (
          <>
            <RailMenuButton
              className="sidebar-btn--account"
              face={activeAccount
                ? <AccountBadge account={activeAccount} size="md" />
                : <span className="sidebar-btn__icon" aria-hidden="true"><Users size={16} /></span>}
              label={t("sidebar.switchAccountCurrent", "Switch account (current: {{name}})", { name: currentName })}
              title={currentName}
              menuLabel={t("sidebar.switchAccount", "Switch account")}
              items={switcherItems}
            />
            {mainFolders.map((folder) => (
              <SidebarButton
                key={folder.id}
                icon={folderIcon(folder.role)}
                label={folderLabel(folder)}
                badge={showUnread ? unreadCountForFolder(folder.id, folders, unreadCounts) : undefined}
                isActive={folder.id === activeFolderId && activeView === "inbox"}
                collapsed
                onClick={() => handleFolderClick(folder.id)}
                dropFolderId={folder.id}
              />
            ))}
            {(otherFolders.length > 0 || hasRealFolders) && (
              <RailMenuButton
                face={<span className="sidebar-btn__icon" aria-hidden="true"><FolderOpen size={16} /></span>}
                label={t("sidebar.moreFolders", "More folders")}
                isActive={otherFolders.some((folder) => folder.id === activeFolderId) && activeView === "inbox"}
                menuLabel={t("sidebar.moreFolders", "More folders")}
                items={otherFolders.map(folderItem)}
                emptyNote={t("sidebar.noFolders", "No folders yet")}
                footerItems={[{
                  id: "__manage__",
                  icon: <FolderCog size={16} />,
                  label: t("folderManagement.manage", "Manage folders"),
                  onSelect: () => openManager(activeAccountId),
                }]}
              />
            )}
          </>
        )}
        bottomLabel={t("sidebar.tools", "Tools")}
        bottom={toolsButtons}
        onContextMenu={handleSidebarContextMenu}
      >
        {folderManagementOverlay}
      </SidebarRail>
    );
  }

  const activeAccountForSwitcher = accounts.find((account) => account.id === activeAccountId) ?? null;

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

      {/* Account switcher */}
      {accounts.length > 0 && (
        <div className="sidebar-account-switcher">
          <span className="sidebar-account-switcher__badge" aria-hidden="true">
            {activeAccountForSwitcher
              ? <AccountBadge account={activeAccountForSwitcher} size="xs" />
              : <Layers size={16} />}
          </span>
          <select
            aria-label={t("settings.emailAccounts", "Email Accounts")}
            value={activeAccountId || ALL_ACCOUNTS_SELECT_VALUE}
            onChange={(e) => switchAccount(e.target.value === ALL_ACCOUNTS_SELECT_VALUE ? null : e.target.value)}
            className="select"
          >
            <option value={ALL_ACCOUNTS_SELECT_VALUE}>
              {t("sidebar.allAccounts", "All accounts")}
            </option>
            {accounts.map((acc) => (
              <option key={acc.id} value={acc.id}>
                {accountOptionLabel(acc)}
              </option>
            ))}
          </select>
        </div>
      )}

      {/* Folders section */}
      <nav
        className="scroll-region sidebar-folder-scroll sidebar-nav sidebar-nav--folders"
        aria-label={t("sidebar.mailFolders", "Mail folders")}
      >
        {hasRealFolders
          ? dedupedFolders.flatMap((folder) => {
              const items: React.ReactNode[] = [];
              if (folder.role === "drafts") items.push(starredButton);
              items.push(
                <SidebarButton
                  key={folder.id}
                  icon={folderIcon(folder.role)}
                  label={folderLabel(folder)}
                  badge={showUnread ? unreadCountForFolder(folder.id, folders, unreadCounts) : undefined}
                  isActive={folder.id === activeFolderId && activeView === "inbox"}
                  collapsed={false}
                  onClick={() => handleFolderClick(folder.id)}
                  dropFolderId={folder.id}
                />
              );
              return items;
            })
          : DEFAULT_FOLDERS.flatMap((df, index) => {
              const items: React.ReactNode[] = [];
              if (df.role === "drafts") items.push(starredButton);
              items.push(
                <SidebarButton
                  key={df.role}
                  icon={ROLE_ICONS[df.role] || <Folder size={16} />}
                  label={t(df.labelKey)}
                  isActive={index === 0 && activeView === "inbox"}
                  collapsed={false}
                  onClick={() => safeSetActiveView("inbox")}
                />
              );
              return items;
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
