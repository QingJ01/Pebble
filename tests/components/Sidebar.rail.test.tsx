import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, fallback?: string | Record<string, unknown>, options?: Record<string, unknown>) => {
      const labels: Record<string, string> = {
        "search.title": "Search",
        "sidebar.navigation": "Sidebar",
        "sidebar.mail": "Mail",
        "sidebar.mailFolders": "Mail folders",
        "sidebar.accounts": "Accounts",
        "sidebar.overview": "Overview",
        "sidebar.inbox": "Inbox",
        "sidebar.sent": "Sent",
        "sidebar.drafts": "Drafts",
        "sidebar.trash": "Trash",
        "sidebar.archive": "Archive",
        "sidebar.spam": "Spam",
        "sidebar.starred": "Starred",
        "sidebar.tools": "Tools",
        "sidebar.contacts": "Contacts",
        "sidebar.snoozed": "Snoozed",
        "sidebar.kanban": "Kanban",
        "sidebar.settings": "Settings",
        "sidebar.accountUnread": "{{name}}, {{count}} unread",
        "sidebar.switchAccount": "Switch account",
        "sidebar.switchAccountCurrent": "Switch account (current: {{name}})",
        "sidebar.moreFolders": "More folders",
        "folderManagement.manage": "Manage folders",
      };
      const template = labels[key] ?? (typeof fallback === "string" ? fallback : key);
      const vars = (typeof fallback === "object" ? fallback : options) ?? {};
      return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(vars[name] ?? ""));
    },
  }),
}));

vi.mock("../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({ data: mocks.accounts }),
  useFoldersForAccountsQuery: (accountIds: string[]) => ({
    data: mocks.folders.filter((f) => accountIds.includes(f.account_id)),
    isFetched: true,
  }),
}));

vi.mock("../../src/hooks/queries/useFolderUnreadCounts", () => ({
  useFolderUnreadCountsForAccounts: () => ({ data: mocks.unreadCounts }),
}));

function account(id: string, email: string): Account {
  return { id, email, display_name: email, provider: "imap", color: null, created_at: 1, updated_at: 1 };
}

function folder(id: string, accountId: string, role: Folder["role"], name: string = role ?? id): Folder {
  return {
    id, account_id: accountId, remote_id: id, name, folder_type: "folder", role,
    parent_id: null, color: null, is_system: role != null, sort_order: 0,
  };
}

function accountsNav() {
  return screen.getByRole("navigation", { name: "Accounts" });
}

describe("Collapsed sidebar rail", () => {
  beforeEach(() => {
    localStorage.clear();
    useUIStore.setState({
      sidebarCollapsed: true,
      activeView: "inbox",
      previousView: "inbox",
      showFolderUnreadCount: true,
      backgroundImage: null,
      sidebarStyle: "grouped",
    });
    useMailStore.setState({ activeAccountId: "account-1", activeFolderId: "a1-inbox" });
    useComposeStore.setState({ composeDirty: false, showComposeLeaveConfirm: false, pendingView: null });
    mocks.accounts = [
      account("account-1", "alice@example.com"),
      account("account-2", "bob@example.com"),
      account("account-3", "carol@example.com"),
    ];
    mocks.folders = [
      folder("a1-inbox", "account-1", "inbox"),
      folder("a1-sent", "account-1", "sent"),
      folder("a1-archive", "account-1", "archive"),
      folder("a1-projects", "account-1", null, "Projects"),
      folder("a2-inbox", "account-2", "inbox"),
      folder("a2-sent", "account-2", "sent"),
      folder("a3-inbox", "account-3", "inbox"),
    ];
    mocks.unreadCounts = { "a2-inbox": 4 };
  });

  it("renders one badge per account and no per-account folder icons", () => {
    render(<Sidebar />);

    const accountButtons = within(accountsNav()).getAllByRole("button");
    expect(accountButtons).toHaveLength(3);
    expect(accountButtons.every((button) => button.getAttribute("aria-haspopup") === "menu")).toBe(true);
    expect(accountButtons.map((button) => button.textContent)).toEqual(["A", "B", "C"]);
    // No folder rows anywhere on the rail until a menu is opened.
    expect(screen.queryByRole("button", { name: /Inbox|Sent|Archive|Projects/ })).toBeNull();

    // Top and bottom groups.
    const top = screen.getByRole("navigation", { name: "Mail" });
    expect(within(top).getAllByRole("button").map((b) => b.getAttribute("aria-label")))
      .toEqual(["Search", "Overview", "Starred"]);
    const bottom = screen.getByRole("navigation", { name: "Tools" });
    expect(within(bottom).getAllByRole("button").map((b) => b.getAttribute("aria-label")))
      .toEqual(["Contacts", "Snoozed", "Kanban", "Settings"]);
  });

  it("marks the current account and accounts with unread inbox mail", () => {
    render(<Sidebar />);

    const alice = screen.getByRole("button", { name: "alice@example.com" });
    expect(alice.querySelector(".account-badge.is-selected")).toBeTruthy();
    expect(alice.querySelector(".account-badge__dot")).toBeNull();

    const bob = screen.getByRole("button", { name: "bob@example.com, 4 unread" });
    expect(bob.querySelector(".account-badge.is-selected")).toBeNull();
    expect(bob.querySelector(".account-badge__dot")).toBeTruthy();
    expect(bob.getAttribute("title")).toBe("bob@example.com, 4 unread");
  });

  it("hides unread dots and counts when folder unread counts are turned off", () => {
    // The unified-inbox filter bar may still have counts cached.
    useUIStore.setState({ showFolderUnreadCount: false });
    render(<Sidebar />);

    const bob = screen.getByRole("button", { name: "bob@example.com" });
    expect(bob.querySelector(".account-badge__dot")).toBeNull();
  });

  it("opens the account menu with the keyboard, moves with arrows and returns focus on Escape", async () => {
    render(<Sidebar />);

    const alice = screen.getByRole("button", { name: "alice@example.com" });
    alice.focus();
    fireEvent.keyDown(alice, { key: "ArrowRight" });

    const menu = await screen.findByRole("menu", { name: "alice@example.com" });
    expect(alice.getAttribute("aria-expanded")).toBe("true");
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((item) => item.querySelector(".sidebar-rail-menu__label")?.textContent))
      .toEqual(["Inbox", "Sent", "Archive", "Projects", "Manage folders"]);
    // The current folder is marked and receives focus.
    const inbox = within(menu).getByRole("menuitem", { name: "Inbox" });
    expect(inbox.getAttribute("aria-current")).toBe("page");
    await waitFor(() => expect(document.activeElement).toBe(inbox));

    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(within(menu).getByRole("menuitem", { name: "Sent" }));

    fireEvent.keyDown(document.activeElement as HTMLElement, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    expect(document.activeElement).toBe(alice);
    expect(alice.getAttribute("aria-expanded")).toBe("false");
  });

  it("selects a folder from the account menu", async () => {
    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: "bob@example.com, 4 unread" }));
    const menu = await screen.findByRole("menu", { name: "bob@example.com" });
    expect(within(within(menu).getByRole("menuitem", { name: /Inbox/ })).getByText("4")).toBeTruthy();
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Sent" }));

    await waitFor(() => {
      expect(useMailStore.getState().activeAccountId).toBe("account-2");
      expect(useMailStore.getState().activeFolderId).toBe("a2-sent");
    });
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("classic mode shows only the current account's main folders", async () => {
    useUIStore.setState({ sidebarStyle: "classic" });
    render(<Sidebar />);

    const nav = screen.getByRole("navigation", { name: "Mail folders" });
    const labels = within(nav).getAllByRole("button").map((b) => b.getAttribute("aria-label"));
    expect(labels).toEqual([
      "Switch account (current: alice@example.com)",
      "Inbox",
      "Sent",
      "Archive",
      "More folders",
    ]);
    expect(screen.getAllByRole("button", { name: "Inbox" })).toHaveLength(1);

    // The switcher lists every account; picking one swaps the folder set.
    fireEvent.click(screen.getByRole("button", { name: /Switch account/ }));
    const menu = await screen.findByRole("menu", { name: "Switch account" });
    fireEvent.click(within(menu).getByRole("menuitem", { name: "bob@example.com" }));

    await waitFor(() => expect(useMailStore.getState().activeAccountId).toBe("account-2"));
    await waitFor(() => {
      const next = within(screen.getByRole("navigation", { name: "Mail folders" }))
        .getAllByRole("button").map((b) => b.getAttribute("aria-label"));
      expect(next).toEqual(["Switch account (current: bob@example.com)", "Inbox", "Sent", "More folders"]);
    });
  });
  it("classic Overview always opens the all-accounts inbox", () => {
    useUIStore.setState({ sidebarStyle: "classic" });
    useMailStore.setState({ activeAccountId: null, activeFolderId: "all:sent" });
    render(<Sidebar />);

    const overview = screen.getByRole("button", { name: "Overview" });
    expect(overview.getAttribute("aria-current")).toBeNull();
    fireEvent.click(overview);

    expect(useMailStore.getState().activeAccountId).toBeNull();
    expect(useMailStore.getState().activeFolderId).toBe("all:inbox");
  });
  describe("narrow window with the sidebar expanded", () => {
    beforeEach(() => {
      vi.stubGlobal("matchMedia", (query: string) => ({
        matches: true, media: query, addEventListener: () => {}, removeEventListener: () => {},
      }));
      useUIStore.setState({ sidebarNarrowExpanded: true });
    });
    afterEach(() => {
      vi.unstubAllGlobals();
      useUIStore.setState({ sidebarNarrowExpanded: false });
    });

    it("floats the full sidebar over the content and closes on an outside click", () => {
      const { container } = render(<Sidebar />);
      const drawer = container.querySelector(".sidebar-drawer");
      expect(drawer).not.toBeNull();
      expect(drawer!.querySelector(".app-sidebar--rail")).toBeNull();

      fireEvent.mouseDown(container.querySelector(".sidebar-drawer-backdrop")!);
      expect(useUIStore.getState().sidebarNarrowExpanded).toBe(false);
    });

    it("moves focus into the drawer when it opens", () => {
      const { container } = render(<Sidebar />);
      const drawer = container.querySelector(".sidebar-drawer")!;
      expect(drawer.contains(document.activeElement)).toBe(true);
    });

    it("closes after navigating to another folder", () => {
      render(<Sidebar />);
      act(() => { useMailStore.setState({ activeFolderId: "a1-sent" }); });
      expect(useUIStore.getState().sidebarNarrowExpanded).toBe(false);
    });
  });
});
