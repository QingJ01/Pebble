import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useComposeStore } from "../../src/stores/compose.store";
import { useMailStore } from "../../src/stores/mail.store";
import { realtimePreferenceToPollInterval, useUIStore } from "../../src/stores/ui.store";

describe("UIStore", () => {
  beforeEach(() => {
    useUIStore.setState({
      sidebarCollapsed: false,
      activeView: "inbox",
      theme: "light",
      language: "en",
      syncStatus: "idle",
      networkStatus: "online",
      lastMailError: null,
      realtimeStatusByAccount: {},
      previousView: "inbox",
      pollInterval: 15,
      realtimeMode: "realtime",
      searchQuery: "",
      settingsTab: "accounts",
      pendingRuleDraftText: null,
      showFolderUnreadCount: false,
    });
    useComposeStore.setState({
      composeMode: null,
      composeReplyTo: null,
      composeDirty: false,
      showComposeLeaveConfirm: false,
      pendingView: null,
    });
    useMailStore.setState({
      activeAccountId: null,
      activeFolderId: null,
      selectedMessageId: null,
      selectedThreadId: null,
      threadView: false,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("should have correct initial state", () => {
    const state = useUIStore.getState();
    expect(state.sidebarCollapsed).toBe(false);
    expect(state.activeView).toBe("inbox");
    expect(state.theme).toBe("light");
    expect(state.syncStatus).toBe("idle");
    expect(state.realtimeMode).toBe("realtime");
  });

  it("should toggle sidebar", () => {
    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarCollapsed).toBe(true);
    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarCollapsed).toBe(false);
  });

  it("should set active view", () => {
    useUIStore.getState().setActiveView("kanban");
    expect(useUIStore.getState().activeView).toBe("kanban");
    useUIStore.getState().setActiveView("settings");
    expect(useUIStore.getState().activeView).toBe("settings");
  });

  it("opens a message in inbox by clearing stale thread selection", () => {
    useUIStore.setState({ activeView: "snoozed" });
    useMailStore.setState({
      selectedMessageId: null,
      selectedThreadId: "thread-1",
      threadView: true,
      selectedMessageIds: new Set(["message-1"]),
      batchMode: true,
    });

    useUIStore.getState().openMessageInInbox("message-2");

    expect(useUIStore.getState().activeView).toBe("inbox");
    expect(useMailStore.getState().selectedMessageId).toBe("message-2");
    expect(useMailStore.getState().selectedThreadId).toBe(null);
    expect(useMailStore.getState().threadView).toBe(false);
    expect(useMailStore.getState().selectedMessageIds.size).toBe(0);
    expect(useMailStore.getState().batchMode).toBe(false);
  });

  it("stores context navigation state for selected-text actions", () => {
    useUIStore.getState().setSearchQuery("invoice total");
    useUIStore.getState().setSettingsTab("rules");
    useUIStore.getState().setPendingRuleDraftText("unsubscribe");

    const state = useUIStore.getState();
    expect(state.searchQuery).toBe("invoice total");
    expect(state.settingsTab).toBe("rules");
    expect(state.pendingRuleDraftText).toBe("unsubscribe");
  });

  it("keeps the user on compose when dirty and shows confirmation", () => {
    useUIStore.setState({
      activeView: "compose",
      previousView: "inbox",
    });
    useComposeStore.setState({
      composeMode: "new",
      composeDirty: true,
    });

    useUIStore.getState().setActiveView("search");

    const state = useUIStore.getState();
    const composeState = useComposeStore.getState();
    // Should stay on compose and show confirmation dialog
    expect(state.activeView).toBe("compose");
    expect(composeState.composeMode).toBe("new");
    expect(composeState.composeDirty).toBe(true);
    expect(composeState.showComposeLeaveConfirm).toBe(true);
    expect(composeState.pendingView).toBe("search");
  });

  it("closeCompose respects unsaved-draft protection", () => {
    useUIStore.setState({
      activeView: "compose",
      previousView: "kanban",
    });
    useComposeStore.setState({
      composeMode: "reply",
      composeDirty: true,
    });

    useComposeStore.getState().closeCompose();

    const state = useUIStore.getState();
    const composeState = useComposeStore.getState();
    // Should stay on compose and show confirmation dialog
    expect(state.activeView).toBe("compose");
    expect(composeState.composeMode).toBe("reply");
    expect(composeState.composeDirty).toBe(true);
    expect(composeState.showComposeLeaveConfirm).toBe(true);
  });

  it("confirmCloseCompose navigates away and clears compose state", () => {
    useUIStore.setState({
      activeView: "compose",
      previousView: "kanban",
    });
    useComposeStore.setState({
      composeMode: "forward",
      composeReplyTo: { id: "message-1" } as never,
      composeDirty: true,
      showComposeLeaveConfirm: true,
      pendingView: null,
    });

    useComposeStore.getState().confirmCloseCompose();

    const state = useUIStore.getState();
    const composeState = useComposeStore.getState();
    expect(state.activeView).toBe("kanban");
    expect(composeState.composeMode).toBe(null);
    expect(composeState.composeReplyTo).toBe(null);
    expect(composeState.composeDirty).toBe(false);
    expect(composeState.showComposeLeaveConfirm).toBe(false);
  });

  it("confirmCloseCompose navigates to pendingView when set", () => {
    useUIStore.setState({
      activeView: "compose",
      previousView: "inbox",
    });
    useComposeStore.setState({
      composeMode: "new",
      composeDirty: true,
      showComposeLeaveConfirm: true,
      pendingView: "search",
    });

    useComposeStore.getState().confirmCloseCompose();

    const state = useUIStore.getState();
    const composeState = useComposeStore.getState();
    expect(state.activeView).toBe("search");
    expect(composeState.composeMode).toBe(null);
    expect(composeState.showComposeLeaveConfirm).toBe(false);
    expect(composeState.pendingView).toBe(null);
  });

  it("cancelCloseCompose clears confirmation state", () => {
    useUIStore.setState({
      activeView: "compose",
      previousView: "inbox",
    });
    useComposeStore.setState({
      composeMode: "new",
      composeDirty: true,
      showComposeLeaveConfirm: true,
      pendingView: "kanban",
    });

    useComposeStore.getState().cancelCloseCompose();

    const state = useUIStore.getState();
    const composeState = useComposeStore.getState();
    expect(state.activeView).toBe("compose");
    expect(composeState.composeMode).toBe("new");
    expect(composeState.composeDirty).toBe(true);
    expect(composeState.showComposeLeaveConfirm).toBe(false);
    expect(composeState.pendingView).toBe(null);
  });

  it("should set theme", () => {
    useUIStore.getState().setTheme("dark");
    expect(useUIStore.getState().theme).toBe("dark");
  });

  it("should set sync status", () => {
    useUIStore.getState().setSyncStatus("syncing");
    expect(useUIStore.getState().syncStatus).toBe("syncing");
    useUIStore.getState().setSyncStatus("error");
    expect(useUIStore.getState().syncStatus).toBe("error");
  });

  it("maps realtime preferences to backend poll intervals", () => {
    expect(realtimePreferenceToPollInterval("realtime")).toBe(10);
    expect(realtimePreferenceToPollInterval("balanced")).toBe(30);
    expect(realtimePreferenceToPollInterval("battery")).toBe(120);
    expect(realtimePreferenceToPollInterval("manual")).toBe(0);
  });
});
