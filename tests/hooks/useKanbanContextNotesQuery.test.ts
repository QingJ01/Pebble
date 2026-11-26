import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { QueryClient } from "@tanstack/react-query";
import {
  fetchKanbanContextNotes,
  kanbanContextNotesQueryKey,
  type KanbanContextNotes,
} from "../../src/hooks/queries/useKanbanContextNotesQuery";
import { setKanbanContextNoteInCache } from "../../src/hooks/mutations/useKanbanMutations";

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

let client: QueryClient;

/** What react-query does with the query function's result. */
async function fetchContextNotes(target: QueryClient = client) {
  const notes = await fetchKanbanContextNotes(target);
  target.setQueryData(kanbanContextNotesQueryKey, notes);
}

function contextNotes(target: QueryClient = client): KanbanContextNotes | undefined {
  return target.getQueryData<KanbanContextNotes>(kanbanContextNotesQueryKey);
}

function setContextNote(messageId: string, note: string) {
  return setKanbanContextNoteInCache(client, messageId, note);
}

describe("kanban context notes (react-query)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    client = new QueryClient();
  });

  it("fetch loads notes only; cards have their own query", async () => {
    mockedInvoke.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") {
        return Promise.resolve({ m1: "note" });
      }
      return Promise.resolve(undefined);
    });

    await fetchContextNotes();

    expect(mockedInvoke).toHaveBeenCalledWith("list_kanban_context_notes");
    expect(mockedInvoke).not.toHaveBeenCalledWith("list_kanban_cards", expect.anything());
    expect(contextNotes()).toEqual({ m1: "note" });
    expect(kanbanContextNotesQueryKey[0]).toBe("kanban");
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

    const olderFetch = fetchContextNotes();
    const newerFetch = fetchContextNotes();
    secondNotes.resolve({ new: "new note" });
    await newerFetch;

    firstNotes.resolve({ old: "old note" });
    await olderFetch;

    expect(contextNotes()).toEqual({ new: "new note" });
  });

  it("preserves a context note edited while a fetch is in flight", async () => {
    const fetchedNotes = deferred<Record<string, string>>();
    const savedNote = deferred<Record<string, string>>();
    mockedInvoke.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") return fetchedNotes.promise;
      if (command === "set_kanban_context_note") return savedNote.promise;
      return Promise.resolve(undefined);
    });

    const fetch = fetchContextNotes();
    const save = setContextNote("m1", "new note");
    expect(contextNotes()).toEqual({ m1: "new note" });

    fetchedNotes.resolve({ m1: "stale note" });
    await fetch;
    expect(contextNotes()).toEqual({ m1: "new note" });

    savedNote.resolve({ m1: "new note" });
    await save;
    expect(contextNotes()).toEqual({ m1: "new note" });
  });

  it("stores context notes through backend storage only", async () => {
    mockedInvoke.mockResolvedValueOnce({ m1: "follow up on the selected paragraph" });

    await setContextNote("m1", "follow up on the selected paragraph");

    expect(contextNotes()?.m1).toBe("follow up on the selected paragraph");
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

    const firstUpdate = setContextNote("m1", "one");
    const secondUpdate = setContextNote("m2", "two");
    expect(contextNotes()).toEqual({ m1: "one", m2: "two" });

    second.resolve({ m2: "two" });
    await secondUpdate;
    expect(contextNotes()).toEqual({ m1: "one", m2: "two" });

    first.resolve({ m1: "one" });
    await firstUpdate;
    expect(contextNotes()).toEqual({ m1: "one", m2: "two" });
  });

  it("ignores a stale response for a newer edit of the same context note", async () => {
    const first = deferred<Record<string, string>>();
    const second = deferred<Record<string, string>>();
    mockedInvoke
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const firstUpdate = setContextNote("m1", "old");
    const secondUpdate = setContextNote("m1", "new");
    second.resolve({ m1: "new" });
    await secondUpdate;
    first.resolve({ m1: "old" });
    await firstUpdate;

    expect(contextNotes()).toEqual({ m1: "new" });
  });

  it("keeps legacy context notes until the backend acknowledges migration", async () => {
    vi.resetModules();
    localStorage.setItem("pebble-kanban-context-notes", JSON.stringify({ m1: "legacy note" }));
    const { invoke: isolatedInvoke } = await import("@tauri-apps/api/core");
    const isolatedInvokeMock = vi.mocked(isolatedInvoke);
    const isolated = await import("../../src/hooks/queries/useKanbanContextNotesQuery");
    const { QueryClient: IsolatedQueryClient } = await import("@tanstack/react-query");
    const isolatedClient = new IsolatedQueryClient();
    const isolatedFetch = async () => isolatedClient.setQueryData(
      isolated.kanbanContextNotesQueryKey,
      await isolated.fetchKanbanContextNotes(isolatedClient),
    );

    expect(localStorage.getItem("pebble-kanban-context-notes"))
      .toBe(JSON.stringify({ m1: "legacy note" }));
    isolatedInvokeMock.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") return Promise.resolve({});
      if (command === "merge_kanban_context_notes") return Promise.resolve({ m1: "legacy note" });
      return Promise.resolve(undefined);
    });

    await isolatedFetch();

    expect(isolatedInvokeMock).toHaveBeenCalledWith("merge_kanban_context_notes", {
      notes: { m1: "legacy note" },
    });
    expect(isolatedClient.getQueryData(isolated.kanbanContextNotesQueryKey)).toEqual({ m1: "legacy note" });
    expect(localStorage.getItem("pebble-kanban-context-notes")).toBeNull();
  });

  it("retains legacy context notes when backend migration fails", async () => {
    vi.resetModules();
    const storedNotes = JSON.stringify({ m1: "legacy note" });
    localStorage.setItem("pebble-kanban-context-notes", storedNotes);
    const { invoke: isolatedInvoke } = await import("@tauri-apps/api/core");
    const isolatedInvokeMock = vi.mocked(isolatedInvoke);
    const isolated = await import("../../src/hooks/queries/useKanbanContextNotesQuery");
    const { QueryClient: IsolatedQueryClient } = await import("@tanstack/react-query");
    const isolatedClient = new IsolatedQueryClient();
    const isolatedFetch = async () => isolatedClient.setQueryData(
      isolated.kanbanContextNotesQueryKey,
      await isolated.fetchKanbanContextNotes(isolatedClient),
    );
    isolatedInvokeMock.mockImplementation((command) => {
      if (command === "list_kanban_context_notes") return Promise.resolve({});
      if (command === "merge_kanban_context_notes") return Promise.reject(new Error("write failed"));
      return Promise.resolve(undefined);
    });

    await expect(isolatedFetch()).rejects.toThrow("write failed");

    expect(localStorage.getItem("pebble-kanban-context-notes")).toBe(storedNotes);
  });

  it("rolls back a failed save to the previous note", async () => {
    client.setQueryData(kanbanContextNotesQueryKey, { m1: "before" });
    mockedInvoke.mockRejectedValueOnce(new Error("save failed"));

    await expect(setContextNote("m1", "after")).rejects.toThrow("save failed");

    expect(contextNotes()).toEqual({ m1: "before" });
  });
});
