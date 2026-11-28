import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import NotificationsTab from "../../../src/features/notify/NotificationsTab";
import type { NotifyPrefs } from "../../../src/features/notify/api";
import { useUIStore } from "../../../src/stores/ui.store";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (_key: string, fallback?: string, vars?: Record<string, string>) =>
      (fallback ?? _key).replace(/\{\{(\w+)\}\}/g, (_, name) => vars?.[name] ?? ""),
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const mockInvoke = vi.mocked(invoke);
const defaults: NotifyPrefs = {
  mode: "smart", quietHours: null, modelAssist: false, modelReviewsBulk: false, modelSendBody: true,
  briefingEnabled: true, briefingMinute: 540, awaitingReplyEnabled: true, awaitingReplyHours: 24, followUpDays: 3,
};
const accounts = [
  { id: "work", email: "me@work.example", provider: "imap", account_label: null },
  { id: "gm", email: "me@gmail.com", provider: "gmail", account_label: null },
];

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><NotificationsTab /></QueryClientProvider>);
}

function aiSection() {
  return screen.findByText("AI help").then((title) => title.closest("section") as HTMLElement);
}

describe("AI help with notifications", () => {
  let model: { configured: boolean; endpoint: string; model: string; hasApiKey: boolean; local: boolean };
  let own: Record<string, NotifyPrefs>;
  let saved: Array<{ accountId: string | null; prefs: NotifyPrefs }>;
  beforeEach(() => {
    model = { configured: true, endpoint: "https://api.example.com", model: "m", hasApiKey: true, local: false };
    own = {};
    saved = [];
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command, args) => {
      switch (command) {
        case "get_notify_settings": return Promise.resolve({ defaults, accounts: own });
        case "list_accounts": return Promise.resolve(accounts);
        case "get_agent_model": return Promise.resolve(model);
        case "list_sender_prefs": return Promise.resolve([]);
        case "list_notification_log":
          return Promise.resolve([{
            accountId: "work", messageId: "m1", tier: "interrupt", reason: "model:promotion", why: "Sale ends today",
            delivered: true, createdAt: 1, seen: false, subject: "48h sale", fromName: "Shop", fromAddress: "d@shop.example",
            threadId: null, date: 1, feedback: null,
          }]);
        case "set_notify_prefs":
          saved.push(args as { accountId: string | null; prefs: NotifyPrefs });
          return Promise.resolve();
        case "notify_feedback": return Promise.resolve({ sender: "d@shop.example", suggest: null });
        default: return Promise.resolve(null);
      }
    });
  });

  it("asks for a model first", async () => {
    model = { ...model, configured: false };
    const setSettingsTab = vi.fn();
    useUIStore.setState({ setSettingsTab });
    renderTab();
    const section = await aiSection();
    fireEvent.click(await within(section).findByRole("button", { name: "Set up a model" }));
    expect(setSettingsTab).toHaveBeenCalledWith("assistant");
  });

  it("says what is sent where, turns it on per account and keeps Gmail out", async () => {
    renderTab();
    const section = await aiSection();
    expect(await within(section).findByText(/sent automatically to https:\/\/api\.example\.com/)).toBeTruthy();
    expect(within(section).getByText(/cloud service/)).toBeTruthy();
    const gmail = within(section).getByLabelText("AI help for me@gmail.com") as HTMLInputElement;
    expect(gmail.disabled).toBe(true);
    expect(within(section).getByText("Not available for Gmail accounts yet")).toBeTruthy();

    fireEvent.click(within(section).getByLabelText("AI help for me@work.example"));
    await waitFor(() => expect(saved.at(-1)).toEqual({ accountId: "work", prefs: { ...defaults, modelAssist: true } }));
  });

  it("applies the shared options to every account using AI", async () => {
    own = { work: { ...defaults, modelAssist: true } };
    renderTab();
    const section = await aiSection();
    fireEvent.click(await within(section).findByLabelText(/Include the start of the message text/));
    await waitFor(() => expect(saved.at(-1)).toEqual({ accountId: "work", prefs: { ...defaults, modelAssist: true, modelSendBody: false } }));
  });

  it("shows the AI's reason in the log and takes a correction", async () => {
    renderTab();
    expect(await screen.findByText(/✦ Sale ends today/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Not important" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("notify_feedback", { accountId: "work", messageId: "m1", important: false }));
  });
});
