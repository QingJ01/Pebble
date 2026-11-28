import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import AgentView from "../../../src/features/agent/AgentView";
import { RichText } from "../../../src/features/agent/AgentThread";
import { useComposeStore } from "../../../src/stores/compose.store";
import { useMailStore } from "../../../src/stores/mail.store";
import type { ChatEntry } from "../../../src/features/agent/chat-api";

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

function entry(partial: Partial<ChatEntry>): ChatEntry {
  return {
    id: "x", role: "assistant", runId: "r1", status: "done", createdAt: NOW, text: "", scope: [], used: [],
    drafts: [], followUps: [], error: null, fallback: false, ...partial,
  };
}

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><AgentView /></QueryClientProvider>);
}

describe("assistant chat", () => {
  let chat: ChatEntry[];
  let running: string[];
  beforeEach(() => {
    useMailStore.setState({ activeAccountId: "work" });
    chat = [
      entry({ id: "q1", role: "user", text: "Reply to Wei: I'm in", createdAt: NOW - 60 }),
      entry({
        id: "a1", createdAt: NOW - 50, text: "Found **Wei's** plan.\n\n- Saturday\n- Ridge trail",
        used: [{ messageId: "m3", accountId: "work", from: "Wei", fromAddress: "wei@x.example", subject: "Hiking", date: NOW }],
        drafts: [{ accountId: "work", replyToMessageId: "m3", to: ["wei@x.example"], cc: [], subject: "Re: Hiking", body: "Count me in!" }],
      }),
      entry({ id: "q2", role: "user", runId: "r2", text: "And the contract?", createdAt: NOW - 40 }),
      entry({ id: "a2", runId: "r2", status: "error", error: "HTTP 401: bad key", createdAt: NOW - 30 }),
    ];
    running = [];
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command, args) => {
      switch (command) {
        case "agent_chat_list": return Promise.resolve(chat);
        case "agent_chat_running": return Promise.resolve(running);
        case "agent_chat_send": return Promise.resolve("r3");
        case "get_agent_model":
          return Promise.resolve({ configured: true, endpoint: "http://localhost:11434", model: "qwen", hasApiKey: false, local: true });
        case "list_accounts":
          return Promise.resolve([{ id: "work", email: "me@work.example" }, { id: "home", email: "me@home.example" }]);
        case "get_message": return Promise.resolve({ id: (args as { messageId: string }).messageId, account_id: "work", subject: "Hiking" });
        default: return Promise.resolve(command.startsWith("list") ? [] : null);
      }
    });
  });

  it("shows answers with formatting, sources and drafts that open in the composer", async () => {
    const openCompose = vi.fn();
    useComposeStore.setState({ openCompose });
    wrap();
    const log = await screen.findByRole("log");
    expect(await within(log).findByText("Reply to Wei: I'm in")).toBeTruthy();
    expect(within(log).getByText("Wei's").tagName).toBe("STRONG");
    expect(within(log).getByText("Ridge trail").closest("li")).toBeTruthy();

    fireEvent.click(within(log).getByRole("button", { name: "Used 1 messages" }));
    expect(within(log).getByRole("button", { name: /Wei.*Hiking/ })).toBeTruthy();

    expect(within(log).getByText("Not sent. You review and send it.")).toBeTruthy();
    fireEvent.click(within(log).getByRole("button", { name: "Open in composer" }));
    await waitFor(() => expect(openCompose).toHaveBeenCalledWith("reply", expect.objectContaining({ id: "m3" }), { to: ["wei@x.example"], cc: [], body: "Count me in!" }));
  });

  it("offers a retry after an error", async () => {
    wrap();
    const log = await screen.findByRole("log");
    expect(await within(log).findByText("I couldn't answer that: HTTP 401: bad key")).toBeTruthy();
    fireEvent.click(within(log).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("agent_chat_send", { text: "And the contract?", accountIds: [] }));
  });

  it("sends with Enter in the chosen scope and keeps Shift+Enter for new lines", async () => {
    wrap();
    const input = await screen.findByRole("textbox", { name: "Ask Pebble about your mail…" });
    await screen.findByRole("combobox", { name: "Which accounts Pebble may read" });
    fireEvent.change(input, { target: { value: "What did Dana say?" } });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(mockInvoke).not.toHaveBeenCalledWith("agent_chat_send", expect.anything());
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("agent_chat_send", { text: "What did Dana say?", accountIds: ["work"] }));

    fireEvent.change(screen.getByRole("combobox", { name: "Which accounts Pebble may read" }), { target: { value: "all" } });
    fireEvent.change(input, { target: { value: "Anything urgent?" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("agent_chat_send", { text: "Anything urgent?", accountIds: null }));
  });

  it("shows progress while answering and can stop", async () => {
    chat = [entry({ id: "q9", role: "user", runId: "r9", text: "Anything urgent?", createdAt: NOW - 5 })];
    running = ["r9"];
    wrap();
    const log = await screen.findByRole("log");
    expect(await within(log).findByText("Thinking…")).toBeTruthy();
    fireEvent.click(within(log).getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("agent_chat_cancel", { runId: "r9" }));
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  });

  it("suggests first questions when the chat is empty", async () => {
    chat = [];
    wrap();
    fireEvent.click(await screen.findByRole("button", { name: "What's waiting for my reply?" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("agent_chat_send", { text: "What's waiting for my reply?", accountIds: ["work"] }));
  });
});

describe("RichText", () => {
  it("renders paragraphs, ordered lists and bold as text nodes only", () => {
    const { container } = render(<RichText text={"Hi <b>there</b>\n\n1. one\n2. **two**"} />);
    expect(container.querySelector("b")).toBeNull();
    expect(container.textContent).toContain("Hi <b>there</b>");
    expect(container.querySelectorAll("ol li")).toHaveLength(2);
    expect(container.querySelector("strong")?.textContent).toBe("two");
  });
});
