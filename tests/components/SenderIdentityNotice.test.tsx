import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SenderIdentityNotice, {
  SENDER_NOTICE_MAX_WAIT_MS,
  SENDER_NOTICE_QUIET_MS,
} from "../../src/components/SenderIdentityNotice";
import { useAccountsQuery } from "../../src/hooks/queries";
import { profileLocalStorage } from "../../src/lib/profileStorage";
import { resetOverlayStackForTests } from "../../src/lib/overlayStack";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }) }));
vi.mock("../../src/hooks/queries", () => ({ useAccountsQuery: vi.fn() }));
vi.mock("../../src/stores/ui.store", () => ({ useUIStore: { getState: vi.fn() } }));
vi.mock("../../src/lib/profileStorage", () => ({ profileLocalStorage: { getItem: vi.fn(), setItem: vi.fn() } }));

describe("sender identity upgrade notice", () => {
  afterEach(() => {
    cleanup();
    resetOverlayStackForTests();
    vi.useRealTimers();
  });
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(useAccountsQuery).mockReturnValue({ data: [
      { id: "smtp", provider: "imap", email: "sender@example.com", display_name: "Public Name", account_label: "Private Description" },
      { id: "outlook", provider: "outlook", email: "outlook@example.com", display_name: "Ignored Name" },
    ] } as ReturnType<typeof useAccountsQuery>);
  });

  it("previews outgoing names without exposing local labels or Outlook overrides", () => {
    render(<SenderIdentityNotice defer={false} />);
    expect(screen.getByText("Public Name <sender@example.com>")).toBeTruthy();
    expect(screen.queryByText(/Private Description/)).toBeNull();
    expect(screen.queryByText(/Ignored Name/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Got it" }));
    expect(profileLocalStorage.setItem).toHaveBeenCalledWith("pebble-sender-identity-notice-v1", "seen");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("renders as a modal dialog with 'Got it' as the focused primary action", () => {
    render(<SenderIdentityNotice defer={false} />);
    expect(screen.getByRole("dialog", { name: "Review your sender names" })).toBeTruthy();
    expect(screen.queryByRole("complementary")).toBeNull();
    const gotIt = screen.getByRole("button", { name: "Got it" });
    expect(document.activeElement).toBe(gotIt);
    expect(gotIt.className).toContain("btn--primary");
    expect(screen.getByRole("button", { name: "Review in account settings" }).className).toContain("btn--secondary");
    expect(screen.getByRole("heading", { level: 3, name: "Outgoing mail will show" })).toBeTruthy();
  });

  it("dismisses with Escape", () => {
    render(<SenderIdentityNotice defer={false} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(profileLocalStorage.setItem).toHaveBeenCalledWith("pebble-sender-identity-notice-v1", "seen");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("remains dismissed when opened again in the same profile", () => {
    vi.mocked(profileLocalStorage.getItem).mockReturnValue("seen");
    render(<SenderIdentityNotice defer={false} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("waits until the user's first interaction has settled before opening", () => {
    vi.useFakeTimers();
    render(<SenderIdentityNotice />);
    expect(screen.queryByRole("dialog")).toBeNull();

    act(() => { vi.advanceTimersByTime(1000); });
    fireEvent.pointerDown(window);
    act(() => { vi.advanceTimersByTime(SENDER_NOTICE_QUIET_MS - 600); });
    fireEvent.keyDown(window, { key: "j" });
    act(() => { vi.advanceTimersByTime(SENDER_NOTICE_QUIET_MS - 600); });
    expect(screen.queryByRole("dialog")).toBeNull();

    act(() => { vi.advanceTimersByTime(1100); });
    expect(screen.getByRole("dialog", { name: "Review your sender names" })).toBeTruthy();
  });

  it("opens on its own after a while when the user never interacts", () => {
    vi.useFakeTimers();
    render(<SenderIdentityNotice />);
    act(() => { vi.advanceTimersByTime(SENDER_NOTICE_MAX_WAIT_MS - 1000); });
    expect(screen.queryByRole("dialog")).toBeNull();
    act(() => { vi.advanceTimersByTime(1500); });
    expect(screen.getByRole("dialog")).toBeTruthy();
  });
});
