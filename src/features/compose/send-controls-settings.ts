const KEY = "pebble-undo-send-seconds";
export const UNDO_OPTIONS = [0, 5, 10, 20, 30, 60] as const;
export function loadUndoSeconds(): number {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return 10;
    const value = Number(raw);
    return UNDO_OPTIONS.some((option) => option === value) ? value : 10;
  } catch { return 10; }
}
export function saveUndoSeconds(seconds: number) {
  if (!UNDO_OPTIONS.some((option) => option === seconds)) return;
  try { localStorage.setItem(KEY, String(seconds)); } catch { /* The current session still applies the selected delay. */ }
}
