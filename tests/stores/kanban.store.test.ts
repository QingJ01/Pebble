import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { useKanbanStore } from "../../src/stores/kanban.store";

const mockedInvoke = vi.mocked(invoke);

describe("KanbanStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    useKanbanStore.setState({ cards: [], cardIdSet: new Set(), contextNotes: {}, loading: false });
  });

  it("fetchCards loads cards from backend", async () => {
    const mockCards = [
      { message_id: "m1", column: "todo", position: 0, created_at: 1000, updated_at: 1000 },
      { message_id: "m2", column: "done", position: 1, created_at: 1000, updated_at: 1000 },
    ];
    mockedInvoke.mockImplementation((command) => {
      if (command === "list_kanban_cards") {
        return Promise.resolve(mockCards);
      }
      if (command === "list_kanban_context_notes") {
        return Promise.resolve({});
      }
      return Promise.resolve(undefined);
    });

    await useKanbanStore.getState().fetchCards();

    expect(mockedInvoke).toHaveBeenCalledWith("list_kanban_cards", { column: undefined });
    expect(mockedInvoke).toHaveBeenCalledWith("list_kanban_context_notes");
    expect(useKanbanStore.getState().cards).toHaveLength(2);
    expect(useKanbanStore.getState().loading).toBe(false);
  });

  it("moveCard performs optimistic update", async () => {
    useKanbanStore.setState({
      cards: [
        { message_id: "m1", column: "todo", position: 0, created_at: 1000, updated_at: 1000 },
      ],
    });
    mockedInvoke.mockResolvedValueOnce(undefined);

    await useKanbanStore.getState().moveCard("m1", "done", 0);

    expect(useKanbanStore.getState().cards[0].column).toBe("done");
    expect(mockedInvoke).toHaveBeenCalledWith("move_to_kanban", { messageId: "m1", column: "done", position: 0 });
  });

  it("moveCard rolls back on error", async () => {
    useKanbanStore.setState({
      cards: [
        { message_id: "m1", column: "todo", position: 0, created_at: 1000, updated_at: 1000 },
      ],
    });
    mockedInvoke.mockRejectedValueOnce(new Error("fail"));

    await useKanbanStore.getState().moveCard("m1", "done", 0);

    expect(useKanbanStore.getState().cards[0].column).toBe("todo");
  });

  it("removeCard removes optimistically", async () => {
    useKanbanStore.setState({
      cards: [
        { message_id: "m1", column: "todo", position: 0, created_at: 1000, updated_at: 1000 },
      ],
    });
    mockedInvoke.mockResolvedValueOnce(undefined);

    await useKanbanStore.getState().removeCard("m1");

    expect(useKanbanStore.getState().cards).toHaveLength(0);
  });

  it("removeCard rolls back on error", async () => {
    useKanbanStore.setState({
      cards: [
        { message_id: "m1", column: "todo", position: 0, created_at: 1000, updated_at: 1000 },
      ],
    });
    mockedInvoke.mockRejectedValueOnce(new Error("fail"));

    await useKanbanStore.getState().removeCard("m1");

    expect(useKanbanStore.getState().cards).toHaveLength(1);
  });

  it("stores context notes through backend storage only", async () => {
    mockedInvoke.mockResolvedValueOnce({ m1: "follow up on the selected paragraph" });

    await useKanbanStore.getState().setContextNote("m1", "follow up on the selected paragraph");

    expect(useKanbanStore.getState().contextNotes.m1).toBe("follow up on the selected paragraph");
    expect(mockedInvoke).toHaveBeenCalledWith("set_kanban_context_note", {
      messageId: "m1",
      note: "follow up on the selected paragraph",
    });
    expect(localStorage.getItem("pebble-kanban-context-notes")).toBeNull();
  });
});
