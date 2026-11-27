/**
 * Pointer-based drag of mail rows onto sidebar folders (#80).
 *
 * Native HTML5 drag-and-drop is unreliable inside the Tauri webview (the
 * window's file-drop handler swallows it on Windows), so this follows the
 * pointer itself: a press that moves past a small threshold becomes a drag,
 * a ghost follows the pointer, and the element under it marked with
 * `data-drop-folder-id` is the drop target.
 */

const DRAG_THRESHOLD_PX = 6;
const TARGET_SELECTOR = "[data-drop-folder-id]";

export interface MailDragOptions<T> {
  /** Called once the press turns into a drag; null cancels it. */
  getPayload: () => { items: T[]; label: string } | null;
  /** Whether `folderId` can take the items (hover styling only). */
  canDrop?: (items: T[], folderId: string) => boolean;
  onDrop: (items: T[], folderId: string) => void;
}

function targetAt(x: number, y: number): HTMLElement | null {
  const element = document.elementFromPoint(x, y);
  return element?.closest<HTMLElement>(TARGET_SELECTOR) ?? null;
}

/**
 * Start tracking a primary-button press on a mail row. Returns immediately;
 * the drag (if any) runs on window listeners until the pointer is released.
 */
export function trackMailDrag<T>(event: React.PointerEvent, options: MailDragOptions<T>): void {
  if (event.button !== 0 || event.pointerType === "touch") return;
  const startX = event.clientX;
  const startY = event.clientY;
  let items: T[] | null = null;
  let ghost: HTMLDivElement | null = null;
  let hovered: HTMLElement | null = null;
  let hoveredValid = false;
  /** A drag began (and may since have been cancelled with Escape). */
  let dragStarted = false;

  const setHovered = (next: HTMLElement | null) => {
    if (next === hovered) return;
    hovered?.classList.remove("is-drop-target", "is-drop-invalid");
    hovered = next;
    if (!hovered || !items) return;
    const folderId = hovered.dataset.dropFolderId!;
    hoveredValid = options.canDrop?.(items, folderId) ?? true;
    hovered.classList.add(hoveredValid ? "is-drop-target" : "is-drop-invalid");
  };

  const begin = () => {
    const payload = options.getPayload();
    if (!payload || payload.items.length === 0) return false;
    items = payload.items;
    dragStarted = true;
    ghost = document.createElement("div");
    ghost.className = "mail-drag-ghost";
    ghost.textContent = payload.label;
    ghost.setAttribute("aria-hidden", "true");
    document.body.appendChild(ghost);
    document.body.classList.add("is-dragging-mail");
    window.getSelection()?.removeAllRanges();
    return true;
  };

  const move = (e: PointerEvent) => {
    if (!items) {
      if (Math.hypot(e.clientX - startX, e.clientY - startY) < DRAG_THRESHOLD_PX) return;
      if (!begin()) {
        cleanup();
        return;
      }
    }
    e.preventDefault();
    if (ghost) ghost.style.transform = `translate(${e.clientX + 12}px, ${e.clientY + 12}px)`;
    setHovered(targetAt(e.clientX, e.clientY));
  };

  const suppressClick = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
  };

  const finish = (e: PointerEvent) => {
    const dragged = items;
    const target = dragged ? targetAt(e.clientX, e.clientY) : null;
    const valid = target === hovered ? hoveredValid : false;
    cleanup();
    if (dragStarted) {
      // The release ends a drag (even one cancelled with Escape), not a
      // click on the row underneath.
      window.addEventListener("click", suppressClick, { capture: true, once: true });
      window.setTimeout(() => window.removeEventListener("click", suppressClick, { capture: true }), 0);
    }
    if (dragged && target && valid) options.onDrop(dragged, target.dataset.dropFolderId!);
  };

  const keydown = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || !items) return;
    e.preventDefault();
    e.stopPropagation();
    // Drop the drag but keep waiting for the release, so it isn't a click.
    endDrag();
    window.removeEventListener("pointermove", move);
  };

  function endDrag() {
    setHovered(null);
    ghost?.remove();
    ghost = null;
    if (items) document.body.classList.remove("is-dragging-mail");
    items = null;
  }

  function cleanup() {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", finish);
    window.removeEventListener("pointercancel", cleanup);
    window.removeEventListener("keydown", keydown, { capture: true });
    window.removeEventListener("blur", cleanup);
    endDrag();
  }

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", finish);
  window.addEventListener("pointercancel", cleanup);
  window.addEventListener("keydown", keydown, { capture: true });
  window.addEventListener("blur", cleanup);
}
