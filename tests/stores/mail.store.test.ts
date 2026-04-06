import { describe, it, expect, beforeEach } from "vitest";
import { useMailStore } from "../../src/stores/mail.store";

describe("MailStore", () => {
  beforeEach(() => {
    useMailStore.setState({
      activeAccountId: null,
      activeFolderId: null,
      selectedMessageId: null,
      selectedThreadId: null,
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
});
