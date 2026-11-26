import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MessageDetail from "../../src/components/MessageDetail";
import type { Message } from "../../src/lib/api";

const loaderState = vi.hoisted(() => ({
  current: {
    message: null as unknown,
    loading: true,
    isStale: false,
  },
}));

const baseMessage = {
  id: "m1",
  account_id: "account-1",
  subject: "First subject",
  from_address: "a@example.com",
  from_name: "A",
  to_list: [],
  cc_list: [],
  bcc_list: [],
  has_attachments: false,
  is_read: true,
  is_starred: false,
  is_draft: false,
  date: 1_700_000_000,
  body_text: "first body",
} as unknown as Message;

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, options?: unknown) => (typeof options === "string" ? options : key),
  }),
}));
vi.mock("../../src/lib/api", () => ({ trustSender: vi.fn() }));
vi.mock("../../src/hooks/useMessageLoader", () => ({
  useMessageLoader: () => ({
    ...loaderState.current,
    setMessage: vi.fn(),
    rendered: null,
    error: null,
    errorInfo: null,
  }),
}));
vi.mock("../../src/hooks/useBilingualTranslation", () => ({
  useBilingualTranslation: () => ({
    bilingualMode: false,
    bilingualResult: null,
    bilingualLoading: false,
    handleBilingualToggle: vi.fn(),
    resetBilingual: vi.fn(),
  }),
}));
vi.mock("../../src/components/MessageActionToolbar", () => ({ default: () => <div>actions</div> }));
vi.mock("../../src/components/MoveToButton", () => ({ default: () => null }));
vi.mock("../../src/components/AttachmentList", () => ({ default: () => null }));
vi.mock("../../src/components/ContactAddressAction", () => ({ default: () => null }));
vi.mock("../../src/components/MessageParticipants", () => ({ default: () => null }));
vi.mock("../../src/components/ShadowDomEmail", () => ({ ShadowDomEmail: () => null }));

const skeleton = () => document.querySelector('[role="status"][aria-live="polite"]');

describe("MessageDetail loading (D3)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    loaderState.current = { message: null, loading: true, isStale: false };
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows no skeleton for loads faster than 200ms", () => {
    const view = render(<MessageDetail messageId="m1" onBack={vi.fn()} />);
    expect(skeleton()).toBeNull();
    act(() => { vi.advanceTimersByTime(150); });
    loaderState.current = { message: baseMessage, loading: false, isStale: false };
    view.rerender(<MessageDetail messageId="m1" onBack={vi.fn()} />);
    act(() => { vi.advanceTimersByTime(500); });
    expect(skeleton()).toBeNull();
    expect(screen.getByRole("heading", { name: "First subject" })).toBeTruthy();
  });

  it("shows the skeleton after 200ms and keeps it for at least 300ms", () => {
    const view = render(<MessageDetail messageId="m1" onBack={vi.fn()} />);
    act(() => { vi.advanceTimersByTime(210); });
    expect(skeleton()).not.toBeNull();

    // Load completes 50ms after the skeleton appeared: it must stay ≥300ms.
    act(() => { vi.advanceTimersByTime(50); });
    loaderState.current = { message: baseMessage, loading: false, isStale: false };
    view.rerender(<MessageDetail messageId="m1" onBack={vi.fn()} />);
    expect(skeleton()).not.toBeNull();
    act(() => { vi.advanceTimersByTime(200); });
    expect(skeleton()).not.toBeNull();
    act(() => { vi.advanceTimersByTime(60); });
    expect(skeleton()).toBeNull();
    expect(screen.getByRole("heading", { name: "First subject" })).toBeTruthy();
  });

  it("keeps the previous message visible but dimmed while the next one loads", () => {
    loaderState.current = { message: baseMessage, loading: false, isStale: false };
    const view = render(<MessageDetail messageId="m1" onBack={vi.fn()} />);

    loaderState.current = { message: baseMessage, loading: true, isStale: true };
    view.rerender(<MessageDetail messageId="m2" onBack={vi.fn()} />);
    const container = document.querySelector(".message-detail-container");
    expect(container?.className).toContain("message-detail--stale");
    expect(container?.getAttribute("aria-busy")).toBe("true");
    expect(screen.getByText("First subject")).toBeTruthy();
    expect(skeleton()).toBeNull();
  });

  it("keeps Back usable while stale but blocks actions on the previous message", () => {
    const onBack = vi.fn();
    loaderState.current = { message: baseMessage, loading: true, isStale: true };
    render(<MessageDetail messageId="m2" onBack={onBack} />);

    const back = screen.getByRole("button", { name: "Back" });
    expect(back.closest("[inert]")).toBeNull();
    fireEvent.click(back);
    expect(onBack).toHaveBeenCalled();
    expect(screen.getByText("actions").closest("[inert]")).not.toBeNull();
    expect(screen.getByText("First subject").closest("[inert]")).not.toBeNull();
  });
  it("moves focus to Back when the toolbar it was on goes stale", () => {
    loaderState.current = { message: baseMessage, loading: false, isStale: false };
    const view = render(<MessageDetail messageId="m1" onBack={vi.fn()} />);
    const snooze = screen.getByRole("button", { name: "Snooze message" });
    snooze.focus();
    expect(document.activeElement).toBe(snooze);

    loaderState.current = { message: baseMessage, loading: true, isStale: true };
    view.rerender(<MessageDetail messageId="m2" onBack={vi.fn()} />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Back" }));
  });
  it("keeps a Back button when the message is not found", () => {
    const onBack = vi.fn();
    loaderState.current = { message: null, loading: false, isStale: false };
    render(<MessageDetail messageId="gone" onBack={onBack} />);
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalled();
  });
});
