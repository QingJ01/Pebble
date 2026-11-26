import { describe, it, expect, beforeEach } from "vitest";
import { isThreadSelected, useMailStore } from "../../src/stores/mail.store";

describe("MailStore", () => {
  beforeEach(() => {
    useMailStore.setState({
      activeAccountId: null,
      activeFolderId: null,
      selectedMessageId: null,
      selectedThreadId: null,
      selectedThreadAccountId: null,
      threadView: false,
    });
  });

  it("should have correct initial state", () => {
    const state = useMailStore.getState();
    expect(state.activeAccountId).toBeNull();
    expect(state.activeFolderId).toBeNull();
    expect(state.selectedMessageId).toBeNull();
    expect(state.selectedThreadId).toBeNull();
    expect(state.threadView).toBe(false);
  });

  it("should set selected message", () => {
    useMailStore.getState().setSelectedMessage("msg-1");
    expect(useMailStore.getState().selectedMessageId).toBe("msg-1");

    useMailStore.getState().setSelectedMessage(null);
    expect(useMailStore.getState().selectedMessageId).toBeNull();
  });

  it("should set active account and reset dependent state", () => {
    useMailStore.setState({
      activeFolderId: "f1",
      selectedMessageId: "m1",
      selectedThreadId: "t1",
    });

    useMailStore.getState().setActiveAccountId("a1");

    const state = useMailStore.getState();
    expect(state.activeAccountId).toBe("a1");
    expect(state.activeFolderId).toBeNull();
    expect(state.selectedMessageId).toBeNull();
    expect(state.selectedThreadId).toBeNull();
  });

  it("should set active folder and reset message/thread selection", () => {
    useMailStore.setState({
      activeAccountId: "a1",
      selectedMessageId: "m1",
      selectedThreadId: "t1",
    });

    useMailStore.getState().setActiveFolderId("f2");

    const state = useMailStore.getState();
    expect(state.activeFolderId).toBe("f2");
    expect(state.selectedMessageId).toBeNull();
    expect(state.selectedThreadId).toBeNull();
    expect(state.activeAccountId).toBe("a1");
  });

  it("should set selected thread", () => {
    useMailStore.getState().setSelectedThreadId("t1");
    expect(useMailStore.getState().selectedThreadId).toBe("t1");
  });

  it("should toggle thread view and reset selections", () => {
    useMailStore.setState({
      threadView: false,
      selectedMessageId: "m1",
      selectedThreadId: "t1",
    });

    useMailStore.getState().toggleThreadView();

    const state = useMailStore.getState();
    expect(state.threadView).toBe(true);
    expect(state.selectedMessageId).toBeNull();
    expect(state.selectedThreadId).toBeNull();
  });

  it("selects threads by (account_id, thread_id)", () => {
    useMailStore.getState().selectThread({ thread_id: "t1", account_id: "a2" });
    const state = useMailStore.getState();
    expect(state.selectedThreadId).toBe("t1");
    expect(state.selectedThreadAccountId).toBe("a2");
    expect(isThreadSelected(state, { thread_id: "t1", account_id: "a2" })).toBe(true);
    expect(isThreadSelected(state, { thread_id: "t1", account_id: "a1" })).toBe(false);
    expect(isThreadSelected(state, { thread_id: "t2", account_id: "a2" })).toBe(false);

    useMailStore.getState().selectThread(null);
    expect(useMailStore.getState().selectedThreadAccountId).toBeNull();
  });

  it("clears the selected thread's account with the thread", () => {
    useMailStore.getState().setSelectedThreadId("t1", "a1");
    useMailStore.getState().setActiveFolderId("f1");
    expect(useMailStore.getState().selectedThreadAccountId).toBeNull();
    useMailStore.getState().setSelectedThreadId("t1", "a1");
    useMailStore.getState().toggleThreadView();
    expect(useMailStore.getState().selectedThreadAccountId).toBeNull();
  });

  it("keeps the unified account filter across unified folders and clears it when leaving", () => {
    const store = useMailStore.getState();
    store.setActiveAccountId(null);
    store.setActiveFolderId("all:inbox");
    useMailStore.setState({ selectedMessageId: "m-1", batchMode: true });

    useMailStore.getState().setUnifiedAccountFilter("acct-2");
    expect(useMailStore.getState().unifiedAccountFilter).toBe("acct-2");
    // Changing the filter drops the selection from the old list.
    expect(useMailStore.getState().selectedMessageId).toBeNull();
    expect(useMailStore.getState().batchMode).toBe(false);

    useMailStore.getState().setActiveFolderId("all:sent");
    expect(useMailStore.getState().unifiedAccountFilter).toBe("acct-2");

    useMailStore.getState().setActiveFolderId("acct-1-inbox");
    expect(useMailStore.getState().unifiedAccountFilter).toBeNull();

    useMailStore.getState().setUnifiedAccountFilter("acct-1");
    useMailStore.getState().setActiveAccountId("acct-1");
    expect(useMailStore.getState().unifiedAccountFilter).toBeNull();
  });
});
