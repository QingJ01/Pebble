import { fireEvent, render } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useOverlayLayer } from "../../src/hooks/useOverlayLayer";
import {
  getOverlayDepth,
  hasModalOverlay,
  hasOverlay,
  pushOverlay,
  resetOverlayStackForTests,
} from "../../src/lib/overlayStack";

function Layer({ name, modal, onEscape }: { name: string; modal?: boolean; onEscape: (name: string) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useOverlayLayer({ modal, onEscape: () => onEscape(name), containerRef: ref });
  return (
    <div ref={ref}>
      <button>{name}-a</button>
      <button>{name}-b</button>
    </div>
  );
}

describe("overlay stack", () => {
  afterEach(() => resetOverlayStackForTests());

  it("delivers Escape only to the top-most layer and stops it from propagating", () => {
    const onEscape = vi.fn();
    const outer = vi.fn();
    document.addEventListener("keydown", outer);
    const { rerender, unmount } = render(<><Layer name="lower" onEscape={onEscape} /><Layer name="upper" onEscape={onEscape} modal /></>);

    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onEscape).toHaveBeenCalledTimes(1);
    expect(onEscape).toHaveBeenLastCalledWith("upper");
    expect(outer).not.toHaveBeenCalled();

    rerender(<Layer name="lower" onEscape={onEscape} />);
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onEscape).toHaveBeenLastCalledWith("lower");

    unmount();
    expect(hasOverlay()).toBe(false);
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(outer).toHaveBeenCalledTimes(1);
    document.removeEventListener("keydown", outer);
  });

  it("traps Tab inside the top modal layer", () => {
    const { getByText } = render(<Layer name="m" modal onEscape={() => {}} />);
    getByText("m-b").focus();
    fireEvent.keyDown(document.activeElement!, { key: "Tab" });
    expect(document.activeElement).toBe(getByText("m-a"));
    fireEvent.keyDown(document.activeElement!, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(getByText("m-b"));
  });

  it("keeps Tab inside a popover opened above a modal dialog", () => {
    const { getByText } = render(
      <>
        <Layer name="dialog" modal onEscape={() => {}} />
        <Layer name="popover" onEscape={() => {}} />
      </>,
    );
    getByText("popover-b").focus();
    fireEvent.keyDown(document.activeElement!, { key: "Tab" });
    expect(document.activeElement).toBe(getByText("popover-a"));
  });

  it("leaves Tab alone for a popover with no modal below it", () => {
    const { getByText } = render(<Layer name="popover" onEscape={() => {}} />);
    getByText("popover-b").focus();
    const event = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    document.activeElement!.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it("tracks modal vs non-modal layers", () => {
    const popover = pushOverlay({ modal: false });
    expect(hasOverlay()).toBe(true);
    expect(hasModalOverlay()).toBe(false);
    const dialog = pushOverlay({ modal: true });
    expect(hasModalOverlay()).toBe(true);
    expect(getOverlayDepth()).toBe(2);
    dialog.release();
    popover.release();
    expect(hasOverlay()).toBe(false);
  });
});
