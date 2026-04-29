import { describe, expect, it } from "vitest";
import { buildComposeEditorContent, shouldApplyInitialEditorContent } from "../../src/hooks/useComposeEditor";
import type { Message } from "../../src/lib/ipc-types";
import type { TFunction } from "i18next";

const t = ((key: string, options?: Record<string, string>) => {
  if (key === "compose.quoteAttribution") {
    return `On ${options?.date}, ${options?.sender} wrote:`;
  }
  if (key === "compose.forwardedHeader") {
    return "Forwarded message";
  }
  if (key === "compose.forwardedFrom") {
    return `From: ${options?.sender}`;
  }
  if (key === "compose.forwardedSubject") {
    return `Subject: ${options?.subject}`;
  }
  return key;
}) as unknown as TFunction;

function makeMessage(overrides: Partial<Message>): Message {
  return {
    id: "message-1",
    account_id: "account-1",
    remote_id: "remote-1",
    message_id_header: "<message-1@example.com>",
    in_reply_to: null,
    references_header: null,
    thread_id: null,
    subject: "HTML message",
    snippet: "",
    from_address: "sender@example.com",
    from_name: "Sender",
    to_list: [],
    cc_list: [],
    bcc_list: [],
    has_attachments: false,
    is_read: true,
    is_starred: false,
    is_draft: false,
    date: 0,
    remote_version: null,
    is_deleted: false,
    deleted_at: null,
    created_at: 0,
    updated_at: 0,
    body_text: "",
    body_html_raw: "",
    ...overrides,
  };
}

describe("buildComposeEditorContent", () => {
  it("renders entity-encoded HTML bodies as quoted HTML when replying", () => {
    const content = buildComposeEditorContent({
      composeMode: "reply",
      composeReplyTo: makeMessage({
        body_html_raw:
          "&lt;html&gt;&lt;body&gt;&lt;p&gt;&lt;strong&gt;Hello&lt;/strong&gt; team&lt;/p&gt;&lt;/body&gt;&lt;/html&gt;",
      }),
      isReply: true,
      signatureHtml: "",
      t,
    });

    expect(content).toContain("<blockquote>");
    expect(content).toContain("<strong>Hello</strong> team");
    expect(content).not.toContain("&lt;strong&gt;");
  });

  it("does not re-apply generated content after the editor was initialized", () => {
    expect(shouldApplyInitialEditorContent({
      editorExists: true,
      initialized: true,
      signatureReady: true,
      hasRestoredDraft: false,
    })).toBe(false);
  });

  it("waits for async signature loading before initializing new compose content", () => {
    expect(shouldApplyInitialEditorContent({
      editorExists: true,
      initialized: false,
      signatureReady: false,
      hasRestoredDraft: false,
    })).toBe(false);
    expect(shouldApplyInitialEditorContent({
      editorExists: true,
      initialized: false,
      signatureReady: true,
      hasRestoredDraft: false,
    })).toBe(true);
  });
});
