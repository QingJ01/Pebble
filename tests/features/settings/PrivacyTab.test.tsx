import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import PrivacyTab from "../../../src/features/settings/PrivacyTab";

const mocks = vi.hoisted(() => ({
  activeAccountId: null as string | null,
  listTrustedSenders: vi.fn(),
  removeTrustedSender: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

vi.mock("../../../src/stores/mail.store", () => ({
  useMailStore: (selector: (state: { activeAccountId: string | null }) => unknown) =>
    selector({ activeAccountId: mocks.activeAccountId }),
}));

vi.mock("../../../src/stores/toast.store", () => ({
  useToastStore: {
    getState: () => ({ addToast: vi.fn() }),
  },
}));

vi.mock("../../../src/lib/api", () => ({
  listTrustedSenders: mocks.listTrustedSenders,
  removeTrustedSender: mocks.removeTrustedSender,
}));

describe("PrivacyTab", () => {
  beforeEach(() => {
    mocks.activeAccountId = null;
    mocks.listTrustedSenders.mockReset();
    mocks.removeTrustedSender.mockReset();
  });

  it("selects relaxed as the default privacy mode when there is no stored preference", () => {
    localStorage.removeItem("pebble-privacy-mode");

    render(<PrivacyTab />);

    expect(screen.getByText("Load external images by default. Trackers are still blocked.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Relaxed" }).getAttribute("style")).toContain(
      "var(--color-accent)",
    );
  });

  it("clears trusted senders when there is no active account", async () => {
    mocks.activeAccountId = "account-1";
    mocks.listTrustedSenders.mockResolvedValue([
      {
        account_id: "account-1",
        email: "trusted@example.com",
        trust_type: "all",
        created_at: 1,
      },
    ]);

    const { rerender } = render(<PrivacyTab />);

    expect(await screen.findByText("trusted@example.com")).toBeTruthy();

    mocks.activeAccountId = null;
    rerender(<PrivacyTab />);

    await waitFor(() => {
      expect(screen.queryByText("trusted@example.com")).toBeNull();
    });
    expect(screen.getByText("No trusted senders yet. Trust a sender from the privacy banner in a message.")).toBeTruthy();
  });
});
