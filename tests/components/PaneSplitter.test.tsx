import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import PaneSplitter, { usePaneWidth } from "../../src/components/PaneSplitter";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback?: string) => fallback ?? _key }),
}));

describe("usePaneWidth", () => {
  afterEach(() => { window.localStorage.clear(); vi.restoreAllMocks(); });

  it("restores, clamps and persists the width", () => {
    window.localStorage.setItem("pane", "9999");
    const { result } = renderHook(() => usePaneWidth("pane", 340, 260, 560));
    expect(result.current.width).toBe(560);
    act(() => result.current.setWidth(100));
    expect(result.current.width).toBe(260);
    expect(window.localStorage.getItem("pane")).toBe("260");
    act(() => result.current.reset());
    expect(result.current.width).toBe(340);
    expect(window.localStorage.getItem("pane")).toBeNull();
  });

  it("falls back to the default when storage is unavailable", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("blocked"); });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked"); });
    const { result } = renderHook(() => usePaneWidth("pane", 340, 260, 560));
    expect(result.current.width).toBe(340);
    act(() => result.current.setWidth(400));
    expect(result.current.width).toBe(400);
  });
});

describe("PaneSplitter", () => {
  it("is a focusable separator that resizes with the arrow keys", () => {
    const onChange = vi.fn();
    render(<PaneSplitter width={340} min={260} max={560} onChange={onChange} />);
    const separator = screen.getByRole("separator", { name: "Resize list" });
    expect(separator.getAttribute("aria-valuenow")).toBe("340");
    fireEvent.keyDown(separator, { key: "ArrowRight" });
    expect(onChange).toHaveBeenLastCalledWith(356);
    fireEvent.keyDown(separator, { key: "ArrowLeft", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(292);
    fireEvent.keyDown(separator, { key: "End" });
    expect(onChange).toHaveBeenLastCalledWith(560);
  });

  it("resets on double-click", () => {
    const onReset = vi.fn();
    render(<PaneSplitter width={340} min={260} max={560} onChange={vi.fn()} onReset={onReset} />);
    fireEvent.doubleClick(screen.getByRole("separator"));
    expect(onReset).toHaveBeenCalledOnce();
  });
});
