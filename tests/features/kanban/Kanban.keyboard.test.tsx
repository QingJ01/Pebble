import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DndContext, KeyboardSensor, useSensor, useSensors } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates } from "@dnd-kit/sortable";
import type { KanbanCard as KanbanCardType, Message } from "../../../src/lib/api";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallbackOrOptions?: string | Record<string, unknown>) =>
      typeof fallbackOrOptions === "string" ? fallbackOrOptions : key,
    i18n: { language: "en" },
  }),
}));

import KanbanCard from "../../../src/features/kanban/KanbanCard";
import { columnsFromCards, moveAcrossColumns } from "../../../src/features/kanban/KanbanView";
import { reorderColumnInList } from "../../../src/hooks/mutations/useKanbanMutations";

function message(id: string): Message {
  return {
    id,
    account_id: "acc",
    subject: `Subject ${id}`,
    from_name: "Sender",
    from_address: "s@example.com",
    date: 1_700_000_000,
  } as Message;
}

function Board({ onOpen }: { onOpen: (id: string) => void }) {
  const sensors = useSensors(
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
      keyboardCodes: { start: ["Space"], cancel: ["Escape"], end: ["Space", "Enter"] },
    }),
  );
  return (
    <DndContext sensors={sensors}>
      <SortableContext items={["a"]}>
        <KanbanCard id="a" message={message("a")} onRemove={vi.fn()} onOpen={onOpen} />
      </SortableContext>
    </DndContext>
  );
}

describe("KanbanCard keyboard", () => {
  it("opens with Enter and starts a keyboard drag with Space", () => {
    const onOpen = vi.fn();
    render(<Board onOpen={onOpen} />);
    const card = screen.getByRole("button", { name: /Enter to open/ });

    fireEvent.keyDown(card, { key: "Enter", code: "Enter" });
    expect(onOpen).toHaveBeenCalledWith("a");

    card.focus();
    fireEvent.keyDown(card, { key: " ", code: "Space" });
    // dnd-kit marks the active draggable as pressed once the keyboard sensor activates.
    expect(card.getAttribute("aria-pressed")).toBe("true");
  });

  it("uses a real 28px icon button for removing a card", () => {
    render(<Board onOpen={vi.fn()} />);
    const remove = screen.getByRole("button", { name: "kanban.removeFromBoard" });
    expect(remove.className).toContain("icon-btn--sm");
  });
});

describe("kanban drop position", () => {
  const cards: KanbanCardType[] = [
    { message_id: "a", column: "todo", position: 0 } as KanbanCardType,
    { message_id: "b", column: "todo", position: 1 } as KanbanCardType,
    { message_id: "c", column: "waiting", position: 1 } as KanbanCardType,
    { message_id: "d", column: "waiting", position: 0 } as KanbanCardType,
  ];

  it("orders columns by position", () => {
    expect(columnsFromCards(cards)).toEqual({ todo: ["a", "b"], waiting: ["d", "c"], done: [] });
  });

  it("inserts a card dragged over another column's card at that card's position", () => {
    const items = columnsFromCards(cards);
    expect(moveAcrossColumns(items, "a", "c")).toEqual({ todo: ["b"], waiting: ["d", "a", "c"], done: [] });
  });

  it("appends when dropped on an empty column", () => {
    const items = columnsFromCards(cards);
    expect(moveAcrossColumns(items, "a", "done")).toEqual({ todo: ["b"], waiting: ["d", "c"], done: ["a"] });
  });

  it("persists a cross-column drop by reordering the target column", () => {
    const next = reorderColumnInList(cards, "waiting", ["d", "a", "c"]);
    const waiting = next.filter((c) => c.column === "waiting").sort((x, y) => x.position - y.position);
    expect(waiting.map((c) => c.message_id)).toEqual(["d", "a", "c"]);
    expect(next.filter((c) => c.message_id === "a")).toHaveLength(1);
  });
});
