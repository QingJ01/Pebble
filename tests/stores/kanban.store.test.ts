import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { useKanbanStore } from "../../src/stores/kanban.store";

const mockedInvoke = vi.mocked(invoke);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("KanbanStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    useKanbanStore.setState({ contextNotes: {} });
  });

  it("fetchContextNotes loads notes only; cards live in react-query", async () => {
    mockedInvoke.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") {
        return Promise.resolve({ m1: "note" });
      }
      return Promise.resolve(undefined);
    });

    await useKanbanStore.getState().fetchContextNotes();

    expect(mockedInvoke).toHaveBeenCalledWith("list_kanban_context_notes");
    expect(mockedInvoke).not.toHaveBeenCalledWith("list_kanban_cards", expect.anything());
    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "note" });
    expect("cards" in useKanbanStore.getState()).toBe(false);
  });

  it("does not let an older fetch overwrite a newer fetch", async () => {
    const firstNotes = deferred<Record<string, string>>();
    const secondNotes = deferred<Record<string, string>>();
    let noteRequest = 0;
    mockedInvoke.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") {
        noteRequest += 1;
        return noteRequest === 1 ? firstNotes.promise : secondNotes.promise;
      }
      return Promise.resolve(undefined);
    });

    const olderFetch = useKanbanStore.getState().fetchContextNotes();
    const newerFetch = useKanbanStore.getState().fetchContextNotes();
    secondNotes.resolve({ new: "new note" });
    await newerFetch;

    firstNotes.resolve({ old: "old note" });
    await olderFetch;

    expect(useKanbanStore.getState().contextNotes).toEqual({ new: "new note" });
  });

  it("preserves a context note edited while a fetch is in flight", async () => {
    const fetchedNotes = deferred<Record<string, string>>();
    const savedNote = deferred<Record<string, string>>();
    mockedInvoke.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") return fetchedNotes.promise;
      if (command === "set_kanban_context_note") return savedNote.promise;
      return Promise.resolve(undefined);
    });

    const fetch = useKanbanStore.getState().fetchContextNotes();
    const save = useKanbanStore.getState().setContextNote("m1", "new note");
    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "new note" });

    fetchedNotes.resolve({ m1: "stale note" });
    await fetch;
    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "new note" });

    savedNote.resolve({ m1: "new note" });
    await save;
    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "new note" });
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

  it("merges out-of-order context-note responses without dropping another optimistic update", async () => {
    const first = deferred<Record<string, string>>();
    const second = deferred<Record<string, string>>();
    mockedInvoke
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const firstUpdate = useKanbanStore.getState().setContextNote("m1", "one");
    const secondUpdate = useKanbanStore.getState().setContextNote("m2", "two");
    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "one", m2: "two" });

    second.resolve({ m2: "two" });
    await secondUpdate;
    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "one", m2: "two" });

    first.resolve({ m1: "one" });
    await firstUpdate;
    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "one", m2: "two" });
  });

  it("ignores a stale response for a newer edit of the same context note", async () => {
    const first = deferred<Record<string, string>>();
    const second = deferred<Record<string, string>>();
    mockedInvoke
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const firstUpdate = useKanbanStore.getState().setContextNote("m1", "old");
    const secondUpdate = useKanbanStore.getState().setContextNote("m1", "new");
    second.resolve({ m1: "new" });
    await secondUpdate;
    first.resolve({ m1: "old" });
    await firstUpdate;

    expect(useKanbanStore.getState().contextNotes).toEqual({ m1: "new" });
  });

  it("keeps legacy context notes until the backend acknowledges migration", async () => {
    vi.resetModules();
    localStorage.setItem("pebble-kanban-context-notes", JSON.stringify({ m1: "legacy note" }));
    const { invoke: isolatedInvoke } = await import("@tauri-apps/api/core");
    const isolatedInvokeMock = vi.mocked(isolatedInvoke);
    const { useKanbanStore: isolatedStore } = await import("../../src/stores/kanban.store");

    expect(localStorage.getItem("pebble-kanban-context-notes"))
      .toBe(JSON.stringify({ m1: "legacy note" }));
    isolatedInvokeMock.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") return Promise.resolve({});
      if (command === "merge_kanban_context_notes") return Promise.resolve({ m1: "legacy note" });
      return Promise.resolve(undefined);
    });

    await isolatedStore.getState().fetchContextNotes();

    expect(isolatedInvokeMock).toHaveBeenCalledWith("merge_kanban_context_notes", {
      notes: { m1: "legacy note" },
    });
    expect(isolatedStore.getState().contextNotes).toEqual({ m1: "legacy note" });
    expect(localStorage.getItem("pebble-kanban-context-notes")).toBeNull();
  });

  it("retains legacy context notes when backend migration fails", async () => {
    vi.resetModules();
    const storedNotes = JSON.stringify({ m1: "legacy note" });
    localStorage.setItem("pebble-kanban-context-notes", storedNotes);
    const { invoke: isolatedInvoke } = await import("@tauri-apps/api/core");
    const isolatedInvokeMock = vi.mocked(isolatedInvoke);
    const { useKanbanStore: isolatedStore } = await import("../../src/stores/kanban.store");
    isolatedInvokeMock.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") return Promise.resolve({});
      if (command === "merge_kanban_context_notes") return Promise.reject(new Error("write failed"));
      return Promise.resolve(undefined);
    });

    await expect(isolatedStore.getState().fetchContextNotes()).rejects.toThrow("write failed");

    expect(localStorage.getItem("pebble-kanban-context-notes")).toBe(storedNotes);
  });
});
