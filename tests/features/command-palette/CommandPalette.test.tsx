import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => (typeof fallback === "string" ? fallback : key),
  }),
}));

vi.mock("@/features/command-palette/commands", () => ({
  COMMAND_SHORTCUT_ACTIONS: { "nav:inbox": "toggle-view-inbox" },
}));

import CommandPalette from "@/features/command-palette/CommandPalette";
import { useCommandStore } from "@/stores/command.store";
import { DEFAULT_BINDINGS, useShortcutStore } from "@/stores/shortcut.store";

const commands = ["nav:inbox", "b", "c", "d"].map((id) => ({
  id,
  name: `Command ${id}`,
  category: "Test",
  shortcut: id === "nav:inbox" ? "Ctrl+Shift+I" : undefined,
  execute: vi.fn(),
}));

describe("CommandPalette", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
    useShortcutStore.setState({ bindings: { ...DEFAULT_BINDINGS, "toggle-view-inbox": "Ctrl+Shift+J" } });
    useCommandStore.setState({ commands, filteredCommands: commands, isOpen: true, query: "" });
  });

  it("keeps the keyboard selection scrolled into view", () => {
    render(<CommandPalette />);
    const input = screen.getByRole("combobox");
    (Element.prototype.scrollIntoView as ReturnType<typeof vi.fn>).mockClear();

    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "ArrowDown" });

    const options = screen.getAllByRole("option");
    expect(options[2].getAttribute("aria-selected")).toBe("true");
    expect(Element.prototype.scrollIntoView).toHaveBeenCalledWith({ block: "nearest" });
  });

  it("moves the single highlight with the pointer", () => {
    render(<CommandPalette />);
    const options = screen.getAllByRole("option");

    fireEvent.mouseMove(options[3]);

    expect(options.filter((o) => o.getAttribute("aria-selected") === "true")).toEqual([options[3]]);
  });

  it("shows the user's current binding, not the hardcoded default", () => {
    render(<CommandPalette />);
    const text = screen.getAllByRole("option")[0].textContent ?? "";
    expect(text).toMatch(/J/);
    expect(text).not.toMatch(/I$/);
  });
});
