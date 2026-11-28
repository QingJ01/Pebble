import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import FollowUpButton from "../../../src/features/agent/FollowUpButton";
import AgentView from "../../../src/features/agent/AgentView";
import { deadlineInDays, type AgentEvent } from "../../../src/features/agent/api";
import { buildBlocks } from "../../../src/features/agent/AgentThread";
import { useUIStore } from "../../../src/stores/ui.store";
import { useComposeStore } from "../../../src/stores/compose.store";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    i18n: { language: "en" },
    t: (_key: string, fallback?: string | Record<string, unknown>, vars?: Record<string, unknown>) => {
      const options = typeof fallback === "object" ? fallback : vars;
      const text = typeof fallback === "string" ? fallback : String(options?.defaultValue ?? _key);
      return text.replace(/\{\{(\w+)\}\}/g, (_, name) => String(options?.[name] ?? ""));
    },
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const mockInvoke = vi.mocked(invoke);
const NOW = Math.floor(Date.now() / 1000);

function event(partial: Partial<AgentEvent>): AgentEvent {
  return {
    id: "e", kind: "awaiting_reply", dedupeKey: "k", accountId: "a1", messageId: "m1", threadId: null,
    watchId: null, conversationId: null, payload: {}, createdAt: NOW, readAt: null, dismissedAt: null,
    pushState: "in_app", pushedAt: null, resolution: null,
    ...partial,
  };
}

function wrap(ui: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

describe("AgentView", () => {
  let events: AgentEvent[];
  let model: { configured: boolean; endpoint: string; model: string; hasApiKey: boolean; local: boolean };
  beforeEach(() => {
    model = { configured: false, endpoint: "", model: "", hasApiKey: false, local: false };
    events = [
      event({ id: "b", kind: "briefing", messageId: null, createdAt: NOW - 4000, readAt: NOW, payload: {
        importantUnread: 2, awaitingReply: [{ messageId: "m1", accountId: "a1", from: "Ann", subject: "Dinner?" }],
        important: [{ messageId: "m9", accountId: "a1", from: "Boss", fromAddress: "boss@corp.example", subject: "Board deck", receivedAt: NOW - 600 }],
        followUpCount: 1, overdueCount: 1, digestCount: 4,
      } }),
      event({ id: "o-old", kind: "watch_overdue", watchId: "w0", messageId: null, createdAt: NOW - 3000, readAt: NOW,
        payload: { title: "Lease" }, resolution: { action: "wait", days: 2, until: NOW + 86_400 } }),
      event({ id: "r", kind: "watch_reply", createdAt: NOW - 2000, payload: { from: "Mr. Patel", title: "Lease renewal" } }),
      event({ id: "w1", kind: "awaiting_reply", messageId: "m1", createdAt: NOW - 300,
        payload: { from: "Ann", subject: "Dinner?", snippet: "Are you free at 7?", receivedAt: NOW - 30 * 3600 } }),
      event({ id: "w2", kind: "awaiting_reply", messageId: "m2", createdAt: NOW - 290, resolution: { action: "answered" },
        payload: { from: "Bo", subject: "Lunch?", receivedAt: NOW - 52 * 3600 } }),
      event({ id: "o", kind: "watch_overdue", watchId: "w1", messageId: "sent-1", createdAt: NOW - 100,
        payload: { title: "Proposal", subject: "Proposal", participants: ["dana@partner.example"], sentAt: NOW - 4 * 86400 } }),
    ];
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command) => {
      switch (command) {
        case "list_agent_events":
          return Promise.resolve(events);
        case "list_thread_watches":
          return Promise.resolve([
            { id: "w1", accountId: "a1", threadId: "t", subjectKey: "proposal", participants: [], anchorMessageId: "send:x",
              title: "Proposal", status: "active", source: "ui", conversationId: null, createdAt: NOW - 86400,
              deadline: NOW - 10, nudgedAt: NOW, matchedMessageId: null, matchedAt: null, updatedAt: NOW },
          ]);
        case "get_notify_settings":
          return Promise.resolve({ defaults: {
            mode: "smart", quietHours: null, modelAssist: false, modelReviewsBulk: false, modelSendBody: true,
            briefingEnabled: true, briefingMinute: 540, awaitingReplyEnabled: true, awaitingReplyHours: 24, followUpDays: 3,
          }, accounts: {} });
        case "list_digest":
          return Promise.resolve([]);
        case "get_message":
          return Promise.resolve({ id: "m1", account_id: "a1", subject: "Dinner?" });
        case "get_agent_model":
          return Promise.resolve(model);
        case "agent_chat_list":
        case "agent_chat_running":
        case "list_accounts":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    });
  });

  it("links every item to its mail and opens replies", async () => {
    const openMessageInInbox = vi.fn();
    useUIStore.setState({ openMessageInInbox });
    const openCompose = vi.fn();
    useComposeStore.setState({ openCompose });
    wrap(<AgentView />);
    const log = await screen.findByRole("log");
    await within(log).findByText("Daily briefing");

    fireEvent.click(within(log).getByRole("button", { name: /Boss.*Board deck/ }));
    expect(openMessageInInbox).toHaveBeenCalledWith("m9");

    fireEvent.click(within(log).getByRole("button", { name: /To dana@partner.example/ }));
    expect(openMessageInInbox).toHaveBeenCalledWith("sent-1");

    fireEvent.click(within(log).getAllByRole("button", { name: "Reply" })[0]);
    await waitFor(() => expect(openCompose).toHaveBeenCalledWith("reply", expect.objectContaining({ id: "m1" })));

    fireEvent.click(within(log).getByRole("button", { name: "Write a follow-up" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("get_message", { messageId: "sent-1" }));
  });

  it("tells the story oldest first, merging reminders from one scan", async () => {
    wrap(<AgentView />);
    const log = await screen.findByRole("log");
    await within(log).findByText("Daily briefing");
    const text = log.textContent ?? "";
    const order = [
      "Hi, I'm Pebble",
      "Daily briefing",
      "Still no reply to “Lease”",
      "Wait 2 more days",
      "OK, I'll check for a reply again",
      "Mr. Patel replied",
      "2 messages seem to be waiting for your reply",
      "Still no reply to “Proposal”",
    ].map((part) => text.indexOf(part));
    expect(order.every((index) => index >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(within(log).getByText("Are you free at 7?")).toBeTruthy();
    expect(within(log).getByText("You replied")).toBeTruthy();
    expect(within(log).getByText("1 follow-ups got no reply by their deadline.")).toBeTruthy();
    expect(within(log).getByRole("separator").textContent).toBe("New");
  });

  it("answers with quick replies", async () => {
    wrap(<AgentView />);
    const log = await screen.findByRole("log");
    await within(log).findByText("Daily briefing");
    fireEvent.click(within(log).getByRole("button", { name: "Remind me tomorrow" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("resolve_agent_event", { id: "w1", action: "later", days: 1 }));
    fireEvent.click(within(log).getByRole("button", { name: "Wait another week" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("resolve_agent_event", { id: "o", action: "wait", days: 7 }));
    fireEvent.click(within(log).getByRole("button", { name: "Stop following" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("resolve_agent_event", { id: "o", action: "stop", days: null }));
  });

  it("asks to set up a model instead of showing a composer", async () => {
    const setActiveView = vi.fn();
    const setSettingsTab = vi.fn();
    useUIStore.setState({ setActiveView, setSettingsTab });
    wrap(<AgentView />);
    fireEvent.click(await screen.findByRole("button", { name: "Set up a model" }));
    expect(setSettingsTab).toHaveBeenCalledWith("assistant");
    expect(setActiveView).toHaveBeenCalledWith("settings");
    expect(screen.queryByRole("textbox", { name: "Ask Pebble about your mail…" })).toBeNull();
  });

  it("shows the composer once a model is set up", async () => {
    model = { configured: true, endpoint: "http://localhost:11434", model: "qwen2.5", hasApiKey: false, local: true };
    wrap(<AgentView />);
    expect(await screen.findByRole("textbox", { name: "Ask Pebble about your mail…" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Set up a model" })).toBeNull();
  });

  it("lists follow-ups and routines in the side column", async () => {
    wrap(<AgentView />);
    const side = await screen.findByRole("complementary", { name: "What Pebble is keeping track of" });
    await waitFor(() => expect(side.textContent).toContain("Proposal"));
    expect(side.textContent).toContain("No reply yet");
    expect(side.textContent).toMatch(/Next: (today|tomorrow) 09:00/);
    expect(side.textContent).toContain("After 1 days");
    fireEvent.click(within(side).getByRole("button", { name: "Stop following" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_thread_watch", expect.objectContaining({ id: "w1", status: "cancelled" })),
    );
  });

  it("marks the conversation read after it has been seen", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    wrap(<AgentView />);
    await within(await screen.findByRole("log")).findByText("Daily briefing");
    vi.advanceTimersByTime(3000);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("mark_agent_events_read"));
    vi.useRealTimers();
  });
});

describe("buildBlocks", () => {
  it("merges reminders raised within ten minutes and keeps the rest apart", () => {
    const blocks = buildBlocks([
      event({ id: "a", kind: "awaiting_reply", createdAt: 1_000 }),
      event({ id: "o", kind: "watch_overdue", createdAt: 1_100 }),
      event({ id: "b", kind: "awaiting_reply", createdAt: 1_200 }),
      event({ id: "c", kind: "awaiting_reply", createdAt: 1_500 }),
      event({ id: "d", kind: "awaiting_reply", createdAt: 3_000 }),
      event({ id: "again", kind: "awaiting_reply", createdAt: 3_100, payload: { again: true } }),
    ]);
    const shape = blocks.map((block) => (block.kind === "awaiting" ? block.events.map((e) => e.id).join("+") : block.event.id));
    expect(shape).toEqual(["a", "o", "b+c", "d", "again"]);
  });
});

describe("FollowUpButton", () => {
  it("follows a thread with a deadline and can stop", async () => {
    let watch: unknown = null;
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command, args) => {
      if (command === "get_thread_watch_for") return Promise.resolve(watch);
      if (command === "create_thread_watch") {
        watch = { id: "w9", status: "active", deadline: (args as { deadline: number }).deadline };
        return Promise.resolve(watch);
      }
      return Promise.resolve(null);
    });
    wrap(<FollowUpButton accountId="a1" messageId="m1" threadId="t1" />);
    fireEvent.click(await screen.findByRole("button", { name: "Follow up" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: /in 3 days/ }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("create_thread_watch", expect.objectContaining({ accountId: "a1", messageId: "m1" })),
    );
    const call = mockInvoke.mock.calls.find(([command]) => command === "create_thread_watch")!;
    const deadline = (call[1] as { deadline: number }).deadline;
    expect(Math.abs(deadline - deadlineInDays(3))).toBeLessThan(5);

    fireEvent.click(await screen.findByRole("button", { name: /Following until/ }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Stop following" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("update_thread_watch", expect.objectContaining({ id: "w9", status: "cancelled" })),
    );
  });
});
