import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { profileLocalStorage } from "@/lib/profileStorage";

/**
 * Width of a resizable left pane, persisted per profile under `storageKey`.
 * Writes are debounced so a drag doesn't hit storage on every pointermove;
 * a write still pending when the pane unmounts is flushed. Widths saved by
 * older versions (unscoped localStorage) are still read.
 * Storage failures (private mode, quota, blocked) are ignored.
 */
export function usePaneWidth(storageKey: string, defaultWidth: number, min: number, max: number) {
  const clamp = useCallback((value: number) => Math.round(Math.min(max, Math.max(min, value))), [min, max]);
  const [width, setWidthState] = useState(() => {
    try {
      const raw = profileLocalStorage.getItem(storageKey) ?? window.localStorage.getItem(storageKey);
      const stored = Number(raw);
      return raw !== null && Number.isFinite(stored) && stored > 0 ? clamp(stored) : defaultWidth;
    } catch {
      return defaultWidth;
    }
  });
  // Only widths the user set are persisted (not the initial or reset width).
  const dirtyRef = useRef(false);
  const pendingRef = useRef<number | null>(null);

  const persist = useCallback((value: number) => {
    try {
      profileLocalStorage.setItem(storageKey, String(value));
    } catch {
      // Persisting is best-effort.
    }
  }, [storageKey]);

  useEffect(() => {
    if (!dirtyRef.current) return;
    pendingRef.current = width;
    const handle = setTimeout(() => {
      pendingRef.current = null;
      persist(width);
    }, 250);
    return () => clearTimeout(handle);
  }, [persist, width]);

  // Leaving the view right after a drag must not lose the new width.
  useEffect(() => () => {
    if (pendingRef.current !== null) {
      persist(pendingRef.current);
      pendingRef.current = null;
    }
  }, [persist]);

  const setWidth = useCallback((value: number) => {
    dirtyRef.current = true;
    setWidthState(clamp(value));
  }, [clamp]);

  const reset = useCallback(() => {
    dirtyRef.current = false;
    pendingRef.current = null;
    setWidthState(defaultWidth);
    try {
      profileLocalStorage.removeItem(storageKey);
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
