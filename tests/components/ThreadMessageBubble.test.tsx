import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import ThreadMessageBubble from "../../src/components/ThreadMessageBubble";
import type { Message } from "../../src/lib/api";
import { getRenderedHtml } from "../../src/lib/api";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: unknown) => {
      if (typeof options === "string") return options;
      const opts = (options ?? {}) as Record<string, string | number>;
      const template = typeof opts.defaultValue === "string" ? opts.defaultValue : key;
      return template.replace(/\{\{(\w+)\}\}/g, (_match, name) => String(opts[name] ?? ""));
    },
  }),
}));

vi.mock("../../src/lib/api", () => ({
  getRenderedHtml: vi.fn().mockResolvedValue({
    html: "<p>Rendered</p>",
    trackers_blocked: [],
    images_blocked: 0,
  }),
}));

vi.mock("../../src/components/ShadowDomEmail", () => ({
  ShadowDomEmail: ({ html }: { html: string }) => <div>{html}</div>,
}));

vi.mock("../../src/components/ContactAddressAction", () => ({
  default: ({ address }: { address: string }) => (
    <span data-testid="contact-address-action">{address}</span>
  ),
}));

const message: Message = {
  id: "message-1",
  account_id: "account-1",
  remote_id: "remote-1",
  message_id_header: null,
  in_reply_to: null,
  references_header: null,
  thread_id: "thread-1",
  subject: "Thread message",
  snippet: "Snippet",
  from_address: "sender@example.com",
  from_name: "Sender",
  to_list: [{ name: null, address: "user@example.com" }],
  cc_list: [{ name: null, address: "cc@example.com" }],
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
  body_text: "Body",
  body_html_raw: "<p>Body</p>",
};

describe("ThreadMessageBubble", () => {
  it("shows copied recipients when expanded", () => {
    render(<ThreadMessageBubble message={message} defaultExpanded />);

    expect(screen.getByText("Cc:")).toBeTruthy();
    expect(screen.getAllByText("cc@example.com").length).toBeGreaterThan(0);
  });

  it("uses relaxed privacy mode by default when rendering expanded thread messages", async () => {
    localStorage.removeItem("pebble-privacy-mode");

    render(<ThreadMessageBubble message={message} defaultExpanded />);

    await waitFor(() => {
      expect(getRenderedHtml).toHaveBeenCalledWith("message-1", "LoadOnce");
    });
  });

  it("offers contact actions for expanded message participants", () => {
    render(<ThreadMessageBubble message={message} defaultExpanded />);

    expect(document.querySelectorAll("[data-testid='contact-address-action']")).toHaveLength(3);
  });

  it("collapses bulk recipient lists when expanded", () => {
    const bulk: Message = {
      ...message,
      to_list: Array.from({ length: 10 }, (_, index) => ({ name: null, address: `bulk${index}@example.com` })),
      cc_list: [],
    };
    render(<ThreadMessageBubble message={bulk} defaultExpanded />);

    expect(document.querySelectorAll("[data-testid='contact-address-action']")).toHaveLength(4);

    fireEvent.click(screen.getByRole("button", { name: "+7 more" }));

    expect(document.querySelectorAll("[data-testid='contact-address-action']")).toHaveLength(11);
  });
});
