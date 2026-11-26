/**
 * useDelayedFlag — debounce a loading flag for display.
 *
 *   const showSkeleton = useDelayedFlag(loading, { delayMs: 200, minVisibleMs: 300 });
 *
 * Becomes true only after `active` has stayed true for `delayMs` (fast loads
 * never flash a placeholder) and, once shown, stays true for at least
 * `minVisibleMs` (no sub-frame flicker when loading ends right after).
 */
import { useEffect, useRef, useState } from "react";

export function useDelayedFlag(
  active: boolean,
  { delayMs = 200, minVisibleMs = 300 }: { delayMs?: number; minVisibleMs?: number } = {},
) {
  const [visible, setVisible] = useState(false);
  const shownAtRef = useRef<number | null>(null);

  useEffect(() => {
    if (active) {
      if (visible) return;
      const timer = setTimeout(() => {
        shownAtRef.current = Date.now();
        setVisible(true);
      }, delayMs);
      return () => clearTimeout(timer);
    }
    if (!visible) return;
    const elapsed = Date.now() - (shownAtRef.current ?? 0);
    const remaining = Math.max(0, minVisibleMs - elapsed);
    const timer = setTimeout(() => {
      shownAtRef.current = null;
      setVisible(false);
    }, remaining);
    return () => clearTimeout(timer);
  }, [active, visible, delayMs, minVisibleMs]);

  return visible;
}
