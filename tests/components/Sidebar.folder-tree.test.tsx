import { fireEvent, render, screen, within } from "@testing-library/react";
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
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, fallback?: string | { name?: string; defaultValue?: string }) => {
      const labels: Record<string, string> = {
        "sidebar.mailFolders": "Mail folders",
        "sidebar.inbox": "Inbox",
        "sidebar.sent": "Sent",
        "sidebar.trash": "Trash",
      };
      if (labels[key]) return labels[key];
      if (typeof fallback === "object") {
        return (fallback.defaultValue ?? key).replace("{{name}}", fallback.name ?? "");
      }
      return fallback ?? key;
    },
  }),
}));

vi.mock("../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({ data: mocks.accounts }),
  useFoldersForAccountsQuery: () => ({ data: mocks.folders, isFetched: true }),
}));

vi.mock("../../src/hooks/queries/useFolderUnreadCounts", () => ({
  useFolderUnreadCountsForAccounts: () => ({ data: mocks.unreadCounts }),
}));

function folder(id: string, name: string, extra: Partial<Folder> = {}): Folder {
  return {
    id,
    account_id: "acc",
    remote_id: name,
    name,
    folder_type: "folder",
    role: null,
    parent_id: null,
    color: null,
    is_system: false,
    sort_order: 0,
    ...extra,
  };
}

function mailNav() {
  return screen.getByRole("navigation", { name: "Mail folders" });
}

/** The folder button whose visible name is exactly `label` */
function folderButton(label: string): HTMLElement {
  const match = within(mailNav())
    .getAllByRole("button")
    .find((b) => b.querySelector(".sidebar-btn__label")?.textContent === label);
  if (!match) throw new Error(`No folder button labelled ${label}`);
  return match;
}

function visibleLabels() {
  return within(mailNav())
    .getAllByRole("button")
    .map((b) => b.textContent?.trim())
    .filter(Boolean);
}

describe("Sidebar folder tree", () => {
  beforeEach(() => {
    localStorage.clear();
    useUIStore.setState({
      sidebarCollapsed: false,
      activeView: "inbox",
      previousView: "inbox",
      showFolderUnreadCount: true,
      backgroundImage: null,
      sidebarStyle: "grouped",
    });
    useMailStore.setState({ activeAccountId: "acc", activeFolderId: "inbox" });
    useComposeStore.setState({ composeDirty: false, showComposeLeaveConfirm: false });
    mocks.accounts = [{
      id: "acc", email: "me@example.com", display_name: "Work", provider: "imap",
      color: null, created_at: 1, updated_at: 1,
    } as Account];
    mocks.folders = [
      folder("inbox", "INBOX", { role: "inbox", is_system: true }),
      folder("sent", "[Gmail]/Sent Mail", { role: "sent", parent_id: "[Gmail]", is_system: true }),
      folder("clients", "Clients"),
      folder("amy", "Clients/Amy & Todd", { parent_id: "clients" }),
      folder("mara", "Clients/Mara Ream", { parent_id: "clients" }),
      folder("colleen", "Clients/Mara Ream/Colleen Erickson", { parent_id: "mara" }),
      folder("all-mail", "[Gmail]/All Mail", { parent_id: "[Gmail]", is_system: true }),
    ];
    mocks.unreadCounts = { amy: 2, colleen: 3, clients: 1 };
  });

  it("starts with sub-folders collapsed and shows short names", () => {
    render(<Sidebar />);
    const labels = visibleLabels();
    expect(labels).toContain("Inbox");
    expect(labels).toContain("Sent");
    expect(labels.some((l) => l?.startsWith("Clients"))).toBe(true);
    expect(labels.some((l) => l?.startsWith("Gmail"))).toBe(true);
    expect(labels.join("|")).not.toContain("Amy & Todd");
    expect(labels.join("|")).not.toContain("[Gmail]");
  });

  it("shows the total unread of a collapsed folder and its sub-folders", () => {
    render(<Sidebar />);
    const clients = folderButton("Clients");
    expect(clients.textContent).toBe("Clients6");
  });

  it("expands with the arrow and remembers it", () => {
    const { unmount } = render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Expand Clients" }));
    expect(visibleLabels()).toEqual(expect.arrayContaining(["Amy & Todd2", "Mara Ream3"]));
    expect(folderButton("Clients").textContent).toBe("Clients1");

    unmount();
    render(<Sidebar />);
    expect(visibleLabels()).toContain("Amy & Todd2");
  });

  it("opens a parent folder when its name is clicked, without toggling it", () => {
    render(<Sidebar />);
    fireEvent.click(folderButton("Clients"));
    expect(useMailStore.getState().activeFolderId).toBe("clients");
    expect(screen.getByRole("button", { name: "Expand Clients" })).toBeTruthy();
  });

  it("shows the full path when hovering a nested folder", () => {
    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Expand Clients" }));
    expect(within(mailNav()).getByTitle("Clients/Amy & Todd").textContent).toBe("Amy & Todd2");
  });

  it("expands the parents of the open folder automatically", () => {
    useMailStore.setState({ activeAccountId: "acc", activeFolderId: "colleen" });
    render(<Sidebar />);
    const active = within(mailNav()).getByTitle("Clients/Mara Ream/Colleen Erickson");
    expect(active.getAttribute("aria-current")).toBe("page");
  });

  it("toggles the virtual Gmail group by clicking its name", () => {
    render(<Sidebar />);
    expect(visibleLabels()).not.toContain("All Mail");
    fireEvent.click(folderButton("Gmail"));
    expect(visibleLabels()).toContain("All Mail");
    expect(useMailStore.getState().activeFolderId).toBe("inbox");
  });
});
