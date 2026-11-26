import { describe, expect, it } from "vitest";
import {
  appendReplyQuoteHtml,
  buildComposeEditorContent,
  buildReplyQuoteHtml,
  initialRawSource,
  shouldApplyInitialEditorContent,
} from "../../src/hooks/useComposeEditor";
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
  it("keeps quoted reply content out of the editable reply body", () => {
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

    expect(content).toContain("<p><br></p>");
    expect(content).not.toContain("<blockquote");
    expect(content).not.toContain("<strong>Hello</strong> team");
    expect(content).not.toContain("&lt;strong&gt;");
  });

  it("builds the original reply as separate quoted HTML", () => {
    const quote = buildReplyQuoteHtml({
      composeReplyTo: makeMessage({
        body_html_raw:
          "&lt;html&gt;&lt;body&gt;&lt;p&gt;&lt;strong&gt;Hello&lt;/strong&gt; team&lt;/p&gt;&lt;/body&gt;&lt;/html&gt;",
      }),
      t,
    });

    expect(quote).toContain("<blockquote");
    expect(quote).toContain("Sender wrote");
    expect(quote).toContain("<strong>Hello</strong> team");
    expect(quote).not.toContain("&lt;strong&gt;");
  });

  it("removes document CSS and remote-loading resources from quoted messages", () => {
    const quote = buildReplyQuoteHtml({
      composeReplyTo: makeMessage({
        body_html_raw:
          '<style>#root { display: none }</style>' +
          '<link rel="stylesheet" href="https://tracker.example/mail.css">' +
          '<p style="color: red">Hello</p>' +
          '<img src="https://tracker.example/pixel" srcset="https://tracker.example/2x 2x">' +
          '<img src=" \\\\tracker.example\\pixel">' +
          '<img src="ht\nttps://tracker.example/obfuscated-pixel">' +
          '<img src="data:image/png;base64,cG5nLWZpeHR1cmU=">',
      }),
      t,
    });

    expect(quote).toContain("Hello");
    expect(quote).toContain("color: red");
    expect(quote).not.toContain("<style");
    expect(quote).not.toContain("<link");
    expect(quote).not.toContain("tracker.example");
    expect(quote).not.toContain("srcset");
    expect(quote).toContain('src="data:image/png;base64,cG5nLWZpeHR1cmU="');
  });

  it("appends the quoted reply only when sending", () => {
    expect(appendReplyQuoteHtml("<p>Reply</p>", "<blockquote><p>Original</p></blockquote>"))
      .toBe("<p>Reply</p><br/><br/><blockquote><p>Original</p></blockquote>");
    expect(appendReplyQuoteHtml("<p>Reply</p>", "")).toBe("<p>Reply</p>");
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

describe("compose untrusted content", () => {
  it("sanitizes the assembled reply quote and blocks SVG <style> escapes", () => {
    const quote = buildReplyQuoteHtml({
      composeReplyTo: makeMessage({
        body_html_raw:
          '<svg><style><a>&lt;/style&gt;&lt;div style="position:fixed;inset:0;z-index:99999"&gt;x&lt;/div&gt;</a></style></svg>' +
          '<p class="fixed inset-0" style="position:absolute;color:red">Body</p>',
      }),
      t,
    });
    const template = document.createElement("template");
    template.innerHTML = quote;

    expect(template.content.querySelector("div")).toBeNull();
    expect(quote).not.toMatch(/position|z-index|inset/i);
    expect(template.content.querySelector("blockquote")?.className).toBe("compose-original-quote");
    expect(template.content.querySelector("p.fixed")).toBeNull();
    expect(quote).toContain("Body");
  });

  it("treats a mailto body as plain text for the raw source", () => {
    const raw = initialRawSource(null, '<img src=x onerror="alert(1)">\nsecond line');

    expect(raw).not.toContain("<img");
    expect(raw).toContain("&lt;img");
    expect(raw).toContain("<p>second line</p>");
    expect(initialRawSource(null, undefined)).toBe("");
    expect(initialRawSource({ rawSource: "<b>draft</b>" }, "ignored")).toBe("<b>draft</b>");
  });
});
