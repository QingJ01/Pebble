import { fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { useRovingList } from "../../src/hooks/useRovingList";

function Harness({ ids, onSelect }: { ids: string[]; onSelect: (id: string) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<string | null>("b");
  const roving = useRovingList({
    ids,
    selectedId: selected,
    onSelect: (id) => {
      setSelected(id);
      onSelect(id);
    },
    containerRef: ref,
    scrollToIndex: () => {},
  });
  return (
    <div ref={ref} role="listbox" {...roving.containerProps}>
      {ids.map((id) => (
        <div
          key={id}
          role="option"
          aria-selected={id === selected}
          data-row-id={id}
          tabIndex={roving.tabIndexFor(id)}
          onFocus={() => roving.setActiveId(id)}
        >
          {id}
        </div>
      ))}
    </div>
  );
}

describe("useRovingList", () => {
  it("keeps one tab stop on the selected row and moves it with arrows, J/K, Home/End", () => {
    const onSelect = vi.fn();
    render(<Harness ids={["a", "b", "c"]} onSelect={onSelect} />);

    const rows = () => screen.getAllByRole("option");
    expect(rows().map((row) => row.getAttribute("tabindex"))).toEqual(["-1", "0", "-1"]);
    expect(screen.getByRole("listbox").getAttribute("tabindex")).toBe("-1");

    fireEvent.keyDown(rows()[1], { key: "ArrowDown" });
    expect(onSelect).toHaveBeenLastCalledWith("c");
    expect(document.activeElement).toBe(rows()[2]);
    expect(rows().map((row) => row.getAttribute("tabindex"))).toEqual(["-1", "-1", "0"]);

    fireEvent.keyDown(rows()[2], { key: "k" });
    expect(onSelect).toHaveBeenLastCalledWith("b");

    fireEvent.keyDown(rows()[1], { key: "Home" });
    expect(onSelect).toHaveBeenLastCalledWith("a");
    fireEvent.keyDown(rows()[0], { key: "End" });
    expect(onSelect).toHaveBeenLastCalledWith("c");
  });
});
