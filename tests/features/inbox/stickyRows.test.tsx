import { describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import { useStickyRows } from "@/features/inbox/stickyRows";

interface Row { id: string; date: number; read: boolean }
const row = (id: string, date: number, read = false): Row => ({ id, date, read });

function useRows(props: { rows: Row[]; openedKey: string | null; resetKey?: string; enabled?: boolean }) {
  return useStickyRows<Row>({
    rows: props.rows,
    enabled: props.enabled ?? true,
    resetKey: props.resetKey ?? "k",
    openedKey: props.openedKey,
    keyOf: (r) => r.id,
    markRead: (r) => ({ ...r, read: true }),
    compareNewestFirst: (a, b) => b.date - a.date,
  });
}

describe("useStickyRows", () => {
  it("keeps the opened row (marked read) after the server list drops it", () => {
    const a = row("a", 3);
    const b = row("b", 2);
    const c = row("c", 1);
    const { result, rerender } = renderHook(useRows, { initialProps: { rows: [a, b, c], openedKey: "b" } });
    expect(result.current.map((r) => r.id)).toEqual(["a", "b", "c"]);
    rerender({ rows: [a, c], openedKey: "b" });
    expect(result.current.map((r) => r.id)).toEqual(["a", "b", "c"]);
    expect(result.current[1].read).toBe(true);
  });

  it("drops kept rows when the reset key changes or the filter is off", () => {
    const a = row("a", 3);
    const b = row("b", 2);
    const { result, rerender } = renderHook(useRows, { initialProps: { rows: [a, b], openedKey: "b" } });
    rerender({ rows: [a], openedKey: "b", resetKey: "other" });
    expect(result.current.map((r) => r.id)).toEqual(["a"]);
    rerender({ rows: [a], openedKey: null, enabled: false });
    expect(result.current.map((r) => r.id)).toEqual(["a"]);
  });
});
