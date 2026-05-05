import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Sidebar from "../../src/components/Sidebar";
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
        "sidebar.sent": "Sent",
        "sidebar.drafts": "Drafts",
        "sidebar.trash": "Trash",
        "sidebar.archive": "Archive",
        "sidebar.spam": "Spam",
        "sidebar.starred": "Starred",
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
    ],
    isFetched: true,
  }),
}));

vi.mock("../../src/hooks/queries/useFolderUnreadCounts", () => ({
  useFolderUnreadCountsForAccounts: () => ({ data: {} }),
}));

describe("Sidebar navigation", () => {
  beforeEach(() => {
    useUIStore.setState({
      sidebarCollapsed: false,
      activeView: "compose",
      previousView: "inbox",
      showFolderUnreadCount: false,
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

  it("keeps the sidebar from shrinking under wide message content", () => {
    render(<Sidebar />);

    const sidebar = screen.getByLabelText("Sidebar");

    expect(sidebar.style.flexShrink).toBe("0");
  });

  it("uses non-submit buttons for bottom navigation actions", () => {
    render(<Sidebar />);

    expect(screen.getByRole("button", { name: "Snoozed" }).getAttribute("type")).toBe("button");
    expect(screen.getByRole("button", { name: "Kanban" }).getAttribute("type")).toBe("button");
    expect(screen.getByRole("button", { name: "Settings" }).getAttribute("type")).toBe("button");
  });

  it("leaves a dirty compose draft after confirming sidebar navigation", async () => {
    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    await waitFor(() => {
      expect(useUIStore.getState().activeView).toBe("settings");
    });
    expect(useComposeStore.getState().composeDirty).toBe(false);
    expect(useComposeStore.getState().showComposeLeaveConfirm).toBe(false);
    expect(useComposeStore.getState().pendingView).toBe(null);
  });
});
