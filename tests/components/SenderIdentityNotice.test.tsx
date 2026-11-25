import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SenderIdentityNotice from "../../src/components/SenderIdentityNotice";
import { useAccountsQuery } from "../../src/hooks/queries";
import { profileLocalStorage } from "../../src/lib/profileStorage";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }) }));
vi.mock("../../src/hooks/queries", () => ({ useAccountsQuery: vi.fn() }));
vi.mock("../../src/stores/ui.store", () => ({ useUIStore: { getState: vi.fn() } }));
vi.mock("../../src/lib/profileStorage", () => ({ profileLocalStorage: { getItem: vi.fn(), setItem: vi.fn() } }));

describe("sender identity upgrade notice", () => {
  afterEach(cleanup);
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(useAccountsQuery).mockReturnValue({ data: [
      { id: "smtp", provider: "imap", email: "sender@example.com", display_name: "Public Name", account_label: "Private Description" },
      { id: "outlook", provider: "outlook", email: "outlook@example.com", display_name: "Ignored Name" },
    ] } as ReturnType<typeof useAccountsQuery>);
  });

  it("previews outgoing names without exposing local labels or Outlook overrides", () => {
    render(<SenderIdentityNotice />);
    expect(screen.getByText("Public Name <sender@example.com>")).toBeTruthy();
    expect(screen.queryByText(/Private Description/)).toBeNull();
    expect(screen.queryByText(/Ignored Name/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Got it" }));
    expect(profileLocalStorage.setItem).toHaveBeenCalledWith("pebble-sender-identity-notice-v1", "seen");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("renders as a modal dialog rather than a layout banner", () => {
    render(<SenderIdentityNotice />);
    expect(screen.getByRole("dialog", { name: "Review your sender names" })).toBeTruthy();
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("dismisses with Escape", () => {
    render(<SenderIdentityNotice />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(profileLocalStorage.setItem).toHaveBeenCalledWith("pebble-sender-identity-notice-v1", "seen");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("remains dismissed when opened again in the same profile", () => {
    vi.mocked(profileLocalStorage.getItem).mockReturnValue("seen");
    render(<SenderIdentityNotice />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
