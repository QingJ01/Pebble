/**
 * useFocusScope — move focus into an overlay when it opens and give it back
 * when it closes.
 *
 *   useFocusScope({ active: open, containerRef: panelRef, initialFocus: okRef });
 *
 * - initialFocus: a ref, "first" (first focusable; `[data-autofocus]` wins),
 *   "container" (the container itself; give it tabIndex={-1}) or "none".
 * - On close/unmount focus returns to the element that was focused when the
 *   overlay opened, but only if focus is still inside the overlay or was lost
 *   to <body> — a deliberate click elsewhere is never overridden.
 */
import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";

export type InitialFocus = RefObject<HTMLElement | null> | "first" | "container" | "none";

const FOCUSABLE =
  'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, [tabindex]:not([tabindex="-1"])';

export function firstFocusable(container: HTMLElement): HTMLElement | null {
  const preferred = container.querySelector<HTMLElement>("[data-autofocus]");
  if (preferred) return preferred;
  const items = Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => !el.closest("[hidden],[inert]") && el.getAttribute("aria-hidden") !== "true",
  );
  return items[0] ?? null;
}

function restore(previous: HTMLElement, container: HTMLElement | null) {
  if (!previous.isConnected) return;
  const current = document.activeElement;
  const lost = !current || current === document.body || (container?.contains(current) ?? false);
  if (lost && current !== previous) previous.focus({ preventScroll: true });
}

export function useFocusScope({
  active,
  containerRef,
  initialFocus = "first",
  restoreFocus = true,
}: {
  active: boolean;
  containerRef: RefObject<HTMLElement | null>;
  initialFocus?: InitialFocus;
  restoreFocus?: boolean;
}) {
  const previousRef = useRef<HTMLElement | null>(null);
  const initialRef = useRef(initialFocus);
  initialRef.current = initialFocus;
  const restoreRef = useRef(restoreFocus);
  restoreRef.current = restoreFocus;

  const pendingRef = useRef<{ previous: HTMLElement; container: HTMLElement | null } | null>(null);

  // Capture the trigger before anything inside the overlay can take focus.
  // Restoring happens twice on purpose: in the cleanup (covers unmount) and in
  // the next layout pass (React re-focuses the previously focused element after
  // a commit if it is still in the document, e.g. during an exit animation).
  useLayoutEffect(() => {
    if (!active) {
      const pending = pendingRef.current;
      pendingRef.current = null;
      if (pending) restore(pending.previous, pending.container);
      return;
    }
    previousRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const container = containerRef.current;
    return () => {
      const previous = previousRef.current;
      previousRef.current = null;
      if (!restoreRef.current || !previous) return;
      pendingRef.current = { previous, container };
      restore(previous, container);
    };
  }, [active, containerRef]);

  useEffect(() => {
    if (!active) return;
    const container = containerRef.current;
    if (!container) return;
    const initial = initialRef.current;
    if (initial === "none") return;
    let target: HTMLElement | null = null;
    if (initial === "container") target = container;
    else if (initial === "first") target = firstFocusable(container) ?? container;
    else target = initial.current ?? firstFocusable(container) ?? container;
    if (target === container && !container.hasAttribute("tabindex")) container.tabIndex = -1;
    target.focus({ preventScroll: true });
  }, [active, containerRef]);
}
