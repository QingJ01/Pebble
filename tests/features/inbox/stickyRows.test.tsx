import { describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useStickyRows } from "@/features/inbox/stickyRows";

interface Row { id: string; date: number; read: boolean; starred?: boolean }
const row = (id: string, date: number, read = false): Row => ({ id, date, read });

type Patch = (rows: Row[]) => Row[];
function patchBus() {
  const listeners = new Set<(t: Patch) => void>();
  return {
    subscribe: (l: (t: Patch) => void) => { listeners.add(l); return () => { listeners.delete(l); }; },
    emit: (t: Patch) => act(() => { for (const l of listeners) l(t); }),
  };
}

function setup(bus = patchBus()) {
  return renderHook(
    (props: { rows: Row[]; openedKey: string | null; resetKey?: string; enabled?: boolean }) =>
      useStickyRows<Row>({
        rows: props.rows,
        enabled: props.enabled ?? true,
        resetKey: props.resetKey ?? "k",
        openedKey: props.openedKey,
        keyOf: (r) => r.id,
        markRead: (r) => ({ ...r, read: true }),
        compareNewestFirst: (a, b) => b.date - a.date,
        subscribePatches: bus.subscribe,
      }),
    { initialProps: { rows: [] as Row[], openedKey: null as string | null } },
  );
}

const a = row("a", 3);
const b = row("b", 2);
const c = row("c", 1);

describe("useStickyRows", () => {
  it("keeps the open row (marked read) after the server list drops it", () => {
    const { result, rerender } = setup();
    rerender({ rows: [a, b, c], openedKey: "b" });
    rerender({ rows: [a, c], openedKey: "b" });
    expect(result.current.map((r) => r.id)).toEqual(["a", "b", "c"]);
    expect(result.current[1].read).toBe(true);
  });

  it("drops the row once the reader closes or moves on", () => {
    const { result, rerender } = setup();
    rerender({ rows: [a, b, c], openedKey: "b" });
    rerender({ rows: [a, c], openedKey: null });
    expect(result.current.map((r) => r.id)).toEqual(["a", "c"]);
    rerender({ rows: [a, c], openedKey: "b" });
    expect(result.current.map((r) => r.id)).toEqual(["a", "c"]);
  });

  it("forgets the row when the filter is turned off and on again", () => {
    const { result, rerender } = setup();
    rerender({ rows: [a, b], openedKey: "b" });
    rerender({ rows: [a], openedKey: "b", enabled: false });
    rerender({ rows: [a], openedKey: "b", enabled: true });
    expect(result.current.map((r) => r.id)).toEqual(["a"]);
  });

  it("applies list patches to the kept row, including removal", () => {
    const bus = patchBus();
    const { result, rerender } = setup(bus);
    rerender({ rows: [a, b], openedKey: "b" });
    rerender({ rows: [a], openedKey: "b" });
    bus.emit((rows) => rows.map((r) => (r.id === "b" ? { ...r, starred: true } : r)));
    expect(result.current.find((r) => r.id === "b")?.starred).toBe(true);
    bus.emit((rows) => rows.filter((r) => r.id !== "b"));
    expect(result.current.map((r) => r.id)).toEqual(["a"]);
  });
});
