import { create } from "zustand";

/** Ephemeral presentation state only; queued mail remains encrypted in SQLite. */
export const useSendControlsStore = create<{ composeUndoId: string | null }>(() => ({ composeUndoId: null }));
