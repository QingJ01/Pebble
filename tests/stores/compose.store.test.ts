import { beforeEach, describe, expect, it } from "vitest";
import { useComposeStore } from "../../src/stores/compose.store";
import { useUIStore } from "../../src/stores/ui.store";

describe("ComposeStore", () => {
  it("clears resumed draft state and completes deferred navigation only after cleanup confirmation", () => {
    const draft = { id: "draft", accountId: "account", to: [], cc: [], bcc: [], subject: "Saved",
      rawSource: "Body", richTextHtml: "", editorMode: "markdown" as const, attachments: [], savedAt: 1 };
    useComposeStore.getState().openDraft(draft);
    expect(useComposeStore.getState().composeDraft).toEqual(draft);
    useComposeStore.getState().setComposeDirty(true);
    useComposeStore.getState().openCompose("new", null, { subject: "New mailto" });
    expect(useComposeStore.getState().showComposeLeaveConfirm).toBe(true);
    expect(useComposeStore.getState().composeDraft).toEqual(draft);
    useComposeStore.getState().confirmCloseCompose();
    expect(useComposeStore.getState().composeDraft).toBeNull();
    expect(useComposeStore.getState().composePrefill).toEqual({ subject: "New mailto" });
    expect(useComposeStore.getState().pendingNavigation).toBeNull();
  });
  beforeEach(() => {
    useUIStore.setState({
      activeView: "inbox",
      previousView: "inbox",
    });
    useComposeStore.setState({
      composeMode: null,
      composeReplyTo: null,
      composeDirty: false,
      showComposeLeaveConfirm: false,
      pendingView: null,
      composePrefill: null,
      composeKey: 0,
    });
  });

  it("opens a new compose with mailto prefill data", () => {
    useComposeStore.getState().openCompose("new", null, {
      to: ["alice@example.com"],
      cc: ["carol@example.com"],
      bcc: [],
      subject: "Hello",
      body: "Line one",
    });

    const compose = useComposeStore.getState();
    expect(useUIStore.getState().activeView).toBe("compose");
    expect(compose.composeMode).toBe("new");
    expect(compose.composePrefill).toEqual({
      to: ["alice@example.com"],
      cc: ["carol@example.com"],
      bcc: [],
      subject: "Hello",
      body: "Line one",
    });
    expect(compose.composeKey).toBe(1);
  });
});
