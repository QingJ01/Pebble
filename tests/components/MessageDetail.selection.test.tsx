import { createEvent, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import MessageDetail from "../../src/components/MessageDetail";
import type { Message } from "../../src/lib/api";

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
  to_list: [],
  cc_list: [],
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

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

vi.mock("../../src/lib/api", () => ({
  trustSender: vi.fn(),
}));

vi.mock("../../src/hooks/useMessageLoader", () => ({
  useMessageLoader: () => ({
    message: mockMessage,
    setMessage: vi.fn(),
    rendered: null,
    loading: false,
  }),
}));

vi.mock("../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({
    data: [{ id: "account-1", email: "recipient@example.com" }],
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
  default: () => <div>privacy banner</div>,
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
});
