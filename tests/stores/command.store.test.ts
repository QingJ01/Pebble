import { vi, describe, it, expect, beforeEach } from "vitest";
import { useCommandStore } from "../../src/stores/command.store";
import type { Command } from "../../src/stores/command.store";

describe("CommandStore", () => {
  const mockCommands: Command[] = [
    { id: "nav:inbox", name: "Go to Inbox", category: "Navigation", execute: vi.fn() },
    { id: "nav:kanban", name: "Go to Kanban", category: "Navigation", execute: vi.fn() },
    { id: "mail:star", name: "Toggle Star", shortcut: "S", category: "Mail", execute: vi.fn() },
  ];

  beforeEach(() => {
    vi.clearAllMocks();
    useCommandStore.setState({
      isOpen: false,
      query: "",
      commands: [],
      filteredCommands: [],
    });
  });

  it("registerCommands populates both commands and filteredCommands", () => {
    useCommandStore.getState().registerCommands(mockCommands);

    expect(useCommandStore.getState().commands).toHaveLength(3);
    expect(useCommandStore.getState().filteredCommands).toHaveLength(3);
  });

  it("open sets isOpen and resets query", () => {
    useCommandStore.getState().registerCommands(mockCommands);
    useCommandStore.getState().open();

    expect(useCommandStore.getState().isOpen).toBe(true);
    expect(useCommandStore.getState().query).toBe("");
    expect(useCommandStore.getState().filteredCommands).toHaveLength(3);
  });

  it("close sets isOpen to false", () => {
    useCommandStore.getState().registerCommands(mockCommands);
    useCommandStore.getState().open();
    useCommandStore.getState().close();

    expect(useCommandStore.getState().isOpen).toBe(false);
  });

  it("setQuery filters commands case-insensitively", () => {
    useCommandStore.getState().registerCommands(mockCommands);
    useCommandStore.getState().setQuery("inbox");

    expect(useCommandStore.getState().filteredCommands).toHaveLength(1);
    expect(useCommandStore.getState().filteredCommands[0].id).toBe("nav:inbox");
  });

  it("setQuery filters by category", () => {
    useCommandStore.getState().registerCommands(mockCommands);
    useCommandStore.getState().setQuery("mail");

    expect(useCommandStore.getState().filteredCommands).toHaveLength(1);
    expect(useCommandStore.getState().filteredCommands[0].id).toBe("mail:star");
  });

  it("empty query returns all commands", () => {
    useCommandStore.getState().registerCommands(mockCommands);
    useCommandStore.getState().setQuery("inbox");
    useCommandStore.getState().setQuery("");

    expect(useCommandStore.getState().filteredCommands).toHaveLength(3);
  });

  it("execute calls command function and closes palette", async () => {
    useCommandStore.getState().registerCommands(mockCommands);
    useCommandStore.getState().open();

    await useCommandStore.getState().execute("nav:inbox");

    expect(mockCommands[0].execute).toHaveBeenCalled();
    expect(useCommandStore.getState().isOpen).toBe(false);
  });

  it("execute with unknown command does nothing", async () => {
    useCommandStore.getState().registerCommands(mockCommands);
    await useCommandStore.getState().execute("unknown:cmd");

    // No error thrown, state unchanged
    expect(useCommandStore.getState().isOpen).toBe(false);
  });
});
