import { createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import MessageDetail from "../../src/components/MessageDetail";
import type { Message } from "../../src/lib/api";
import { trustSender } from "../../src/lib/api";

const privacyMocks = vi.hoisted(() => ({
  calls: [] as Array<{ messageId: string; privacyMode: unknown }>,
}));
const trustSenderMock = vi.mocked(trustSender);

const mockMessage: Message = {
  id: "message-1",
  account_id: "account-1",
  remote_id: "remote-1",
  message_id_header: null,
  in_reply_to: null,
  references_header: null,
  thread_id: null,
  subject: "Context actions",
  snippet: "Selected text action test",
  from_address: "sender@example.com",
  from_name: "Sender",
  to_list: [{ name: "Destination", address: "destination@example.com" }],
  cc_list: [{ name: "Copied teammate", address: "cc@example.com" }],
  bcc_list: [],
  has_attachments: false,
  is_read: true,
  is_starred: false,
  is_draft: false,
  date: 1_700_000_000,
  remote_version: null,
  is_deleted: false,
  deleted_at: null,
  created_at: 1_700_000_000,
  updated_at: 1_700_000_000,
  body_text: "selected email text inside the message body",
  body_html_raw: "",
};

const bulkMessage: Message = {
  ...mockMessage,
  id: "message-bulk",
  to_list: Array.from({ length: 10 }, (_, index) => ({ name: `Bulk ${index}`, address: `bulk${index}@example.com` })),
  cc_list: [],
};

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (_key: string, options?: unknown) => {
      if (typeof options === "string") return options;
      const opts = (options ?? {}) as Record<string, string | number>;
      const template = typeof opts.defaultValue === "string" ? opts.defaultValue : _key;
      return template.replace(/\{\{(\w+)\}\}/g, (_match, name) => String(opts[name] ?? ""));
    },
  }),
}));

vi.mock("../../src/lib/api", () => ({
  trustSender: vi.fn(),
}));

vi.mock("../../src/hooks/useMessageLoader", () => ({
  useMessageLoader: (messageId: string, privacyMode: unknown) => {
    privacyMocks.calls.push({ messageId, privacyMode });
    return {
    message: {
      ...(messageId === "message-bulk" ? bulkMessage : mockMessage),
      id: messageId,
      from_address: messageId === "message-1" ? "sender@example.com" : "second@example.com",
    },
    setMessage: vi.fn(),
    rendered: { html: "<p>Body</p>", images_blocked: 1, trackers_blocked: [] },
    loading: false,
    };
  },
}));

vi.mock("../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({
    data: [{ id: "account-1", email: "current@example.com" }],
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

vi.mock("../../src/components/MessageActionToolbar", () => ({
  default: () => <div>message actions</div>,
}));

vi.mock("../../src/components/AttachmentList", () => ({
  default: () => <div>attachments</div>,
}));

vi.mock("../../src/components/PrivacyBanner", () => ({
  default: ({ onTrustSender }: { onTrustSender: (trustType: "all") => void }) => (
    <button onClick={() => onTrustSender("all")}>trust sender</button>
  ),
}));

vi.mock("../../src/features/inbox/SnoozePopover", () => ({
  default: () => <div>snooze</div>,
}));

vi.mock("../../src/features/translate/TranslatePopover", () => ({
  default: () => <div>translate popover</div>,
}));

vi.mock("../../src/components/ShadowDomEmail", () => ({
  ShadowDomEmail: ({ html }: { html: string }) => <div>{html}</div>,
}));

vi.mock("../../src/components/ContactAddressAction", () => ({
  default: ({ address }: { address: string }) => (
    <span data-testid="contact-address-action">{address}</span>
  ),
}));

function setSelectedText(text: string) {
  Object.defineProperty(window, "getSelection", {
    configurable: true,
    value: () => ({
      toString: () => text,
      rangeCount: text ? 1 : 0,
      getRangeAt: () => ({
        getBoundingClientRect: () => ({
          left: 80,
          bottom: 120,
          width: 40,
          height: 14,
        }),
      }),
    }),
  });
}

describe("MessageDetail selected-text context actions", () => {
  beforeEach(() => {
    setSelectedText("");
    privacyMocks.calls = [];
    trustSenderMock.mockReset();
    trustSenderMock.mockResolvedValue(undefined);
  });

  it("opens selected-text actions from right click, not from selection alone", () => {
    render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);
    setSelectedText("selected email text");
    const body = screen.getByRole("region", { name: "Message body" });

    fireEvent.mouseUp(body, { clientX: 100, clientY: 120 });

    expect(screen.queryByRole("toolbar", { name: "Selected text actions" })).toBeNull();

    const contextMenu = createEvent.contextMenu(body, { clientX: 100, clientY: 120 });
    const preventDefault = vi.spyOn(contextMenu, "preventDefault");
    fireEvent(body, contextMenu);

    expect(preventDefault).toHaveBeenCalled();
    expect(screen.getByRole("toolbar", { name: "Selected text actions" })).toBeTruthy();
  });

  it("does not suppress the keyboard focus outline on the message body", () => {
    render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);

    const body = screen.getByRole("region", { name: "Message body" });

    expect(body.getAttribute("style")).not.toContain("outline: none");
  });

  it("uses the shared smooth scroll region for the message body", () => {
    render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);

    const body = screen.getByRole("region", { name: "Message body" });

    expect(body.className).toContain("scroll-region");
    expect(body.className).toContain("message-body-scroll");
  });

  it("shows message recipients instead of the account email in the header", () => {
    render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);

    expect(screen.getAllByText(/destination@example\.com/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/cc@example\.com/).length).toBeGreaterThan(0);
    expect(screen.queryByText(/current@example\.com/)).toBeNull();
  });

  it("renders the sender line as a semibold header card with a right-aligned date", () => {
    render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);

    const heading = screen.getByRole("heading", { name: "Context actions" });

    expect(heading.style.fontSize).toBe("16px");

    const senderName = screen.getByText("Sender");

    expect(senderName.style.fontWeight).toBe("600");

    const senderRow = senderName.parentElement as HTMLElement;
    const card = senderRow.parentElement as HTMLElement;

    expect(senderRow.style.fontSize).toBe("13px");
    expect(card.style.borderRadius).toBe("8px");
    expect(card.style.padding).toBe("8px 12px");

    const dateSpan = Array.from(senderRow.querySelectorAll("span"))
      .find((node) => node.style.marginLeft === "auto");

    expect(dateSpan?.textContent).toContain("2023");
  });

  it("offers contact actions for the sender and each visible recipient", () => {
    render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);

    expect(screen.getAllByTestId("contact-address-action").map((node) => node.textContent)).toEqual([
      "sender@example.com",
      "destination@example.com",
      "cc@example.com",
    ]);
  });

  it("collapses bulk recipient lists behind a more toggle", () => {
    render(<MessageDetail messageId="message-bulk" onBack={vi.fn()} />);

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(4);
    expect(screen.getByRole("button", { name: "+7 more" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "+7 more" }));

    expect(screen.getAllByTestId("contact-address-action")).toHaveLength(11);
    expect(screen.getByRole("button", { name: "Show less" })).toBeTruthy();
  });

  it("does not carry a sender trust override to the next message", async () => {
    const { rerender } = render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "trust sender" }));

    await waitFor(() => {
      expect(privacyMocks.calls.at(-1)).toEqual({
        messageId: "message-1",
        privacyMode: { TrustSender: "sender@example.com" },
      });
    });

    rerender(<MessageDetail messageId="message-2" onBack={vi.fn()} />);

    expect(privacyMocks.calls.at(-1)).toEqual({
      messageId: "message-2",
      privacyMode: "LoadOnce",
    });

    rerender(<MessageDetail messageId="message-1" onBack={vi.fn()} />);

    expect(privacyMocks.calls.at(-1)).toEqual({
      messageId: "message-1",
      privacyMode: "LoadOnce",
    });
  });

  it("does not relax privacy when persisting sender trust fails", async () => {
    trustSenderMock.mockRejectedValueOnce(new Error("write failed"));
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    render(<MessageDetail messageId="message-1" onBack={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "trust sender" }));
    await waitFor(() => expect(trustSenderMock).toHaveBeenCalledOnce());

    expect(privacyMocks.calls.at(-1)).toEqual({
      messageId: "message-1",
      privacyMode: "LoadOnce",
    });
    consoleError.mockRestore();
  });
});
