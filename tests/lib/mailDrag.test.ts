import { afterEach, describe, expect, it, vi } from "vitest";
import { MAIL_DRAG_EXPAND_EVENT, trackMailDrag } from "../../src/lib/mailDrag";

function pointer(type: string, x: number, y: number) {
  const event = new MouseEvent(type, { clientX: x, clientY: y, bubbles: true });
  window.dispatchEvent(event);
}

describe("trackMailDrag", () => {
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("announces a collapsed group after the dragged mail hovers it, and never drops on it", () => {
    vi.useFakeTimers();
    const group = document.createElement("div");
    group.dataset.dropExpand = "acc:clients";
    document.body.appendChild(group);
    document.elementFromPoint = vi.fn(() => group);
    const expanded: unknown[] = [];
    const onExpand = (event: Event) => expanded.push((event as CustomEvent).detail);
    document.addEventListener(MAIL_DRAG_EXPAND_EVENT, onExpand);
    const onDrop = vi.fn();

    trackMailDrag({ button: 0, pointerType: "mouse", clientX: 0, clientY: 0 } as React.PointerEvent, {
      getPayload: () => ({ items: ["m-1"], label: "1 message" }),
      onDrop,
    });
    pointer("pointermove", 50, 50);
    vi.advanceTimersByTime(300);
    expect(expanded).toEqual([]);
    vi.advanceTimersByTime(400);
    expect(expanded).toEqual(["acc:clients"]);

    pointer("pointerup", 50, 50);
    expect(onDrop).not.toHaveBeenCalled();
    document.removeEventListener(MAIL_DRAG_EXPAND_EVENT, onExpand);
  });
});
