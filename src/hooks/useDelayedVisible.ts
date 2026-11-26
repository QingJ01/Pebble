import { useEffect, useState } from "react";

/** Default delay before a loading indicator appears (avoids flashes on fast loads). */
export const LOADING_INDICATOR_DELAY_MS = 180;

/**
 * Returns false for the first `delayMs` after mount, then true. Use it to hold
 * back skeletons/spinners so quick loads never flash a placeholder.
 */
export function useDelayedVisible(delayMs: number = LOADING_INDICATOR_DELAY_MS): boolean {
  const [visible, setVisible] = useState(delayMs <= 0);
  useEffect(() => {
    if (delayMs <= 0) return;
    const timer = setTimeout(() => setVisible(true), delayMs);
    return () => clearTimeout(timer);
  }, [delayMs]);
  return visible;
}
