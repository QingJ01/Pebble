import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SelectionActionPopover from "../../src/components/SelectionActionPopover";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

describe("SelectionActionPopover", () => {
  it("clears the copied feedback timer when unmounted", async () => {
    vi.useFakeTimers();
    try {
      const view = render(<SelectionActionPopover text="test" position={{ x: 0, y: 0 }}
        onTranslate={vi.fn()} onSearch={vi.fn()} onCreateRule={vi.fn()}
        onAddToKanbanNote={vi.fn()} onClose={vi.fn()} />);
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Copy selected text" })); });
      expect(vi.getTimerCount()).toBeGreaterThan(0);
      view.unmount();
      // Flush jsdom's own 0ms selectionchange timers (from focus moves); the
      // 1500ms "copied" timer must already be gone.
      act(() => { vi.advanceTimersByTime(0); });
      expect(vi.getTimerCount()).toBe(0);
    } finally { vi.useRealTimers(); }
  });
  beforeEach(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });
  });

  it("offers copy as the primary selected-text action", async () => {
    render(
      <SelectionActionPopover
        text="selected email text"
        position={{ x: 100, y: 120 }}
        onTranslate={vi.fn()}
        onSearch={vi.fn()}
        onCreateRule={vi.fn()}
        onAddToKanbanNote={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Copy selected text" }));

    await waitFor(() => {
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("selected email text");
    });
    expect(await screen.findByRole("button", { name: "Copied selected text" })).toBeTruthy();
  });

  it("keeps translate behind the secondary actions menu", () => {
    const onTranslate = vi.fn();
    const onSearch = vi.fn();
    const onCreateRule = vi.fn();
    const onAddToKanbanNote = vi.fn();
    render(
      <SelectionActionPopover
        text="selected email text"
        position={{ x: 100, y: 120 }}
        onTranslate={onTranslate}
        onSearch={onSearch}
        onCreateRule={onCreateRule}
        onAddToKanbanNote={onAddToKanbanNote}
        onClose={vi.fn()}
      />,
    );

    expect(screen.queryByRole("menuitem", { name: "Translate selected text" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Search selected text" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Create rule from selected text" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Add selected text as kanban note" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "More selected-text actions" }));

    expect(screen.getByRole("menuitem", { name: "Search selected text" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Create rule from selected text" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Add selected text as kanban note" })).toBeTruthy();

    fireEvent.click(screen.getByRole("menuitem", { name: "Translate selected text" }));

    expect(onTranslate).toHaveBeenCalledWith("selected email text", { x: 100, y: 120 });
    expect(onSearch).not.toHaveBeenCalled();
    expect(onCreateRule).not.toHaveBeenCalled();
    expect(onAddToKanbanNote).not.toHaveBeenCalled();
  });

  it("runs secondary selected-text actions with the selected text", () => {
    const onSearch = vi.fn();
    const onCreateRule = vi.fn();
    const onAddToKanbanNote = vi.fn();
    const props = {
      text: "selected email text",
      position: { x: 100, y: 120 },
      onTranslate: vi.fn(),
      onSearch,
      onCreateRule,
      onAddToKanbanNote,
      onClose: vi.fn(),
    };
    let view = render(<SelectionActionPopover {...props} />);

    fireEvent.click(screen.getByRole("button", { name: "More selected-text actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Search selected text" }));

    expect(onSearch).toHaveBeenCalledWith("selected email text");

    view.unmount();
    view = render(<SelectionActionPopover {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "More selected-text actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Create rule from selected text" }));

    expect(onCreateRule).toHaveBeenCalledWith("selected email text");

    view.unmount();
    render(<SelectionActionPopover {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "More selected-text actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Add selected text as kanban note" }));

    expect(onAddToKanbanNote).toHaveBeenCalledWith("selected email text");
  });
});
