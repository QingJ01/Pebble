import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Sidebar from "../../src/components/Sidebar";
import { useComposeStore } from "../../src/stores/compose.store";
import { useMailStore } from "../../src/stores/mail.store";
import { useUIStore } from "../../src/stores/ui.store";
import type { Account, Folder } from "../../src/lib/api";

const mocks = vi.hoisted(() => ({
  accounts: [] as Account[],
  folders: [] as Folder[],
  unreadCounts: {} as Record<string, number>,
}));

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "search.title": "Search",
        "sidebar.navigation": "Sidebar",
        "sidebar.search": "Search",
        "sidebar.mail": "Mail",
        "sidebar.mailFolders": "Mail folders",
        "sidebar.overview": "Overview",
        "sidebar.inbox": "Inbox",
        "sidebar.sent": "Sent",
        "sidebar.drafts": "Drafts",
        "sidebar.trash": "Trash",
        "sidebar.archive": "Archive",
        "sidebar.spam": "Spam",
        "sidebar.starred": "Starred",
        "sidebar.noFolders": "No folders yet",
        "sidebar.tools": "Tools",
        "sidebar.contacts": "Contacts",
        "sidebar.snoozed": "Snoozed",
        "sidebar.kanban": "Kanban",
        "sidebar.settings": "Settings",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

vi.mock("../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({
    data: mocks.accounts,
  }),
  useFoldersForAccountsQuery: () => ({
    data: mocks.folders,
    isFetched: true,
  }),
}));

vi.mock("../../src/hooks/queries/useFolderUnreadCounts", () => ({
  useFolderUnreadCountsForAccounts: () => ({ data: mocks.unreadCounts }),
}));

function account(id: string, email: string): Account {
  return {
    id,
    email,
    display_name: email,
    provider: "imap",
    color: null,
    created_at: 1,
    updated_at: 1,
  };
}

function folder(id: string, accountId: string, role: Folder["role"]): Folder {
  return {
    id,
    account_id: accountId,
    remote_id: id,
    name: role ?? id,
    folder_type: "folder",
    role,
    parent_id: null,
    color: null,
    is_system: true,
    sort_order: 0,
  };
}

function mailNav() {
  return screen.getByRole("navigation", { name: "Mail folders" });
}

describe("Sidebar account groups", () => {
  beforeEach(() => {
    localStorage.clear();
    useUIStore.setState({
      sidebarCollapsed: false,
      activeView: "inbox",
      previousView: "inbox",
      showFolderUnreadCount: false,
      backgroundImage: null,
      sidebarStyle: "grouped",
    });
    useMailStore.setState({
      activeAccountId: null,
      activeFolderId: "all:inbox",
    });
    useComposeStore.setState({
      composeMode: null,
      composeReplyTo: null,
      composeDirty: false,
      showComposeLeaveConfirm: false,
      pendingView: null,
    });
    mocks.accounts = [account("account-1", "user@example.com"), account("account-2", "second@example.com")];
    mocks.folders = [
      folder("a1-inbox", "account-1", "inbox"),
      folder("a1-archive", "account-1", "archive"),
      folder("a1-trash", "account-1", "trash"),
      folder("a2-inbox", "account-2", "inbox"),
      folder("a2-trash", "account-2", "trash"),
    ];
    mocks.unreadCounts = {};
  });

  it("renders overview, starred, and one tiled group per account without a dropdown", () => {
    render(<Sidebar />);

    const labels = within(mailNav())
      .getAllByRole("button")
      .map((button) => button.textContent);

    expect(labels).toEqual([
      "Overview",
      "Starred",
      "user@example.com",
      "Inbox",
      "Archive",
      "Trash",
      "second@example.com",
      "Inbox",
      "Trash",
    ]);
    expect(screen.queryByRole("combobox")).toBeNull();
  });

  it("switches to the classic sidebar when the appearance setting changes", () => {
    render(<Sidebar />);

    expect(screen.queryByRole("combobox")).toBeNull();

    act(() => {
      useUIStore.setState({ sidebarStyle: "classic" });
    });

    expect(screen.getByRole("combobox", { name: "Email Accounts" })).toBeTruthy();
  });

  it("selects a folder inside an account group", async () => {
    render(<Sidebar />);

    fireEvent.click(screen.getAllByRole("button", { name: "Inbox" })[0]);

    await waitFor(() => {
      expect(useMailStore.getState().activeAccountId).toBe("account-1");
    });
    expect(useMailStore.getState().activeFolderId).toBe("a1-inbox");
    expect(useUIStore.getState().activeView).toBe("inbox");
  });

  it("selects the merged overview inbox", async () => {
    useMailStore.setState({ activeAccountId: "account-2", activeFolderId: "a2-inbox" });

    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: "Overview" }));

    await waitFor(() => {
      expect(useMailStore.getState().activeAccountId).toBeNull();
    });
    expect(useMailStore.getState().activeFolderId).toBe("all:inbox");
    expect(useUIStore.getState().activeView).toBe("inbox");
  });

  it("collapses an account group via its header and persists the state", async () => {
    render(<Sidebar />);

    const header = screen.getByRole("button", { name: "user@example.com" });
    expect(header.getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(header);

    expect(screen.getByRole("button", { name: "user@example.com" }).getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("button", { name: "Archive" })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Inbox" })).toHaveLength(1);
    expect(JSON.parse(localStorage.getItem("pebble-sidebar-collapsed-accounts") ?? "[]")).toEqual(["account-1"]);

    fireEvent.click(screen.getByRole("button", { name: "user@example.com" }));

    expect(screen.getAllByRole("button", { name: "Inbox" })).toHaveLength(2);
    expect(JSON.parse(localStorage.getItem("pebble-sidebar-collapsed-accounts") ?? "[]")).toEqual([]);
  });

  it("uses per-folder unread counts for account entries and the merged count for overview", () => {
    useUIStore.setState({ showFolderUnreadCount: true });
    mocks.unreadCounts = { "a1-inbox": 5, "a2-inbox": 3 };

    render(<Sidebar />);

    expect(within(screen.getByRole("button", { name: /Overview/ })).getByText("8")).toBeTruthy();
    const inboxButtons = screen.getAllByRole("button", { name: /Inbox/ });
    expect(within(inboxButtons[0]).getByText("5")).toBeTruthy();
    expect(within(inboxButtons[1]).getByText("3")).toBeTruthy();
  });

  it("shows a placeholder for an account whose folders have not loaded", () => {
    mocks.folders = [];

    render(<Sidebar />);

    expect(within(mailNav()).getAllByText("No folders yet")).toHaveLength(2);
  });
});
