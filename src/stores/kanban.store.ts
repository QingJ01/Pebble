import { create } from "zustand";
import {
  listKanbanContextNotes,
  mergeKanbanContextNotes,
  setKanbanContextNote,
} from "@/lib/api";

/**
 * Kanban client state. Board cards are server data and live in react-query
 * (see `useKanbanCardsQuery` / `useKanbanMutations`); this store only keeps
 * the context-note overlay with its in-flight edit bookkeeping.
 */
interface KanbanState {
  contextNotes: Record<string, string>;
  fetchContextNotes: () => Promise<void>;
  setContextNote: (messageId: string, note: string) => Promise<void>;
}

const LEGACY_CONTEXT_NOTES_STORAGE_KEY = "pebble-kanban-context-notes";

function loadLegacyContextNotes(): Record<string, string> {
  if (typeof localStorage === "undefined") return {};
  try {
    const parsed = JSON.parse(localStorage.getItem(LEGACY_CONTEXT_NOTES_STORAGE_KEY) || "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

let legacyContextNotes = loadLegacyContextNotes();
let nextContextNoteGeneration = 0;
const latestContextNoteGeneration = new Map<string, number>();
const contextNoteMutationGeneration = new Map<string, number>();
let nextFetchGeneration = 0;
let latestFetchGeneration = 0;

async function loadContextNotes(): Promise<Record<string, string>> {
  const backendNotes = await listKanbanContextNotes();
  const legacyNotes = legacyContextNotes;
  const legacyEntries = Object.entries(legacyNotes).filter(
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

export const useKanbanStore = create<KanbanState>((set) => ({
  contextNotes: {},

  fetchContextNotes: async () => {
    const fetchGeneration = ++nextFetchGeneration;
    latestFetchGeneration = fetchGeneration;
    const contextNoteGenerationAtStart = nextContextNoteGeneration;
    const pendingContextNotesAtStart = new Set(latestContextNoteGeneration.keys());
    const contextNotes = await loadContextNotes();
    if (latestFetchGeneration !== fetchGeneration) return;
    set((state) => {
      const mergedContextNotes = { ...contextNotes };
      const notesToPreserve = new Set([
        ...pendingContextNotesAtStart,
        ...latestContextNoteGeneration.keys(),
        ...[...contextNoteMutationGeneration.entries()]
          .filter(([, generation]) => generation > contextNoteGenerationAtStart)
          .map(([messageId]) => messageId),
      ]);
      for (const messageId of notesToPreserve) {
        if (Object.prototype.hasOwnProperty.call(state.contextNotes, messageId)) {
          mergedContextNotes[messageId] = state.contextNotes[messageId];
        } else {
          delete mergedContextNotes[messageId];
        }
      }
      return { contextNotes: mergedContextNotes };
    });
  },

  setContextNote: async (messageId, note) => {
    const prev = useKanbanStore.getState().contextNotes;
    const hadPreviousNote = Object.prototype.hasOwnProperty.call(prev, messageId);
    const previousNote = prev[messageId];
    const generation = ++nextContextNoteGeneration;
    latestContextNoteGeneration.set(messageId, generation);
    contextNoteMutationGeneration.set(messageId, generation);
    const next = { ...prev };
    if (note) {
      next[messageId] = note;
    } else {
      delete next[messageId];
    }
    set({ contextNotes: next });
    try {
      const saved = await setKanbanContextNote(messageId, note);
      if (latestContextNoteGeneration.get(messageId) !== generation) return;
      set((state) => {
        const merged = { ...state.contextNotes };
        if (Object.prototype.hasOwnProperty.call(saved, messageId)) {
          merged[messageId] = saved[messageId];
        } else {
          delete merged[messageId];
        }
        return { contextNotes: merged };
      });
      latestContextNoteGeneration.delete(messageId);
    } catch (err) {
      if (latestContextNoteGeneration.get(messageId) === generation) {
        set((state) => {
          const rolledBack = { ...state.contextNotes };
          if (hadPreviousNote) {
            rolledBack[messageId] = previousNote;
          } else {
            delete rolledBack[messageId];
          }
          return { contextNotes: rolledBack };
        });
        latestContextNoteGeneration.delete(messageId);
      }
      throw err;
    }
  },
}));
