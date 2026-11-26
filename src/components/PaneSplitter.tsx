import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { useTranslation } from "react-i18next";

/**
 * Width of a resizable left pane, persisted in localStorage under `storageKey`.
 * Storage failures (private mode, quota, blocked) are ignored.
 */
export function usePaneWidth(storageKey: string, defaultWidth: number, min: number, max: number) {
  const clamp = useCallback((value: number) => Math.round(Math.min(max, Math.max(min, value))), [min, max]);
  const [width, setWidthState] = useState(() => {
    try {
      const stored = Number(window.localStorage.getItem(storageKey));
      return Number.isFinite(stored) && stored > 0 ? clamp(stored) : defaultWidth;
    } catch {
      return defaultWidth;
    }
  });

  const setWidth = useCallback((value: number) => {
    const next = clamp(value);
    setWidthState(next);
    try {
      window.localStorage.setItem(storageKey, String(next));
    } catch {
      // Persisting is best-effort.
    }
  }, [clamp, storageKey]);

  const reset = useCallback(() => {
    setWidthState(defaultWidth);
    try {
      window.localStorage.removeItem(storageKey);
    } catch {
      // ignore
    }
  }, [defaultWidth, storageKey]);

  return { width, setWidth, reset, min, max };
}

/**
 * Vertical drag handle between a list pane and the reader. Pointer-drag,
 * arrow keys (Shift = larger steps), Home/End, and double-click to reset.
 */
export default function PaneSplitter({
  width, min, max, onChange, onReset, controls,
}: {
  width: number;
  min: number;
  max: number;
  onChange: (width: number) => void;
  onReset?: () => void;
  /** id of the pane being resized */
  controls?: string;
}) {
  const { t } = useTranslation();
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    if (!dragging) return;
    const previous = document.body.style.cursor;
    document.body.style.cursor = "col-resize";
    return () => { document.body.style.cursor = previous; };
  }, [dragging]);

  function handlePointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    dragRef.current = { startX: event.clientX, startWidth: width };
    setDragging(true);
  }

  function handlePointerMove(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current;
    if (!drag) return;
    onChange(drag.startWidth + event.clientX - drag.startX);
  }

  function endDrag(event: PointerEvent<HTMLDivElement>) {
    if (!dragRef.current) return;
    dragRef.current = null;
    setDragging(false);
    event.currentTarget.releasePointerCapture?.(event.pointerId);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const step = event.shiftKey ? 48 : 16;
    if (event.key === "ArrowLeft") onChange(width - step);
    else if (event.key === "ArrowRight") onChange(width + step);
    else if (event.key === "Home") onChange(min);
    else if (event.key === "End") onChange(max);
    else return;
    event.preventDefault();
    event.stopPropagation();
  }

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("layout.resizeList", "Resize list")}
      aria-controls={controls}
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      title={t("layout.resizeListHint", "Drag to resize; double-click to reset")}
      className="pane-splitter"
      data-dragging={dragging || undefined}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDoubleClick={onReset}
      onKeyDown={handleKeyDown}
    />
  );
}
