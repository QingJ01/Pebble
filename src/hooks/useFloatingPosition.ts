/**
 * useFloatingPosition — position a `position: fixed` element next to an anchor
 * element or a viewport point using @floating-ui/dom (offset + flip + shift +
 * size). The floating element is written to directly (left/top/maxHeight), so
 * no React re-render is needed and there is no first-frame jump.
 *
 *   useFloatingPosition({ open, floatingRef, anchorRef, placement: "bottom-end" });
 *   useFloatingPosition({ open, floatingRef, point: { x, y } });
 *
 * maxHeight is min(60vh, space available on the chosen side); the element is
 * expected to scroll internally (`.popover-panel` sets overflow: auto).
 */
import { useLayoutEffect, type RefObject } from "react";
import {
  autoUpdate,
  computePosition,
  flip,
  offset,
  shift,
  size,
  type Placement,
  type VirtualElement,
} from "@floating-ui/dom";

const VIEWPORT_PADDING = 8;
const MIN_HEIGHT = 120;

export interface FloatingPoint {
  x: number;
  y: number;
}

export function useFloatingPosition({
  open,
  floatingRef,
  anchorRef,
  point,
  placement = "bottom-start",
  gap = 6,
  matchWidth = false,
}: {
  open: boolean;
  floatingRef: RefObject<HTMLElement | null>;
  anchorRef?: RefObject<HTMLElement | null>;
  point?: FloatingPoint | null;
  placement?: Placement;
  gap?: number;
  /** Make the floating element exactly as wide as the anchor (comboboxes). */
  matchWidth?: boolean;
}) {
  const px = point?.x;
  const py = point?.y;

  useLayoutEffect(() => {
    if (!open) return;
    const floating = floatingRef.current;
    if (!floating) return;
    let reference: Element | VirtualElement | null = anchorRef?.current ?? null;
    if (!reference && px !== undefined && py !== undefined) {
      const rect = { x: px, y: py, left: px, top: py, right: px, bottom: py, width: 0, height: 0 };
      reference = { getBoundingClientRect: () => ({ ...rect, toJSON: () => rect }) as DOMRect };
    }
    if (!reference) return;
    const ref = reference;

    let cancelled = false;
    const update = () => {
      void computePosition(ref, floating, {
        strategy: "fixed",
        placement,
        middleware: [
          offset(gap),
          flip({ padding: VIEWPORT_PADDING }),
          shift({ padding: VIEWPORT_PADDING }),
          size({
            padding: VIEWPORT_PADDING,
            apply({ availableHeight, elements, rects }) {
              if (matchWidth) elements.floating.style.width = `${Math.round(rects.reference.width)}px`;
              const limit = Math.min(window.innerHeight * 0.6, availableHeight);
              elements.floating.style.maxHeight = `${Math.max(MIN_HEIGHT, Math.floor(limit))}px`;
            },
          }),
        ],
      }).then(({ x, y, placement: finalPlacement }) => {
        if (cancelled) return;
        Object.assign(floating.style, { left: `${Math.round(x)}px`, top: `${Math.round(y)}px` });
        floating.dataset.placement = finalPlacement;
      }).catch(() => {});
    };

    if (ref instanceof Element) {
      const stop = autoUpdate(ref, floating, update);
      return () => { cancelled = true; stop(); };
    }
    update();
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(update) : null;
    ro?.observe(floating);
    window.addEventListener("resize", update);
    return () => {
      cancelled = true;
      ro?.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [open, floatingRef, anchorRef, px, py, placement, gap, matchWidth]);
}
