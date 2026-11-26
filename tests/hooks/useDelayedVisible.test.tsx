import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useDelayedVisible } from "../../src/hooks/useDelayedVisible";
import Spinner from "../../src/components/Spinner";

function Probe({ delay }: { delay?: number }) {
  return <span>{useDelayedVisible(delay) ? "shown" : "hidden"}</span>;
}

describe("useDelayedVisible", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("stays hidden until the delay passes", () => {
    render(<Probe delay={200} />);
    expect(screen.getByText("hidden")).toBeTruthy();
    act(() => { vi.advanceTimersByTime(199); });
    expect(screen.getByText("hidden")).toBeTruthy();
    act(() => { vi.advanceTimersByTime(1); });
    expect(screen.getByText("shown")).toBeTruthy();
  });

  it("is visible immediately without a delay", () => {
    render(<Probe delay={0} />);
    expect(screen.getByText("shown")).toBeTruthy();
  });

  it("holds the spinner label back for fast loads", () => {
    render(<Spinner delayMs={150} label="Loading" />);
    expect(screen.queryByText("Loading")).toBeNull();
    expect(screen.getByRole("status").getAttribute("aria-busy")).toBe("true");
    act(() => { vi.advanceTimersByTime(150); });
    expect(screen.getByText("Loading")).toBeTruthy();
  });
});
