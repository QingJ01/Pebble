import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Message, PrivacyMode, RenderedHtml } from "../../src/lib/api";
import { clearMessageCache, revalidateMessage, useMessageLoader } from "../../src/hooks/useMessageLoader";
import { getMessageWithHtml, getRenderedHtml } from "../../src/lib/api";

vi.mock("../../src/lib/api", () => ({
  getMessageWithHtml: vi.fn(),
  getRenderedHtml: vi.fn(),
}));

vi.mock("../../src/hooks/mutations/useUpdateFlagsMutation", () => ({
  useUpdateFlagsMutation: () => ({ mutate: vi.fn() }),
}));

function makeMessage(overrides: Partial<Message> = {}): Message {
  return {
    id: "message-1",
    account_id: "account-1",
    remote_id: "remote-1",
    message_id_header: null,
    in_reply_to: null,
    references_header: null,
    thread_id: "thread-1",
    subject: "Subject",
    snippet: "Snippet",
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
    body_text: "Body",
    body_html_raw: "<p>Body</p>",
    ...overrides,
  };
}

function makeRendered(html = "<p>Body</p>"): RenderedHtml {
  return {
    html,
    trackers_blocked: [],
    images_blocked: 0,
  };
}

describe("useMessageLoader", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    clearMessageCache();
  });

  it("does not request a second HTML render after the initial message load", async () => {
    vi.mocked(getMessageWithHtml).mockResolvedValue([makeMessage(), makeRendered()]);

    const { result } = renderHook(() => useMessageLoader("message-1", "Strict"));

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(getMessageWithHtml).toHaveBeenCalledOnce();
    expect(getMessageWithHtml).toHaveBeenCalledWith("message-1", "Strict");
    expect(getRenderedHtml).not.toHaveBeenCalled();
  });

  it("re-renders HTML without reloading the message when privacy mode changes", async () => {
    vi.mocked(getMessageWithHtml).mockResolvedValue([makeMessage(), makeRendered()]);
    vi.mocked(getRenderedHtml).mockResolvedValue(makeRendered("<p>Strict again</p>"));

    const { rerender } = renderHook(
      ({ privacyMode }) => useMessageLoader("message-1", privacyMode),
      { initialProps: { privacyMode: "Strict" as const } },
    );

    await waitFor(() => expect(getMessageWithHtml).toHaveBeenCalledOnce());

    rerender({ privacyMode: "LoadOnce" as const });

    await waitFor(() => expect(getRenderedHtml).toHaveBeenCalledOnce());
    expect(getRenderedHtml).toHaveBeenCalledWith("message-1", "LoadOnce");
    expect(getMessageWithHtml).toHaveBeenCalledOnce();
  });

  it("exposes message load failures instead of collapsing them into a missing message", async () => {
    vi.mocked(getMessageWithHtml).mockRejectedValue(new Error("IPC failed"));

    const { result } = renderHook(() => useMessageLoader("message-1", "Strict"));

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.message).toBeNull();
    expect(result.current.error).toBe("IPC failed");
  });

  it("exposes privacy re-render failures without clearing the loaded message", async () => {
    vi.mocked(getMessageWithHtml).mockResolvedValue([makeMessage(), makeRendered()]);
    vi.mocked(getRenderedHtml).mockRejectedValue(new Error("render failed"));

    const { result, rerender } = renderHook(
      ({ privacyMode }) => useMessageLoader("message-1", privacyMode),
      { initialProps: { privacyMode: "Strict" as const } },
    );

    await waitFor(() => expect(result.current.message?.id).toBe("message-1"));

    rerender({ privacyMode: "LoadOnce" as const });

    await waitFor(() => expect(result.current.error).toBe("render failed"));
    expect(result.current.message?.id).toBe("message-1");
  });

  it("keeps the previous message (marked stale) while the next one loads", async () => {
    let resolveSecond: (value: [Message, RenderedHtml]) => void = () => {};
    vi.mocked(getMessageWithHtml).mockImplementation((id) => {
      if (id === "message-1") return Promise.resolve([makeMessage(), makeRendered()]);
      return new Promise((resolve) => { resolveSecond = resolve; });
    });

    const { result, rerender } = renderHook(
      ({ id }) => useMessageLoader(id, "Strict"),
      { initialProps: { id: "message-1" } },
    );
    await waitFor(() => expect(result.current.message?.id).toBe("message-1"));

    rerender({ id: "message-2" });
    expect(result.current.loading).toBe(true);
    expect(result.current.message?.id).toBe("message-1");
    expect(result.current.isStale).toBe(true);

    resolveSecond([makeMessage({ id: "message-2", subject: "Second" }), makeRendered("<p>Two</p>")]);
    await waitFor(() => expect(result.current.message?.id).toBe("message-2"));
    expect(result.current.isStale).toBe(false);
    expect(result.current.loading).toBe(false);
  });

  it("shows a cached message instantly when revisiting it and revalidates in the background", async () => {
    vi.mocked(getMessageWithHtml).mockImplementation((id) =>
      Promise.resolve([makeMessage({ id }), makeRendered(`<p>${id}</p>`)]),
    );
    const { result, rerender } = renderHook(
      ({ id }) => useMessageLoader(id, "Strict"),
      { initialProps: { id: "message-1" } },
    );
    await waitFor(() => expect(result.current.message?.id).toBe("message-1"));
    rerender({ id: "message-2" });
    await waitFor(() => expect(result.current.message?.id).toBe("message-2"));

    rerender({ id: "message-1" });
    expect(result.current.loading).toBe(false);
    expect(result.current.message?.id).toBe("message-1");
    expect(result.current.rendered?.html).toContain("message-1");
    await waitFor(() => expect(getMessageWithHtml).toHaveBeenCalledTimes(3));
  });
  it("does not show cached HTML that was rendered under a looser privacy mode", async () => {
    vi.mocked(getRenderedHtml).mockResolvedValue(makeRendered("<p>re-rendered</p>"));
    let release: (() => void) | null = null;
    vi.mocked(getMessageWithHtml).mockImplementation((id, mode) => {
      const value: [Message, RenderedHtml] = [makeMessage({ id }), makeRendered(`<p>${id} ${String(mode)}</p>`)];
      if (mode === "Strict" && id === "message-1") {
        return new Promise((resolve) => { release = () => resolve(value); });
      }
      return Promise.resolve(value);
    });
    const { result, rerender } = renderHook(
      ({ id, mode }) => useMessageLoader(id, mode),
      { initialProps: { id: "message-1", mode: "Off" as PrivacyMode } },
    );
    await waitFor(() => expect(result.current.rendered?.html).toContain("message-1 Off"));
    rerender({ id: "message-2", mode: "Off" });
    await waitFor(() => expect(result.current.message?.id).toBe("message-2"));

    // The user tightens privacy, then reopens message-1 (cached under "Off").
    rerender({ id: "message-2", mode: "Strict" });
    rerender({ id: "message-1", mode: "Strict" });
    expect(result.current.rendered).toBeNull();
    expect(result.current.loading).toBe(true);

    release!();
    await waitFor(() => expect(result.current.rendered?.html).toContain("message-1 Strict"));
  });
  it("shows not-found instead of the previous message when the next one no longer exists", async () => {
    vi.mocked(getMessageWithHtml).mockImplementation((id) =>
      Promise.resolve(id === "gone" ? null : [makeMessage({ id }), makeRendered()]),
    );
    const { result, rerender } = renderHook(
      ({ id }) => useMessageLoader(id, "Strict"),
      { initialProps: { id: "message-1" } },
    );
    await waitFor(() => expect(result.current.message?.id).toBe("message-1"));

    rerender({ id: "gone" });
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.message).toBeNull();
    expect(result.current.isStale).toBe(false);
  });
  it("switches an open message to not-found when a revalidation finds it gone", async () => {
    vi.mocked(getMessageWithHtml).mockResolvedValueOnce([makeMessage(), makeRendered()]);
    const { result } = renderHook(() => useMessageLoader("message-1", "Strict"));
    await waitFor(() => expect(result.current.message?.id).toBe("message-1"));

    vi.mocked(getMessageWithHtml).mockResolvedValueOnce(null);
    act(() => revalidateMessage("message-1"));
    await waitFor(() => expect(result.current.message).toBeNull());
  });
});
