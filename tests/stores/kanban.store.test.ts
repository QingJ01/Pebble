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
    useKanbanStore.setState({ cards: [], loading: false });
  });

  it("fetchCards loads cards from backend", async () => {
    const mockCards = [
      { message_id: "m1", column: "todo", position: 0, created_at: 1000, updated_at: 1000 },
      { message_id: "m2", column: "done", position: 1, created_at: 1000, updated_at: 1000 },
    ];
    mockedInvoke.mockResolvedValueOnce(mockCards);

    await useKanbanStore.getState().fetchCards();

    expect(mockedInvoke).toHaveBeenCalledWith("list_kanban_cards", { column: undefined });
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
    expect(mockedInvoke).toHaveBeenCalledWith("move_to_kanban", { message_id: "m1", column: "done" });
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
});
