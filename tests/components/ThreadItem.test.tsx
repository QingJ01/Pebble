import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ThreadSummary } from "../../src/lib/api";
import ThreadItem from "../../src/components/ThreadItem";


const mocks = vi.hoisted(() => ({
  invalidateQueries: vi.fn(),
  listThreadMessages: vi.fn(),
  batchMarkRead: vi.fn(),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: mocks.invalidateQueries }),
}));

vi.mock("../../src/lib/api", () => ({
  listThreadMessages: mocks.listThreadMessages,
  batchMarkRead: mocks.batchMarkRead,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
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
    mocks.listThreadMessages.mockResolvedValue([{ id: "m-1" }, { id: "m-2" }]);
    mocks.batchMarkRead.mockResolvedValue(2);
  });

  it("marks unread rows with a row class", () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 2 })}
        isSelected={false}
        onClick={vi.fn()}
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
      />,
    );

    expect(screen.getByRole("option").className).not.toContain("thread-list-row--unread");
  });

  it("keeps the participants line semibold regardless of read state", () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    const participantsLine = screen.getByText("Sender").parentElement;

    expect(participantsLine?.style.fontWeight).toBe("600");
  });

  it("renders read subjects regular and unread subjects semibold", () => {
    const { unmount } = render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByText("Thread subject").style.fontWeight).toBe("400");

    unmount();

    render(
      <ThreadItem
        thread={makeThread({ unread_count: 1 })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByText("Thread subject").style.fontWeight).toBe("600");
  });
  it("marks the whole thread unread from its context menu when fully read", async () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 0 })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 20, clientY: 30 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark thread as unread" }));

    await waitFor(() => expect(mocks.batchMarkRead).toHaveBeenCalledWith(["m-1", "m-2"], false));
    expect(mocks.listThreadMessages).toHaveBeenCalledWith("thread-1");
    expect(mocks.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["threads"] });
  });

  it("marks the whole thread read from its context menu when it has unread mail", async () => {
    render(
      <ThreadItem
        thread={makeThread({ unread_count: 2 })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 20, clientY: 30 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark thread as read" }));

    await waitFor(() => expect(mocks.batchMarkRead).toHaveBeenCalledWith(["m-1", "m-2"], true));
  });

});
