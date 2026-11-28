import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import AgentView from "../../../src/features/agent/AgentView";
import { upcomingDeadlines } from "../../../src/features/agent/AgentSide";
import { dueLabel } from "../../../src/features/agent/AgentThread";
import type { AgentEvent } from "../../../src/features/agent/api";
import type { NotificationEntry } from "../../../src/features/notify/api";
import { useToastStore } from "../../../src/stores/toast.store";

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
const t = (_k: string, fallback?: unknown) => String(fallback);

function event(partial: Partial<AgentEvent>): AgentEvent {
  return {
    id: "e", kind: "deadline", dedupeKey: "k", accountId: "a1", messageId: "m1", threadId: null,
    watchId: null, conversationId: null, payload: {}, createdAt: NOW - 600, readAt: null, dismissedAt: null,
    pushState: "in_app", pushedAt: null, resolution: null, ...partial,
  };
}

function digestEntry(partial: Partial<NotificationEntry>): NotificationEntry {
  return {
    accountId: "a1", messageId: "m5", tier: "digest", reason: "model:newsletter", why: "Weekly product news",
    delivered: false, createdAt: NOW, seen: false, subject: "This week at Acme", fromName: "Acme",
    fromAddress: "news@acme.example", threadId: null, date: NOW, feedback: null, ...partial,
  };
}

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><AgentView /></QueryClientProvider>);
}

describe("deadlines in the assistant", () => {
  let events: AgentEvent[];
  beforeEach(() => {
    useToastStore.setState({ toasts: [] });
    events = [
      event({ id: "d1", payload: { from: "Dana", subject: "Budget numbers", due: NOW + 2 * 86_400, allDay: true, phrase: "by Friday", source: "rule" } }),
      event({ id: "d2", messageId: "m2", createdAt: NOW - 300, kind: "deadline_soon",
        payload: { from: "Wei", subject: "Venue deposit", due: NOW + 2 * 3600, allDay: false } }),
      event({ id: "d0", messageId: "m2", createdAt: NOW - 90_000,
        payload: { from: "Wei", subject: "Venue deposit", due: NOW + 2 * 3600, allDay: false } }),
    ];
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command) => {
      switch (command) {
        case "list_agent_events": return Promise.resolve(events);
        case "list_thread_watches":
        case "agent_chat_list":
        case "agent_chat_running":
        case "list_accounts":
          return Promise.resolve([]);
        case "list_digest": return Promise.resolve([digestEntry({})]);
        case "notify_feedback": return Promise.resolve({ sender: "news@acme.example", suggest: "vip" });
        case "get_agent_model": return Promise.resolve({ configured: false, endpoint: "", model: "", hasApiKey: false, local: false });
        case "get_notify_settings": return Promise.resolve(null);
        default: return Promise.resolve(null);
      }
    });
  });

  it("notes a deadline with its phrase and lets the user settle it", async () => {
    wrap();
    const log = await screen.findByRole("log");
    expect(await within(log).findByText(/Dana needs something from you by/)).toBeTruthy();
    expect(within(log).getByText("“by Friday”")).toBeTruthy();
    // Two hours from now may already be tomorrow late in the evening.
    expect(within(log).getByText(/Due (today|tomorrow) .*: Wei is waiting on “Venue deposit”/)).toBeTruthy();

    const notOne = within(log).getAllByRole("button", { name: "Not a deadline" });
    expect(notOne).toHaveLength(2); // the arrival notes, not the reminder
    fireEvent.click(notOne[1]);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("resolve_agent_event", { id: "d1", action: "not_deadline", days: null }));
  });

  it("lists what is coming up in the side column and marks it done", async () => {
    wrap();
    const side = await screen.findByRole("complementary");
    const section = await within(side).findByRole("region", { name: /Coming up/ });
    const titles = within(section).getAllByRole("button").map((b) => b.textContent);
    expect(titles[0]).toBe("Venue deposit"); // soonest first
    fireEvent.click(within(section).getAllByRole("button", { name: "It's done" })[0]);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("resolve_agent_event", { id: "d0", action: "done", days: null }));
  });

  it("shows the AI's reason in the digest and offers a sender rule after corrections", async () => {
    wrap();
    const side = await screen.findByRole("complementary");
    fireEvent.click(await within(side).findByRole("button", { name: "Show" }));
    expect(within(side).getByText(/✦ Weekly product news/)).toBeTruthy();
    fireEvent.click(within(side).getByRole("button", { name: "Should notify" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("notify_feedback", { accountId: "a1", messageId: "m5", important: true }));
    await waitFor(() => expect(useToastStore.getState().toasts[0]?.action?.label).toBe("Make VIP"));
    useToastStore.getState().toasts[0].action!.onClick();
    expect(mockInvoke).toHaveBeenCalledWith("set_sender_pref", { accountId: "a1", pattern: "news@acme.example", kind: "vip" });
  });
});

describe("deadline helpers", () => {
  it("keeps open deadlines of the coming week, soonest first", () => {
    const list = upcomingDeadlines([
      event({ id: "late", payload: { due: NOW - 20 * 3600 } }),
      event({ id: "far", payload: { due: NOW + 10 * 86_400 } }),
      event({ id: "b", payload: { due: NOW + 3600 * 5 } }),
      event({ id: "a", payload: { due: NOW + 3600 } }),
      event({ id: "done", payload: { due: NOW + 3600 }, resolution: { action: "done" } }),
      event({ id: "soon", kind: "deadline_soon", payload: { due: NOW + 3600 } }),
    ], NOW);
    expect(list.map((e) => e.id)).toEqual(["a", "b"]);
  });

  it("labels due times relative to today", () => {
    const noon = new Date();
    noon.setHours(12, 0, 0, 0);
    const today = Math.floor(noon.getTime() / 1000);
    expect(dueLabel(t as never, today, true, "en", today)).toBe("today");
    expect(dueLabel(t as never, today + 86_400, true, "en", today)).toBe("tomorrow");
    expect(dueLabel(t as never, today, false, "en", today)).toMatch(/^today \d/);
  });
});
