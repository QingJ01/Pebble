import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import NotificationsTab from "../../../src/features/notify/NotificationsTab";
import { effectiveSenderPref } from "../../../src/features/notify/SenderNotifyAction";
import { minutesToTime, timeToMinutes, type NotifyPrefs, type SenderPref } from "../../../src/features/notify/api";
import { reasonLabel } from "../../../src/features/notify/labels";
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
const defaults: NotifyPrefs = { mode: "all", quietHours: null, modelAssist: false, modelReviewsBulk: false, modelSendBody: true };

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <NotificationsTab />
    </QueryClientProvider>,
  );
}

describe("NotificationsTab", () => {
  let saved: Array<{ accountId: string | null; prefs: NotifyPrefs }>;
  beforeEach(() => {
    saved = [];
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command, args) => {
      switch (command) {
        case "get_notify_settings":
          return Promise.resolve({ defaults, accounts: {} });
        case "list_accounts":
          return Promise.resolve([]);
        case "list_sender_prefs":
          return Promise.resolve([{ account_id: "", pattern: "@shop.example", kind: "mute", created_at: 1 }]);
        case "list_notification_log":
          return Promise.resolve([
            {
              accountId: "a", messageId: "m1", tier: "silent", reason: "rule_read", why: null, delivered: false,
              createdAt: 1, seen: false, subject: "Weekly deals", fromName: "Shop", fromAddress: "d@shop.example",
              threadId: null, date: 1,
            },
          ]);
        case "set_notify_prefs":
          saved.push(args as { accountId: string | null; prefs: NotifyPrefs });
          return Promise.resolve();
        default:
          return Promise.resolve(null);
      }
    });
  });

  it("saves the mode and quiet hours and lists senders and decisions", async () => {
    renderTab();
    const smart = await screen.findByRole("button", { name: /Smart/ });
    await waitFor(() => expect((smart as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(smart);
    await waitFor(() => expect(saved.at(-1)?.prefs.mode).toBe("smart"));
    expect(saved.at(-1)?.accountId).toBeNull();

    fireEvent.click(screen.getByLabelText("Use quiet hours"));
    await waitFor(() => expect(saved.at(-1)?.prefs.quietHours).toMatchObject({ startMinute: 1320, endMinute: 420 }));

    expect(await screen.findByText("@shop.example")).toBeTruthy();
    expect(await screen.findByText("Weekly deals")).toBeTruthy();
    expect(screen.getByText(/A rule marked it read/)).toBeTruthy();
  });

  it("keeps the desktop notification switch and test button (moved from General)", async () => {
    useUIStore.setState({ notificationsEnabled: true });
    renderTab();
    const checkbox = screen.getByRole("checkbox", { name: "Enable desktop notifications" });
    expect((checkbox as HTMLInputElement).checked).toBe(true);
    fireEvent.click(checkbox);
    expect(useUIStore.getState().notificationsEnabled).toBe(false);
    useUIStore.setState({ notificationsEnabled: true });
    fireEvent.click(await screen.findByRole("button", { name: "Send test notification" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("show_test_notification"));
  });

  it("adds a VIP from the form", async () => {
    renderTab();
    fireEvent.change(await screen.findByLabelText("Address or domain"), { target: { value: "boss@corp.example" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_sender_pref", { accountId: null, pattern: "boss@corp.example", kind: "vip" }),
    );
  });
});

describe("notify helpers", () => {
  it("resolves the most specific sender preference", () => {
    const prefs: SenderPref[] = [
      { account_id: "", pattern: "@shop.example", kind: "mute", created_at: 1 },
      { account_id: "a1", pattern: "boss@shop.example", kind: "vip", created_at: 2 },
    ];
    expect(effectiveSenderPref(prefs, "a1", "Boss@Shop.example")?.kind).toBe("vip");
    expect(effectiveSenderPref(prefs, "a2", "boss@shop.example")?.kind).toBe("mute");
    expect(effectiveSenderPref(prefs, "a1", "ann@else.example")).toBeNull();
    expect(effectiveSenderPref(prefs, "a1", "nope")).toBeNull();
  });

  it("converts quiet-hour times", () => {
    expect(minutesToTime(22 * 60 + 5)).toBe("22:05");
    expect(timeToMinutes("07:30")).toBe(450);
    expect(timeToMinutes("24:00")).toBeNull();
    expect(timeToMinutes("x")).toBeNull();
  });

  it("labels model reasons", () => {
    const t = ((key: string, fallback?: string, vars?: Record<string, string>) =>
      (fallback ?? key).replace(/\{\{(\w+)\}\}/g, (_, n) => vars?.[n] ?? "")) as never;
    expect(reasonLabel(t, "model:needs_action")).toBe("AI: Needs your action");
    expect(reasonLabel(t, "vip")).toBe("VIP");
    expect(reasonLabel(t, "unknown_code")).toBe("unknown_code");
  });
});

describe("SenderNotifyAction", () => {
  it("mutes a sender from the reader's bell menu", async () => {
    const { default: SenderNotifyAction } = await import("../../../src/features/notify/SenderNotifyAction");
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command) => {
      if (command === "list_sender_prefs") return Promise.resolve([]);
      if (command === "list_accounts") return Promise.resolve([{ id: "a1", email: "me@pebble.example" }]);
      return Promise.resolve("deals@shop.example");
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <SenderNotifyAction accountId="a1" address="Deals@Shop.example" />
      </QueryClientProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Notifications for this sender" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: /Mute this sender/ }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_sender_pref", { accountId: null, pattern: "deals@shop.example", kind: "mute" }),
    );
  });

  it("hides itself for the user's own address", async () => {
    const { default: SenderNotifyAction } = await import("../../../src/features/notify/SenderNotifyAction");
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command) =>
      Promise.resolve(command === "list_accounts" ? [{ id: "a1", email: "me@pebble.example" }] : []),
    );
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const { container } = render(
      <QueryClientProvider client={client}>
        <SenderNotifyAction accountId="a1" address="me@pebble.example" />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("list_accounts"));
    await waitFor(() => expect(container.querySelector("button")).toBeNull());
  });
});
