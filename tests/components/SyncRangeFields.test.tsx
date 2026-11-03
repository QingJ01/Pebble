import { useState } from "react";
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import SyncRangeFields, { syncRangeDays, syncRangeDraft } from "../../src/components/SyncRangeFields";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

describe("sync range", () => {
  it.each([null, 30, 90, 365, 7, 36500])("round-trips persisted range %s", (days) => {
    expect(syncRangeDays(syncRangeDraft(days))).toBe(days);
  });

  it.each(["", "0", "-1", "1.5", "36501", "Infinity", "1e2", "NaN"])("rejects invalid custom days %s", (customDays) => {
    expect(syncRangeDays({ preset: "custom", customDays })).toBeUndefined();
  });

  it("retains custom input while switching presets", () => {
    function Harness() {
      const [value, setValue] = useState(syncRangeDraft(null));
      return <SyncRangeFields value={value} onChange={setValue} />;
    }
    render(<Harness />);
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "custom" } });
    expect(screen.getByRole("alert")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Number of days"), { target: { value: "45" } });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "365" } });
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "custom" } });
    expect(screen.getByLabelText("Number of days")).toHaveValue(45);
  });

  it("makes the unsupported POP3 behavior explicit", () => {
    render(<SyncRangeFields value={syncRangeDraft(30)} onChange={vi.fn()} unsupported />);
    expect(screen.getByLabelText("Sync range")).toBeDisabled();
    expect(screen.getByLabelText("Sync range")).toHaveValue("all");
    expect(screen.getByText(/POP3 does not support/)).toBeInTheDocument();
  });
});
