import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { listKanbanContextNotes, mergeKanbanContextNotes } from "@/lib/api";
import { kanbanQueryRoot } from "@/hooks/queries/useKanbanCardsQuery";

export type KanbanContextNotes = Record<string, string>;

export const kanbanContextNotesQueryKey = [...kanbanQueryRoot, "context-notes"] as const;

const EMPTY_NOTES: KanbanContextNotes = {};
const LEGACY_CONTEXT_NOTES_STORAGE_KEY = "pebble-kanban-context-notes";

function loadLegacyContextNotes(): KanbanContextNotes {
  if (typeof localStorage === "undefined") return {};
  try {
    const parsed = JSON.parse(localStorage.getItem(LEGACY_CONTEXT_NOTES_STORAGE_KEY) || "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

// ─── In-flight edit bookkeeping ──────────────────────────────────────────────
// A fetch that started before (or overlaps) a local edit must not clobber the
// optimistic value, and a slow response for an older edit of a note must not
// overwrite a newer one. Shared by the query function and the mutation.

let legacyContextNotes = loadLegacyContextNotes();
let nextContextNoteGeneration = 0;
/** messageId → generation of the edit whose save is still in flight. */
const latestContextNoteGeneration = new Map<string, number>();
/** messageId → generation of the most recent edit (settled or not). */
const contextNoteMutationGeneration = new Map<string, number>();
let nextFetchGeneration = 0;
let latestFetchGeneration = 0;

async function loadContextNotes(): Promise<KanbanContextNotes> {
  const backendNotes = await listKanbanContextNotes();
  const legacyEntries = Object.entries(legacyContextNotes).filter(
    ([messageId, note]) => messageId && note && backendNotes[messageId] === undefined,
  );

  const mergedNotes = legacyEntries.length === 0
    ? backendNotes
    : await mergeKanbanContextNotes(Object.fromEntries(legacyEntries));

  legacyContextNotes = {};
  if (typeof localStorage !== "undefined") {
    try {
      localStorage.removeItem(LEGACY_CONTEXT_NOTES_STORAGE_KEY);
    } catch { /* keep the in-memory migration result */ }
  }
  return mergedNotes;
}

function cachedNotes(client: QueryClient): KanbanContextNotes {
  return client.getQueryData<KanbanContextNotes>(kanbanContextNotesQueryKey) ?? EMPTY_NOTES;
}

function hasNote(notes: KanbanContextNotes, messageId: string) {
  return Object.prototype.hasOwnProperty.call(notes, messageId);
}

function withNote(notes: KanbanContextNotes, messageId: string, note: string | undefined, present: boolean) {
  const next = { ...notes };
  if (present && note !== undefined) {
    next[messageId] = note;
  } else {
    delete next[messageId];
  }
  return next;
}

/**
 * Query function for the context notes: loads backend notes (migrating any
 * legacy localStorage notes once) and keeps notes edited while the fetch was
 * in flight. A fetch superseded by a newer one keeps the cached value.
 */
export async function fetchKanbanContextNotes(client: QueryClient): Promise<KanbanContextNotes> {
  const fetchGeneration = ++nextFetchGeneration;
  latestFetchGeneration = fetchGeneration;
  const contextNoteGenerationAtStart = nextContextNoteGeneration;
  const pendingAtStart = new Set(latestContextNoteGeneration.keys());
  const fetched = await loadContextNotes();
  const current = cachedNotes(client);
  if (latestFetchGeneration !== fetchGeneration) return current;

  let merged = { ...fetched };
  const notesToPreserve = new Set([
    ...pendingAtStart,
    ...latestContextNoteGeneration.keys(),
    ...[...contextNoteMutationGeneration.entries()]
      .filter(([, generation]) => generation > contextNoteGenerationAtStart)
      .map(([messageId]) => messageId),
  ]);
  for (const messageId of notesToPreserve) {
    merged = withNote(merged, messageId, current[messageId], hasNote(current, messageId));
  }
  return merged;
}

export function useKanbanContextNotesQuery(options: { refetchOnMount?: boolean | "always" } = {}) {
  const client = useQueryClient();
  return useQuery({
    queryKey: kanbanContextNotesQueryKey,
    queryFn: () => fetchKanbanContextNotes(client),
    refetchOnMount: options.refetchOnMount,
  });
}

/**
 * Optimistically set (or clear, with an empty note) one context note and
 * persist it. Rolls back on failure unless a newer edit of the same note
 * already superseded this one.
 */
export async function saveKanbanContextNote(
  client: QueryClient,
  messageId: string,
  note: string,
  save: (messageId: string, note: string) => Promise<KanbanContextNotes>,
): Promise<void> {
  const previous = cachedNotes(client);
  const hadPreviousNote = hasNote(previous, messageId);
  const previousNote = previous[messageId];
  const generation = ++nextContextNoteGeneration;
  latestContextNoteGeneration.set(messageId, generation);
  contextNoteMutationGeneration.set(messageId, generation);
  client.setQueryData<KanbanContextNotes>(
    kanbanContextNotesQueryKey,
    withNote(previous, messageId, note, Boolean(note)),
  );
  try {
    const saved = await save(messageId, note);
    if (latestContextNoteGeneration.get(messageId) !== generation) return;
    client.setQueryData<KanbanContextNotes>(kanbanContextNotesQueryKey, (current = EMPTY_NOTES) =>
      withNote(current, messageId, saved[messageId], hasNote(saved, messageId)));
    latestContextNoteGeneration.delete(messageId);
  } catch (err) {
    if (latestContextNoteGeneration.get(messageId) === generation) {
      client.setQueryData<KanbanContextNotes>(kanbanContextNotesQueryKey, (current = EMPTY_NOTES) =>
        withNote(current, messageId, previousNote, hadPreviousNote));
      latestContextNoteGeneration.delete(messageId);
    }
    throw err;
  }
}
