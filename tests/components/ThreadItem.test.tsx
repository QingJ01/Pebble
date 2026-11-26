import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ThreadSummary } from "../../src/lib/api";
import ThreadItem from "../../src/components/ThreadItem";
import { useToastStore } from "../../src/stores/toast.store";


const mocks = vi.hoisted(() => ({
  invalidateQueries: vi.fn(),
  updateThreadReadFlagsInFolders: vi.fn(),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: mocks.invalidateQueries }),
}));

vi.mock("../../src/lib/api", () => ({
  updateThreadReadFlagsInFolders: mocks.updateThreadReadFlagsInFolders,
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallbackOrOptions?: string | Record<string, unknown>) =>
      typeof fallbackOrOptions === "string" ? fallbackOrOptions : key,
  }),
}));

function makeThread(overrides: Partial<ThreadSummary> = {}): ThreadSummary {
  return {
    thread_id: "thread-1",
    subject: "Thread subject",
    snippet: "Thread snippet",
    last_date: 1_700_000_000,
    message_count: 2,
    unread_count: 0,
    is_starred: false,
    participants: ["Sender"],
    has_attachments: false,
    ...overrides,
  };
}

describe("ThreadItem", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.updateThreadReadFlagsInFolders.mockResolvedValue({ updated: 2, queued: 0, failed: 0 });
    useToastStore.setState({ toasts: [] });
  });

  it("shows an account badge before the date when given an account", () => {
    render(
      <ThreadItem
        thread={makeThread({ account_id: "personal" })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
        account={{ id: "personal", email: "me@example.com", display_name: "Me", color: "#8b5cf6", account_label: null }}
      />,
    );

    const badge = screen.getByRole("img", { name: "me@example.com" });
    expect(badge.getAttribute("title")).toBe("me@example.com");
    expect(badge.textContent).toBe("M");
    expect(badge.nextElementSibling?.className).toContain("mail-row__date");
  });

  it("shows no account badge by default", () => {
    const { container } = render(
      <ThreadItem thread={makeThread()} isSelected={false} onClick={vi.fn()} folderIds={["folder-1"]} />,
    );

    expect(container.querySelector(".account-badge")).toBeNull();
  });

  it("marks unread rows with a row class", () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 2 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    expect(screen.getByRole("option").className).toContain("thread-list-row--unread");
  });

  it("does not add unread row treatment when every thread message is read", () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    expect(screen.getByRole("option").className).not.toContain("thread-list-row--unread");
  });

  it("styles participants and subject through the unread row class (regular when read)", () => {
    const { unmount } = render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    expect(screen.getByText("Sender").closest(".mail-row__primary")).toBeTruthy();
    expect(screen.getByText("Thread subject").className).toContain("mail-row__subject");
    expect(screen.getByRole("option").className).not.toContain("thread-list-row--unread");
    expect(screen.getByText("Thread subject").getAttribute("title")).toBe("Thread subject");

    unmount();

    render(
      <ThreadItem
        thread={makeThread({ unread_count: 1 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    expect(screen.getByRole("option").className).toContain("thread-list-row--unread");
  });

  it("does not open the thread when Enter is pressed on a control inside the row", () => {
    const onClick = vi.fn();
    render(
      <ThreadItem
        thread={makeThread()}
        isSelected={false}
        onClick={onClick}
        folderIds={["folder-1"]}
      />,
    );
    const row = screen.getByRole("option");
    const inner = document.createElement("button");
    row.appendChild(inner);

    fireEvent.keyDown(inner, { key: "Enter" });
    expect(onClick).not.toHaveBeenCalled();

    fireEvent.keyDown(row, { key: "Enter" });
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("opens the context menu from the keyboard with Shift+F10", () => {
    render(
      <ThreadItem
        thread={makeThread()}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    fireEvent.keyDown(screen.getByRole("option"), { key: "F10", shiftKey: true });

    const menu = screen.getByRole("menu");
    expect(menu).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Mark thread as unread" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Archive" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeTruthy();
  });

  it("marks the whole thread unread from its context menu when fully read", async () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 20, clientY: 30 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark thread as unread" }));

    await waitFor(() => expect(mocks.updateThreadReadFlagsInFolders).toHaveBeenCalledWith("thread-1", ["folder-1"], false));
    expect(mocks.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["threads"] });
  });

  it("marks the whole thread read from its context menu when it has unread mail", async () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 2 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 20, clientY: 30 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark thread as read" }));

    await waitFor(() => expect(mocks.updateThreadReadFlagsInFolders).toHaveBeenCalledWith("thread-1", ["folder-1"], true));
  });

  it("shows an error when the scoped thread update fails for some messages", async () => {
    mocks.updateThreadReadFlagsInFolders.mockResolvedValue({ updated: 1, queued: 0, failed: 1 });
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 20, clientY: 30 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark thread as unread" }));

    await waitFor(() => expect(useToastStore.getState().toasts).toEqual([
      expect.objectContaining({ type: "error", message: "messageActions.threadReadFailed" }),
    ]));
  });

  it("shows an error when no scoped thread messages were updated or queued", async () => {
    mocks.updateThreadReadFlagsInFolders.mockResolvedValue({ updated: 0, queued: 0, failed: 0 });
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 20, clientY: 30 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark thread as unread" }));

    await waitFor(() => expect(useToastStore.getState().toasts).toEqual([
      expect.objectContaining({ type: "error", message: "messageActions.threadReadNoMessages" }),
    ]));
  });

  it("tells the user when scoped thread updates are queued for retry", async () => {
    mocks.updateThreadReadFlagsInFolders.mockResolvedValue({ updated: 0, queued: 2, failed: 0 });
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
        folderIds={["folder-1"]}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 20, clientY: 30 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark thread as unread" }));

    await waitFor(() => expect(useToastStore.getState().toasts).toEqual([
      expect.objectContaining({ type: "info", message: "messageActions.threadReadQueued" }),
    ]));
  });

});
