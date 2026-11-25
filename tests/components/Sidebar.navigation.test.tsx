import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClientProvider } from "@tanstack/react-query";
import Sidebar from "../../src/components/Sidebar";
import { queryClient } from "../../src/lib/query-client";
import { useComposeStore } from "../../src/stores/compose.store";
import { useConfirmStore } from "../../src/stores/confirm.store";
import { useMailStore } from "../../src/stores/mail.store";
import { useUIStore } from "../../src/stores/ui.store";

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
        "sidebar.tools": "Tools",
        "sidebar.inbox": "Inbox",
        "sidebar.outbox": "发件箱",
        "sidebar.sent": "Sent",
        "sidebar.drafts": "Drafts",
        "sidebar.trash": "Trash",
        "sidebar.archive": "Archive",
        "sidebar.spam": "Spam",
        "sidebar.starred": "Starred",
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
    data: [
      {
        id: "account-1",
        email: "user@example.com",
        display_name: "User",
        provider: "imap",
        created_at: 1,
        updated_at: 1,
      },
    ],
  }),
  useFoldersForAccountsQuery: () => ({
    data: [
      {
        id: "folder-inbox",
        account_id: "account-1",
        remote_id: "INBOX",
        name: "Inbox",
        folder_type: "folder",
        role: "inbox",
        parent_id: null,
        color: null,
        is_system: true,
        sort_order: 0,
      },
      { id: "outbox", account_id: "account-1", remote_id: "__local_outbox__", name: "Outbox",
        folder_type: "folder", role: null, parent_id: null, color: null, is_system: true, sort_order: 6 },
    ],
    isFetched: true,
  }),
}));

vi.mock("../../src/hooks/queries/useFolderUnreadCounts", () => ({
  useFolderUnreadCountsForAccounts: () => ({ data: {} }),
}));

describe("Sidebar navigation", () => {
  it.each(["classic", "grouped"] as const)("translates local Outbox in %s mode", (sidebarStyle) => {
    useUIStore.setState({ sidebarStyle });
    render(<Sidebar />);
    expect(screen.getByText("发件箱")).toBeTruthy();
    expect(screen.queryByText("Outbox")).toBeNull();
  });
  beforeEach(() => {
    useUIStore.setState({
      sidebarCollapsed: false,
      activeView: "compose",
      previousView: "inbox",
      showFolderUnreadCount: false,
      sidebarStyle: "grouped",
    });
    useMailStore.setState({
      activeAccountId: "account-1",
      activeFolderId: "folder-inbox",
    });
    useComposeStore.setState({
      composeMode: "new",
      composeReplyTo: null,
      composeDirty: true,
      showComposeLeaveConfirm: false,
      pendingView: null,
    });
    useConfirmStore.setState({
      confirm: vi.fn().mockResolvedValue(true),
    });
  });

  it.each([
    ["Contacts", "contacts"],
    ["Snoozed", "snoozed"],
    ["Kanban", "kanban"],
    ["Settings", "settings"],
  ] as const)("switches to the %s view from the bottom navigation", async (label, view) => {
    useUIStore.setState({ activeView: "inbox" });
    useComposeStore.setState({ composeDirty: false, composeMode: null });

    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: label }));

    await waitFor(() => {
      expect(useUIStore.getState().activeView).toBe(view);
    });
  });

  it.each(["classic", "grouped"] as const)("opens folder management from blank-area context menu in %s mode", (sidebarStyle) => {
    useUIStore.setState({ sidebarStyle, activeView: "inbox" });
    useComposeStore.setState({ composeDirty: false, composeMode: null });
    render(<Sidebar />);

    expect(screen.queryByRole("menuitem", { name: "Manage folders" })).toBeNull();
    expect(document.querySelector(".folder-action-button")).toBeNull();

    fireEvent.contextMenu(screen.getByLabelText("Sidebar"), { clientX: 32, clientY: 500 });

    const menuItem = screen.getByRole("menuitem", { name: "Manage folders" });
    expect(menuItem).toBeTruthy();
    expect(document.activeElement).toBe(menuItem);
  });

  it("does not open folder management when right-clicking a sidebar button icon", () => {
    useUIStore.setState({ sidebarStyle: "grouped", activeView: "inbox" });
    useComposeStore.setState({ composeDirty: false, composeMode: null });
    render(<Sidebar />);

    const overview = screen.getByRole("button", { name: /Overview|sidebar.overview/ });
    const icon = overview.querySelector("svg");
    expect(icon).toBeTruthy();
    fireEvent.contextMenu(icon!, { clientX: 24, clientY: 100 });

    expect(screen.queryByRole("menuitem", { name: "Manage folders" })).toBeNull();
  });

  it("keeps sidebar key events on the keyboard-only folder management entry", () => {
    useUIStore.setState({ sidebarStyle: "grouped", activeView: "inbox" });
    useComposeStore.setState({ composeDirty: false, composeMode: null });
    render(<Sidebar />);

    const listener = vi.fn();
    document.addEventListener("keydown", listener);
    try {
      const keyboardButton = document.querySelector<HTMLButtonElement>(".sidebar-keyboard-only-action");
      expect(keyboardButton).toBeTruthy();
      fireEvent.keyDown(keyboardButton!, { key: "Enter" });
      expect(listener).not.toHaveBeenCalled();
      fireEvent.keyDown(keyboardButton!, { key: "k", ctrlKey: true });
      expect(listener).toHaveBeenCalledTimes(1);
    } finally {
      document.removeEventListener("keydown", listener);
    }
  });

  it("keeps a keyboard-only folder management entry and restores focus to it", async () => {
    useUIStore.setState({ sidebarStyle: "grouped", activeView: "inbox" });
    useComposeStore.setState({ composeDirty: false, composeMode: null });
    render(
      <QueryClientProvider client={queryClient}>
        <Sidebar />
      </QueryClientProvider>,
    );

    const keyboardButton = document.querySelector<HTMLButtonElement>(".sidebar-keyboard-only-action");
    expect(keyboardButton).toBeTruthy();
    keyboardButton!.focus();
    fireEvent.click(keyboardButton!);

    expect(await screen.findByRole("dialog", { name: "Manage folders" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Manage folders" })).toBeNull());
    expect(document.activeElement).toBe(keyboardButton);
  });

  it("keeps the sidebar from shrinking under wide message content", () => {
    render(<Sidebar />);

    const sidebar = screen.getByLabelText("Sidebar");

    expect(sidebar.style.flexShrink).toBe("0");
  });

  it("uses non-submit buttons for bottom navigation actions", () => {
    render(<Sidebar />);

    expect(screen.getByRole("button", { name: "Snoozed" }).getAttribute("type")).toBe("button");
    expect(screen.getByRole("button", { name: "Contacts" }).getAttribute("type")).toBe("button");
    expect(screen.getByRole("button", { name: "Kanban" }).getAttribute("type")).toBe("button");
    expect(screen.getByRole("button", { name: "Settings" }).getAttribute("type")).toBe("button");
  });

  it("defers dirty compose navigation to the compose save/discard dialog", async () => {
    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(useUIStore.getState().activeView).toBe("compose");
    expect(useComposeStore.getState().showComposeLeaveConfirm).toBe(true);
    expect(useComposeStore.getState().pendingView).toBe("settings");
    act(() => useComposeStore.getState().confirmCloseCompose());

    await waitFor(() => {
      expect(useUIStore.getState().activeView).toBe("settings");
    });
    expect(useComposeStore.getState().composeDirty).toBe(false);
    expect(useComposeStore.getState().showComposeLeaveConfirm).toBe(false);
    expect(useComposeStore.getState().pendingView).toBe(null);
  });
});
