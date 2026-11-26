import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import Popover from "../../../src/components/ui/Popover";
import Dialog from "../../../src/components/ui/Dialog";
import SnoozePopover from "../../../src/features/inbox/SnoozePopover";
import { resetOverlayStackForTests } from "../../../src/lib/overlayStack";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({ t: (_key: string, fallback?: string) => fallback ?? _key }),
}));
vi.mock("../../../src/lib/api", () => ({ snoozeMessage: vi.fn() }));

function PopoverWithDialog() {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [popoverOpen, setPopoverOpen] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  return (
    <>
      <button ref={anchorRef} onClick={() => setPopoverOpen(true)}>open popover</button>
      <Popover open={popoverOpen} onClose={() => setPopoverOpen(false)} anchorRef={anchorRef} aria-label="test popover">
        <button onClick={() => setDialogOpen(true)}>open dialog</button>
      </Popover>
      <Dialog open={dialogOpen} onClose={() => setDialogOpen(false)} ariaLabel="test dialog">
        <button>inside dialog</button>
      </Dialog>
    </>
  );
}

function SnoozeHarness() {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button ref={anchorRef} onClick={() => setOpen((v) => !v)}>snooze trigger</button>
      <SnoozePopover open={open} anchorRef={anchorRef} messageId="m1" onClose={() => setOpen(false)} onSnoozed={() => setOpen(false)} />
    </>
  );
}

describe("Popover overlay behaviour", () => {
  afterEach(() => {
    cleanup();
    resetOverlayStackForTests();
  });

  it("only the top-most overlay handles Escape", () => {
    render(<PopoverWithDialog />);
    fireEvent.click(screen.getByText("open popover"));
    fireEvent.click(screen.getByText("open dialog"));
    expect(screen.getByRole("dialog", { name: "test dialog" })).toBeTruthy();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "test dialog" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "test popover" })).toBeTruthy();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "test popover" })).toBeNull();
  });

  it("ignores outside clicks while a dialog is stacked above it", () => {
    render(<PopoverWithDialog />);
    fireEvent.click(screen.getByText("open popover"));
    fireEvent.click(screen.getByText("open dialog"));
    fireEvent.mouseDown(screen.getByText("inside dialog"));
    expect(screen.getByRole("dialog", { name: "test popover" })).toBeTruthy();

    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("dialog", { name: "test popover" })).toBeNull();
  });

  it("does not let Escape reach global listeners", () => {
    const globalListener = vi.fn();
    document.addEventListener("keydown", globalListener);
    try {
      render(<PopoverWithDialog />);
      fireEvent.click(screen.getByText("open popover"));
      fireEvent.keyDown(document, { key: "Escape" });
      expect(globalListener).not.toHaveBeenCalled();
    } finally {
      document.removeEventListener("keydown", globalListener);
    }
  });
});

describe("SnoozePopover", () => {
  afterEach(() => {
    cleanup();
    resetOverlayStackForTests();
  });

  it("is a keyboard menu that returns focus to its trigger on Escape", async () => {
    render(<SnoozeHarness />);
    const trigger = screen.getByText("snooze trigger");
    trigger.focus();
    fireEvent.click(trigger);

    const menu = screen.getByRole("menu", { name: "Snooze until" });
    const items = screen.getAllByRole("menuitem");
    expect(items).toHaveLength(4);
    await act(async () => {});
    expect(document.activeElement).toBe(items[0]);

    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(items[1]);
    fireEvent.keyDown(menu, { key: "End" });
    expect(document.activeElement).toBe(items[3]);
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(items[0]);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});

function DialogHarness() {
  const [open, setOpen] = useState(false);
  const [, setTick] = useState(0);
  return (
    <>
      <button onClick={() => setOpen(true)}>open</button>
      <Dialog open={open} onClose={() => setOpen(false)} ariaLabel="harness">
        <input aria-label="field" />
        <button onClick={() => setTick((n) => n + 1)}>rerender parent</button>
      </Dialog>
    </>
  );
}

describe("Dialog focus management", () => {
  afterEach(() => {
    cleanup();
    resetOverlayStackForTests();
  });

  it("moves focus in once, keeps it across parent re-renders and restores it on close", () => {
    render(<DialogHarness />);
    const trigger = screen.getByText("open");
    trigger.focus();
    fireEvent.click(trigger);
    expect(document.activeElement).toBe(screen.getByLabelText("field"));

    const rerender = screen.getByText("rerender parent");
    rerender.focus();
    fireEvent.click(rerender);
    expect(document.activeElement).toBe(rerender);

    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByLabelText("field"));

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});
