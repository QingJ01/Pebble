import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createInstance } from "i18next";
import { I18nextProvider, initReactI18next } from "react-i18next";
import ScheduledSendsView from "../../../src/features/compose/ScheduledSendsView";
import SendControlExitGuard from "../../../src/features/compose/SendControlExitGuard";
import SendControlsStatus from "../../../src/features/compose/SendControlsStatus";
import { useSendControlsStore } from "../../../src/stores/send-controls.store";
import { cleanupStagedComposeAttachment } from "../../../src/lib/api";
import { deleteControlledSend, listControlledSends, rescheduleControlledSend, resolveSendControlExit, restoreControlledSend, sendControlExitPending, type ControlledSend } from "../../../src/lib/send-controls-api";
import { useComposeStore } from "../../../src/stores/compose.store";
import { useUIStore } from "../../../src/stores/ui.store";
import en from "../../../src/locales/en.json";
import zh from "../../../src/locales/zh.json";
import sendEn from "../../../src/locales/send-controls.en.json";
import sendZh from "../../../src/locales/send-controls.zh.json";

const mocks = vi.hoisted(() => ({ confirm: vi.fn(), callbacks: {} as Record<string, () => void> }));
vi.mock("../../../src/lib/send-controls-api", () => ({
  listControlledSends: vi.fn(), restoreControlledSend: vi.fn(), rescheduleControlledSend: vi.fn(), deleteControlledSend: vi.fn(), resolveSendControlExit: vi.fn(), sendControlExitPending: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, cb: () => void) => { mocks.callbacks[name] = cb; return () => { delete mocks.callbacks[name]; }; }) }));
vi.mock("../../../src/stores/confirm.store", () => ({ useConfirmStore: { getState: () => ({ confirm: mocks.confirm }) } }));
vi.mock("../../../src/lib/api", () => ({ cleanupStagedComposeAttachment: vi.fn().mockResolvedValue(undefined) }));

const job: ControlledSend = { id: "job", accountId: "a", subject: "A scheduled message", dueAt: 2_000_000_000, state: "pending", kind: "scheduled", revision: 5 };
async function mount(element: React.ReactNode, language = "en") {
  const i18n = createInstance();
  await i18n.use(initReactI18next).init({ lng: language, fallbackLng: "en", resources: { en: { translation: { ...en, ...sendEn } }, zh: { translation: { ...zh, ...sendZh } } }, interpolation: { escapeValue: false } });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return { ...render(<I18nextProvider i18n={i18n}><QueryClientProvider client={client}>{element}</QueryClientProvider></I18nextProvider>), client };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.confirm.mockResolvedValue(true);
  vi.mocked(listControlledSends).mockResolvedValue([job]);
  vi.mocked(sendControlExitPending).mockResolvedValue(false);
  vi.mocked(resolveSendControlExit).mockResolvedValue(undefined);
  useUIStore.setState({ activeView: "settings" });
  useComposeStore.setState({ composeDirty: false, composeDraft: null });
  useSendControlsStore.setState({ composeUndoId: null });
});
afterEach(() => { cleanup(); localStorage.clear(); });

describe("ScheduledSendsView with real locales", () => {
  it.each(["en", "zh"])("renders actual %s resources, policy, timezone and states", async (language) => {
    await mount(<ScheduledSendsView />, language);
    await screen.findByText(job.subject);
    expect(screen.getByRole("heading", { name: language === "zh" ? "定时发送" : "Scheduled sends" })).toBeTruthy();
    expect(screen.getByText(language === "zh" ? sendZh.sendControls.policy : sendEn.sendControls.policy)).toBeTruthy();
    expect(screen.getByLabelText(new RegExp(Intl.DateTimeFormat().resolvedOptions().timeZone))).toBeTruthy();
    expect(document.body.textContent).not.toContain("sendControls.");
  });
  it("restores full editable content, BCC, reply mode and staged attachment paths", async () => {
    vi.mocked(restoreControlledSend).mockResolvedValue({ accountId: "a", to: ["to@example.com"], cc: [], bcc: ["private@example.com"], subject: "Subject", bodyText: "body", editorMode: "markdown", composeMode: "reply-all", rawSource: "**body**", richTextHtml: "<p>body</p>", quotedReplyHtml: "<blockquote>quote</blockquote>", attachments: [{ name: "file.txt", path: "/staged/file.txt", size: 20 }] });
    await mount(<ScheduledSendsView />);
    fireEvent.click(await screen.findByRole("button", { name: "Cancel / restore to draft" }));
    await waitFor(() => expect(useComposeStore.getState().composeDraft?.rawSource).toBe("**body**"));
    expect(restoreControlledSend).toHaveBeenCalledWith(job);
    expect(useComposeStore.getState().composeMode).toBe("reply-all");
    expect(useComposeStore.getState().composeDraft?.bcc).toEqual(["private@example.com"]);
    expect(useComposeStore.getState().composeDraft?.attachments[0].path).toBe("/staged/file.txt");
  });
  it("does not offer cancel/reschedule for an in-flight message", async () => {
    vi.mocked(listControlledSends).mockResolvedValue([{ ...job, state: "dispatching" }]);
    await mount(<ScheduledSendsView />); await screen.findByText(job.subject);
    expect(screen.queryByRole("button", { name: "Cancel / restore to draft" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reschedule" })).toBeNull();
  });
  it("requires explicit review after interruption instead of offering blind reschedule", async () => {
    vi.mocked(listControlledSends).mockResolvedValue([{ ...job, state: "review" }]);
    await mount(<ScheduledSendsView />);
    await screen.findByText(sendEn.sendControls.reviewWarning);
    expect(screen.queryByRole("button", { name: "Reschedule" })).toBeNull();
    expect(restoreControlledSend).not.toHaveBeenCalled();
    expect(rescheduleControlledSend).not.toHaveBeenCalled();
  });
  it("reports a stale revision race instead of claiming cancellation succeeded", async () => {
    vi.mocked(restoreControlledSend).mockRejectedValue(new Error("already dispatching"));
    await mount(<ScheduledSendsView />);
    fireEvent.click(await screen.findByRole("button", { name: "Cancel / restore to draft" }));
    await screen.findByRole("alert");
    expect(useComposeStore.getState().composeDraft).toBeNull();
  });
  it("reschedules to a validated UTC instant using the observed revision", async () => {
    vi.mocked(rescheduleControlledSend).mockResolvedValue(undefined);
    await mount(<ScheduledSendsView />); await screen.findByText(job.subject);
    const input = screen.getByLabelText(new RegExp(Intl.DateTimeFormat().resolvedOptions().timeZone));
    const date = new Date(Date.now() + 86400000);
    const value = `${date.getFullYear()}-${String(date.getMonth()+1).padStart(2,"0")}-${String(date.getDate()).padStart(2,"0")}T12:30`;
    fireEvent.change(input, { target: { value } });
    fireEvent.click(screen.getByRole("button", { name: "Reschedule" }));
    await waitFor(() => expect(rescheduleControlledSend).toHaveBeenCalledWith(job, new Date(value).getTime()/1000));
  });
  it("only removes a restored backup after a destructive confirmation", async () => {
    vi.mocked(listControlledSends).mockResolvedValue([{ ...job, state: "draft" }]);
    vi.mocked(deleteControlledSend).mockResolvedValue(undefined);
    await mount(<ScheduledSendsView />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove backup" }));
    await waitFor(() => expect(deleteControlledSend).toHaveBeenCalledWith({ ...job, state: "draft" }));
    expect(mocks.confirm).toHaveBeenCalledWith(expect.objectContaining({ destructive: true }));
  });
});

describe("persistent Undo status", () => {
  it("keeps a lost-receipt Undo visible in an active dirty compose without overwriting newer edits", async () => {
    const pending: ControlledSend = { ...job, kind: "undo" };
    vi.mocked(listControlledSends).mockResolvedValue([pending]);
    vi.mocked(restoreControlledSend).mockResolvedValue({ accountId: "a", to: ["to@example.com"], cc: [], bcc: [], subject: "Original", bodyText: "body", editorMode: "rich", composeMode: "new", rawSource: "body", richTextHtml: "body", quotedReplyHtml: "", attachments: [{ name: "old.txt", path: "/staged/copy.txt", size: 4 }] });
    useUIStore.setState({ activeView: "compose" });
    useComposeStore.setState({ composeDirty: true, composeDraft: { accountId: "a", subject: "Newer edits", bodyText: "changed", attachments: [], savedAt: Date.now() } });
    await mount(<SendControlsStatus />);
    fireEvent.click(await screen.findByRole("button", { name: "Undo send" }));
    await waitFor(() => expect(restoreControlledSend).toHaveBeenCalledWith(pending));
    expect(useComposeStore.getState().composeDraft?.subject).toBe("Newer edits");
    expect(cleanupStagedComposeAttachment).toHaveBeenCalledWith("/staged/copy.txt");
  });
  it("does not duplicate the countdown owned by the mounted compose view", async () => {
    vi.mocked(listControlledSends).mockResolvedValue([{ ...job, kind: "undo" }]);
    useSendControlsStore.setState({ composeUndoId: job.id });
    await mount(<SendControlsStatus />);
    await waitFor(() => expect(listControlledSends).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: "Undo send" })).toBeNull();
  });
});

describe("native exit request bridge", () => {
  it("handles an exit request that arrived before frontend listeners were mounted", async () => {
    vi.mocked(sendControlExitPending).mockResolvedValue(true);
    await mount(<SendControlExitGuard />, "zh");
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByRole("button", { name: "继续运行" }));
    await waitFor(() => expect(resolveSendControlExit).toHaveBeenCalledWith(false));
  });
  it("confirms a native request and keeps the warning visible if confirmation fails", async () => {
    vi.mocked(resolveSendControlExit).mockRejectedValue(new Error("storage unavailable"));
    await mount(<SendControlExitGuard />);
    await waitFor(() => expect(mocks.callbacks["send-controls-exit-requested"]).toBeTypeOf("function"));
    act(() => mocks.callbacks["send-controls-exit-requested"]());
    fireEvent.click(screen.getByRole("button", { name: "Quit Pebble" }));
    await screen.findByRole("alert");
    expect(resolveSendControlExit).toHaveBeenCalledWith(true);
    expect(screen.getByRole("dialog")).toBeTruthy();
  });
});
