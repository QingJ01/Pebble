import { useEffect, useMemo, useState } from "react";

/**
 * Unread-only lists keep the rows the user opened, even after opening them
 * marks them read and the server list drops them, so the list doesn't jump
 * under the reader and J/K keeps its place. The rows go away when the filter,
 * folder or view changes (`resetKey`).
 *
 * `rows` is the server list; `openedKey` the key of the open row. Kept rows
 * missing from `rows` are passed through `markRead` (they left the unread
 * list, so they are read now) and merged back in by `compareNewestFirst`.
 */
export function useStickyRows<T>({
  rows,
  enabled,
  resetKey,
  openedKey,
  keyOf,
  markRead,
  compareNewestFirst,
}: {
  rows: T[];
  enabled: boolean;
  resetKey: string;
  openedKey: string | null;
  keyOf: (row: T) => string;
  markRead: (row: T) => T;
  compareNewestFirst: (a: T, b: T) => number;
}): T[] {
  const [sticky, setSticky] = useState<{ resetKey: string; rows: Map<string, T> }>(
    () => ({ resetKey, rows: new Map() }),
  );
  // A changed filter/folder starts empty in the same render (no stale flash).
  const kept = sticky.resetKey === resetKey ? sticky.rows : null;

  useEffect(() => {
    if (!enabled || !openedKey) return;
    const row = rows.find((candidate) => keyOf(candidate) === openedKey);
    setSticky((prev) => {
      const base = prev.resetKey === resetKey ? prev.rows : new Map<string, T>();
      if (!row) return base === prev.rows ? prev : { resetKey, rows: base };
      // Keep the latest copy (e.g. after the optimistic "read" patch).
      if (base.get(openedKey) === row) return prev;
      const next = new Map(base);
      next.set(openedKey, row);
      return { resetKey, rows: next };
    });
  }, [enabled, openedKey, rows, resetKey]); // eslint-disable-line react-hooks/exhaustive-deps

  return useMemo(() => {
    if (!enabled || !kept || kept.size === 0) return rows;
    const present = new Set(rows.map(keyOf));
    const missing = [...kept.entries()]
      .filter(([key]) => !present.has(key))
      .map(([, row]) => markRead(row));
    if (missing.length === 0) return rows;
    return [...rows, ...missing].sort(compareNewestFirst);
  }, [enabled, kept, rows]); // eslint-disable-line react-hooks/exhaustive-deps
}
