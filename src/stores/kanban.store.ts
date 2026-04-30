import { create } from "zustand";
import type { KanbanCard, KanbanColumnType } from "@/lib/api";
import {
  listKanbanCards,
  listKanbanContextNotes,
  mergeKanbanContextNotes,
  moveToKanban,
  removeFromKanban,
  setKanbanContextNote,
} from "@/lib/api";

interface KanbanState {
  cards: KanbanCard[];
  cardIdSet: Set<string>;
  contextNotes: Record<string, string>;
  loading: boolean;
  fetchCards: () => Promise<void>;
  moveCard: (messageId: string, column: KanbanColumnType, position: number) => Promise<void>;
  addCard: (messageId: string, column: KanbanColumnType) => Promise<void>;
  removeCard: (messageId: string) => Promise<void>;
  reorderInColumn: (column: KanbanColumnType, orderedIds: string[]) => void;
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
if (typeof localStorage !== "undefined") {
  localStorage.removeItem(LEGACY_CONTEXT_NOTES_STORAGE_KEY);
}

async function loadContextNotes(): Promise<Record<string, string>> {
  const backendNotes = await listKanbanContextNotes();
  const legacyNotes = legacyContextNotes;
  legacyContextNotes = {};
  const legacyEntries = Object.entries(legacyNotes).filter(
    ([messageId, note]) => messageId && note && backendNotes[messageId] === undefined,
  );

  if (legacyEntries.length === 0) {
    return backendNotes;
  }

  return mergeKanbanContextNotes(Object.fromEntries(legacyEntries));
}

function buildIdSet(cards: KanbanCard[]): Set<string> {
  return new Set(cards.map((c) => c.message_id));
}

export const useKanbanStore = create<KanbanState>((set, get) => ({
  cards: [],
  cardIdSet: new Set<string>(),
  contextNotes: {},
  loading: false,

  fetchCards: async () => {
    set({ loading: true });
    try {
      const [cards, contextNotes] = await Promise.all([listKanbanCards(), loadContextNotes()]);
      set({ cards, cardIdSet: buildIdSet(cards), contextNotes });
    } finally {
      set({ loading: false });
    }
  },

  moveCard: async (messageId: string, column: KanbanColumnType, position: number) => {
    // Optimistic update
    const prev = get().cards;
    const updated = prev.map((c) =>
      c.message_id === messageId ? { ...c, column, position } : c,
    );
    set({ cards: updated, cardIdSet: buildIdSet(updated) });
    try {
      await moveToKanban(messageId, column, position);
    } catch {
      // Rollback on error
      set({ cards: prev, cardIdSet: buildIdSet(prev) });
    }
  },

  addCard: async (messageId: string, column: KanbanColumnType) => {
    await moveToKanban(messageId, column);
    await get().fetchCards();
  },

  reorderInColumn: (column, orderedIds) => {
    const prev = get().cards;
    const others = prev.filter((c) => c.column !== column);
    const reordered = orderedIds
      .map((id, i) => {
        const card = prev.find((c) => c.message_id === id);
        return card ? { ...card, position: i } : null;
      })
      .filter((c): c is KanbanCard => c !== null);
    const merged = [...others, ...reordered];
    set({ cards: merged, cardIdSet: buildIdSet(merged) });
    // Persist all position changes and rollback entirely on any failure
    Promise.all(
      reordered.map((card) => moveToKanban(card.message_id, card.column, card.position)),
    ).catch(() => {
      set({ cards: prev, cardIdSet: buildIdSet(prev) });
    });
  },

  removeCard: async (messageId: string) => {
    const prev = get().cards;
    const filtered = prev.filter((c) => c.message_id !== messageId);
    set({ cards: filtered, cardIdSet: buildIdSet(filtered) });
    try {
      await removeFromKanban(messageId);
    } catch {
      set({ cards: prev, cardIdSet: buildIdSet(prev) });
    }
  },

  setContextNote: async (messageId, note) => {
    const prev = get().contextNotes;
    const next = { ...prev };
    if (note) {
      next[messageId] = note;
    } else {
      delete next[messageId];
    }
    set({ contextNotes: next });
    try {
      const saved = await setKanbanContextNote(messageId, note);
      set({ contextNotes: saved });
    } catch (err) {
      set({ contextNotes: prev });
      throw err;
    }
  },
}));
