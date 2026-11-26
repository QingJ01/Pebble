/**
 * usePresence — keep an element mounted for its exit animation.
 *
 *   const { mounted, exiting } = usePresence(open);          // default --dur-exit (120ms)
 *   if (!mounted) return null;
 *   <div className={exiting ? "anim-dialog-out" : "anim-dialog-in"} />
 */
import { useEffect, useState } from "react";

export const EXIT_DURATION_MS = 120;

export function usePresence(open: boolean, exitMs: number = EXIT_DURATION_MS) {
  const [mounted, setMounted] = useState(open);
  const [exiting, setExiting] = useState(false);

  useEffect(() => {
    if (open) {
      setMounted(true);
      setExiting(false);
      return;
    }
    if (!mounted) return;
    setExiting(true);
    const timer = setTimeout(() => {
      setMounted(false);
      setExiting(false);
    }, exitMs);
    return () => clearTimeout(timer);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  return { mounted: open || mounted, exiting: !open && exiting };
}
