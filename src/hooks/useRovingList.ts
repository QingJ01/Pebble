/**
 * useRovingList — one tabbable row (roving tabindex) for a virtualized listbox,
 * with ↑/↓, J/K (the next/prev-message bindings), Home/End and PageUp/PageDown.
 * Moving also selects (opens) the row, like MessageList.
 *
 *   const roving = useRovingList({ ids, selectedId, onSelect, containerRef, scrollToIndex });
 *   <div ref={containerRef} role="listbox" {...roving.containerProps}>
 *     <Row tabIndex={roving.tabIndexFor(id)} onRowFocus={roving.setActiveId} data-row-id={id} />
 */
import { useCallback, useMemo, useState, type FocusEvent, type KeyboardEvent, type RefObject } from "react";
import { eventToKeyString, useShortcutStore } from "@/stores/shortcut.store";

const PAGE_STEP = 10;

interface Options {
  ids: string[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  containerRef: RefObject<HTMLElement | null>;
  scrollToIndex: (index: number) => void;
  /** Ids whose rows are currently rendered (virtualized lists). Defaults to all. */
  renderedIds?: string[];
}

function matches(event: KeyboardEvent, binding: string | undefined) {
  return Boolean(binding) && eventToKeyString(event.nativeEvent).toLowerCase() === binding!.toLowerCase();
}

export function useRovingList({ ids, selectedId, onSelect, containerRef, scrollToIndex, renderedIds }: Options) {
  const [activeId, setActiveId] = useState<string | null>(null);
  const indexById = useMemo(() => new Map(ids.map((id, index) => [id, index])), [ids]);
  const effectiveActiveId =
    (activeId && indexById.has(activeId) ? activeId : null)
    ?? (selectedId && indexById.has(selectedId) ? selectedId : null)
    ?? ids[0]
    ?? null;

  const focusRow = useCallback((id: string) => {
    const index = indexById.get(id);
    if (index === undefined) return;
    scrollToIndex(index);
    const tryFocus = (attempt: number) => {
      const row = Array.from(containerRef.current?.querySelectorAll<HTMLElement>("[data-row-id]") ?? [])
        .find((el) => el.dataset.rowId === id);
      if (row) row.focus({ preventScroll: true });
      else if (attempt < 3) requestAnimationFrame(() => tryFocus(attempt + 1));
    };
    tryFocus(0);
  }, [indexById, scrollToIndex, containerRef]);

  const moveTo = useCallback((index: number) => {
    if (ids.length === 0) return;
    const id = ids[Math.max(0, Math.min(ids.length - 1, index))];
    setActiveId(id);
    onSelect(id);
    focusRow(id);
  }, [ids, onSelect, focusRow]);

  function onKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (event.defaultPrevented || event.altKey) return;
    const target = event.target as HTMLElement;
    if (target !== event.currentTarget && target.getAttribute("role") !== "option") return;
    const current = effectiveActiveId ? indexById.get(effectiveActiveId) ?? 0 : 0;
    const bindings = useShortcutStore.getState().bindings;
    let next: number | null = null;
    if (event.key === "ArrowDown" || matches(event, bindings["next-message"])) next = current + 1;
    else if (event.key === "ArrowUp" || matches(event, bindings["prev-message"])) next = current - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = ids.length - 1;
    else if (event.key === "PageDown") next = current + PAGE_STEP;
    else if (event.key === "PageUp") next = current - PAGE_STEP;
    if (next === null) return;
    event.preventDefault();
    moveTo(next);
  }

  const activeRowRendered = effectiveActiveId !== null
    && (renderedIds ? renderedIds.includes(effectiveActiveId) : true);

  return {
    activeId: effectiveActiveId,
    setActiveId,
    tabIndexFor: (id: string) => (id === effectiveActiveId ? 0 : -1),
    containerProps: {
      // The container is the tab stop only while the active row is not rendered.
      tabIndex: activeRowRendered ? -1 : 0,
      "data-shortcut-scope": "list",
      onKeyDown,
      onFocus: (event: FocusEvent<HTMLElement>) => {
        if (event.target === event.currentTarget && effectiveActiveId) focusRow(effectiveActiveId);
      },
    },
  };
}
