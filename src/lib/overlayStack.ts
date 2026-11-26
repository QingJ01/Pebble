/**
 * Overlay stack — one ordered list of open overlays (dialogs, popovers, menus)
 * so that only the top-most one reacts to Escape / Tab.
 *
 * React components should use `useOverlayLayer` (src/hooks/useOverlayLayer.ts).
 * Low-level API:
 *   const release = pushOverlay({ modal, onEscape, getContainer, trapFocus });
 *   updateOverlay(id, partial) · isTopOverlay(id) · getTopOverlay()
 *   hasOverlay() · hasModalOverlay() · getOverlayDepth()
 *
 * Behaviour (one document-level capture listener, installed while any layer exists):
 * - Escape → the top-most layer's `onEscape` (if it has one) is called and the
 *   event is preventDefault()+stopPropagation()'d, so global shortcuts and
 *   lower overlays never see it. Exception: while focus is inside an expanded
 *   combobox (aria-expanded="true"), Escape is left to that widget first.
 * - Tab / Shift+Tab → if the top layer traps focus (default: modal layers, and
 *   any layer opened above a modal one) and provides `getContainer`, focus
 *   cycles within that container.
 * Layers without `onEscape` still count for isTopOverlay()/hasOverlay(): use
 * that from your own key handlers (`if (!layer.isTop()) return;`).
 */

export interface OverlayLayerOptions {
  /** Modal layers block global shortcuts entirely (see useKeyboard). */
  modal?: boolean;
  /** Called when Escape is pressed while this layer is top-most. */
  onEscape?: () => void;
  /** Container used for the Tab focus trap. */
  getContainer?: () => HTMLElement | null;
  /** Trap Tab inside getContainer(); defaults to `modal`. */
  trapFocus?: boolean;
}

interface OverlayLayer extends OverlayLayerOptions {
  id: number;
}

const FOCUSABLE =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

let layers: OverlayLayer[] = [];
let nextId = 0;
let listening = false;

function focusableIn(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => !el.hasAttribute("disabled") && !el.closest("[hidden],[inert]"),
  );
}

function handleKeyDown(event: KeyboardEvent) {
  const top = layers[layers.length - 1];
  if (!top) return;

  if (event.key === "Escape") {
    if (!top.onEscape) return;
    const active = document.activeElement;
    // Let an open autocomplete/listbox inside the overlay close itself first.
    if (active instanceof HTMLElement && active.closest('[role="combobox"][aria-expanded="true"]')) return;
    event.preventDefault();
    event.stopPropagation();
    top.onEscape();
    return;
  }

  if (event.key === "Tab") {
    // A non-modal layer (popover, menu) above a modal dialog traps too: it is
    // portaled outside the dialog, so leaving it with Tab would escape the modal.
    const aboveModal = layers.slice(0, -1).some((layer) => layer.modal);
    const trap = (top.trapFocus ?? top.modal ?? false) || aboveModal;
    const container = trap ? top.getContainer?.() : null;
    if (!container) return;
    const focusable = focusableIn(container);
    event.preventDefault();
    event.stopPropagation();
    if (focusable.length === 0) return;
    const current = focusable.indexOf(document.activeElement as HTMLElement);
    const next = event.shiftKey
      ? (current <= 0 ? focusable.length - 1 : current - 1)
      : (current === -1 || current === focusable.length - 1 ? 0 : current + 1);
    focusable[next]?.focus();
  }
}

function syncListener() {
  if (typeof document === "undefined") return;
  if (layers.length > 0 && !listening) {
    document.addEventListener("keydown", handleKeyDown, true);
    listening = true;
  } else if (layers.length === 0 && listening) {
    document.removeEventListener("keydown", handleKeyDown, true);
    listening = false;
  }
}

/** Push a layer on top of the stack. Returns a release function (idempotent). */
export function pushOverlay(options: OverlayLayerOptions = {}): { id: number; release: () => void } {
  const layer: OverlayLayer = { ...options, id: ++nextId };
  layers = [...layers, layer];
  syncListener();
  return {
    id: layer.id,
    release: () => {
      layers = layers.filter((l) => l.id !== layer.id);
      syncListener();
    },
  };
}

/** Update a layer's callbacks/flags in place (keeps its stack position). */
export function updateOverlay(id: number, options: Partial<OverlayLayerOptions>) {
  const layer = layers.find((l) => l.id === id);
  if (layer) Object.assign(layer, options);
}

export function isTopOverlay(id: number): boolean {
  return layers.length > 0 && layers[layers.length - 1].id === id;
}

export function getTopOverlay(): Readonly<OverlayLayer> | undefined {
  return layers[layers.length - 1];
}

export function hasOverlay(): boolean {
  return layers.length > 0;
}

export function hasModalOverlay(): boolean {
  return layers.some((l) => l.modal);
}

export function getOverlayDepth(): number {
  return layers.length;
}

/** Test helper: drop every layer and detach the listener. */
export function resetOverlayStackForTests() {
  layers = [];
  syncListener();
}
