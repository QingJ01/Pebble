/**
 * useOverlayLayer — register a React overlay on the shared overlay stack
 * (src/lib/overlayStack.ts) while `open` is true.
 *
 *   const panelRef = useRef<HTMLDivElement>(null);
 *   const layer = useOverlayLayer({ open, modal: true, onEscape: close, containerRef: panelRef });
 *   // in your own key handlers: if (!layer.isTop()) return;
 *
 * - modal: blocks global single-key shortcuts and traps Tab in containerRef.
 * - onEscape: called only when this overlay is top-most (Escape is consumed).
 *   For non-modal popovers pass modal:false + onEscape — global Escape handling
 *   (useKeyboard) then leaves Escape to the popover.
 * - trapFocus: override the Tab trap (defaults to `modal`).
 * Callbacks may change every render; the stack position is fixed at open time.
 */
import { useCallback, useLayoutEffect, useRef, type RefObject } from "react";
import { isTopOverlay, pushOverlay, updateOverlay } from "@/lib/overlayStack";

export interface UseOverlayLayerOptions {
  open?: boolean;
  modal?: boolean;
  onEscape?: () => void;
  containerRef?: RefObject<HTMLElement | null>;
  trapFocus?: boolean;
}

export function useOverlayLayer({
  open = true,
  modal = false,
  onEscape,
  containerRef,
  trapFocus,
}: UseOverlayLayerOptions) {
  const idRef = useRef<number | null>(null);
  const onEscapeRef = useRef(onEscape);
  onEscapeRef.current = onEscape;
  const hasEscape = Boolean(onEscape);

  useLayoutEffect(() => {
    if (!open) return;
    const { id, release } = pushOverlay({});
    idRef.current = id;
    return () => {
      release();
      idRef.current = null;
    };
  }, [open]);

  useLayoutEffect(() => {
    if (idRef.current === null) return;
    updateOverlay(idRef.current, {
      modal,
      trapFocus,
      onEscape: hasEscape ? () => onEscapeRef.current?.() : undefined,
      getContainer: containerRef ? () => containerRef.current : undefined,
    });
  });

  const isTop = useCallback(() => idRef.current !== null && isTopOverlay(idRef.current), []);
  return { isTop };
}
