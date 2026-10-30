import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Sidebar from "../../src/components/Sidebar";
import { useComposeStore } from "../../src/stores/compose.store";
import { useMailStore } from "../../src/stores/mail.store";
import { useUIStore } from "../../src/stores/ui.store";
import type { Folder } from "../../src/lib/api";

const mocks = vi.hoisted(() => ({
  accounts: [{
    id: "account-1",
    email: "user@example.com",
    display_name: "User",
    provider: "imap",
    color: null,
    created_at: 1,
    updated_at: 1,
  }],
  folders: [] as Folder[],
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
    data: mocks.accounts,
  }),
  useFoldersForAccountsQuery: () => ({
    data: mocks.folders,
    isFetched: true,
  }),
}));

vi.mock("../../src/hooks/queries/useFolderUnreadCounts", () => ({
  useFolderUnreadCountsForAccounts: () => ({ data: {} }),
}));

function folder(id: string, role: Folder["role"], sortOrder: number): Folder {
  return {
    id,
    account_id: "account-1",
    remote_id: id,
    name: id,
    folder_type: "folder",
    role,
    parent_id: null,
    color: null,
    is_system: true,
    sort_order: sortOrder,
  };
}

describe("Sidebar folder order", () => {
  beforeEach(() => {
    useUIStore.setState({
      sidebarCollapsed: false,
      activeView: "inbox",
      previousView: "inbox",
      showFolderUnreadCount: false,
      backgroundImage: null,
      sidebarStyle: "classic",
    });
    useMailStore.setState({
      activeAccountId: "account-1",
      activeFolderId: "inbox",
    });
    useComposeStore.setState({
      composeMode: null,
      composeReplyTo: null,
      composeDirty: false,
      showComposeLeaveConfirm: false,
      pendingView: null,
    });
    mocks.folders = [
      folder("drafts", "drafts", 1),
      folder("spam", "spam", 2),
      folder("inbox", "inbox", 3),
      folder("trash", "trash", 4),
      folder("archive", "archive", 5),
      folder("sent", "sent", 6),
    ];
    mocks.accounts = [{
      id: "account-1",
      email: "user@example.com",
      display_name: "User",
      provider: "imap",
      color: null,
      created_at: 1,
      updated_at: 1,
    }];
  });

  it("uses the same system folder order for a single account as all accounts", () => {
    render(<Sidebar />);

    const folderNav = screen.getByRole("navigation", { name: "Mail folders" });
    const labels = within(folderNav).getAllByRole("button").map((button) => button.textContent);

    expect(labels).toEqual([
      "Inbox",
      "Sent",
      "Archive",
      "Starred",
      "Drafts",
      "Trash",
      "Spam",
    ]);
  });

  it("uses a translucent account selector when an app background is active", () => {
    mocks.accounts = [
      {
        id: "account-1",
        email: "user@example.com",
        display_name: "User",
        provider: "imap",
        color: null,
        created_at: 1,
        updated_at: 1,
      },
      {
        id: "account-2",
        email: "second@example.com",
        display_name: "Second",
        provider: "imap",
        color: null,
        created_at: 2,
        updated_at: 2,
      },
    ];
    useMailStore.setState({ activeAccountId: null });
    useUIStore.setState({
      backgroundImage: {
        path: "/tmp/backgrounds/background.png",
        filename: "background.png",
        fit: "cover",
        opacity: 1,
        updatedAt: 1,
      },
    });

    render(<Sidebar />);

    const selector = screen.getByRole("combobox", { name: "Email Accounts" });
    const style = selector.getAttribute("style") ?? "";
    expect(style).toContain("color-mix(in srgb, var(--color-accent) 6%, transparent)");
    expect(style).not.toContain("background-color: var(--color-bg)");
  });
});
