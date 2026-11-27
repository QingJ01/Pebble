import { useEffect, useMemo, useState } from "react";

type RowsTransform<T> = (rows: T[]) => T[];

/**
 * Unread-only lists keep the row the user has open, even after opening it
 * marks it read and the server list drops it, so the list doesn't jump
 * under the reader and J/K keeps its place. Only the open row is kept: once
 * the reader moves on or closes (including after the message is archived,
 * deleted or moved), it leaves the list. A change of `resetKey` (folder or
 * view) or turning the filter off also drops it.
 *
 * `rows` is the server list; `openedKey` the key of the open row. A kept row
 * missing from `rows` goes through `markRead` (it left the unread list, so it
 * is read now) and is merged back in by `compareNewestFirst`.
 * `subscribePatches` feeds the kept copy the same optimistic patches the list
 * cache gets (stars, flags, removals), since it is no longer in that cache.
 */
export function useStickyRows<T>({
  rows,
  enabled,
  resetKey,
  openedKey,
  keyOf,
  markRead,
  compareNewestFirst,
  subscribePatches,
}: {
  rows: T[];
  enabled: boolean;
  resetKey: string;
  openedKey: string | null;
  keyOf: (row: T) => string;
  markRead: (row: T) => T;
  compareNewestFirst: (a: T, b: T) => number;
  subscribePatches?: (listener: (transform: RowsTransform<T>) => void) => () => void;
}): T[] {
  const [kept, setKept] = useState<{ resetKey: string; key: string; row: T } | null>(null);
  // A changed filter/folder or a different open row drops it in the same
  // render (no stale flash).
  const current = enabled && kept && kept.resetKey === resetKey && kept.key === openedKey ? kept : null;

  useEffect(() => {
    // Closing the reader or turning the filter off forgets the row.
    if (!enabled) {
      setKept(null);
      return;
    }
    if (!openedKey) {
      setKept(null);
      return;
    }
    const row = rows.find((candidate) => keyOf(candidate) === openedKey);
    setKept((prev) => {
      const same = prev && prev.resetKey === resetKey && prev.key === openedKey;
      if (!row) return same ? prev : null;
      // Keep the latest copy (e.g. after the optimistic "read" patch).
      if (same && prev.row === row) return prev;
      return { resetKey, key: openedKey, row };
    });
  }, [enabled, openedKey, rows, resetKey]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!enabled || !subscribePatches) return;
    return subscribePatches((transform) => {
      setKept((prev) => {
        if (!prev) return prev;
        const next = transform([prev.row])[0];
        if (next === prev.row) return prev;
        return next === undefined ? null : { ...prev, row: next };
      });
    });
  }, [enabled, subscribePatches]);

  return useMemo(() => {
    if (!current || rows.some((row) => keyOf(row) === current.key)) return rows;
    return [...rows, markRead(current.row)].sort(compareNewestFirst);
  }, [current, rows]); // eslint-disable-line react-hooks/exhaustive-deps
}
