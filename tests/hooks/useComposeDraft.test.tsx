import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useComposeDraft } from "../../src/hooks/useComposeDraft";
import { saveDraft } from "../../src/lib/api";

vi.mock("../../src/lib/api", () => ({
  saveDraft: vi.fn(),
}));

const saveDraftMock = vi.mocked(saveDraft);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function baseProps(overrides: Partial<Parameters<typeof useComposeDraft>[0]> = {}) {
  return {
    to: [],
    cc: [],
    bcc: [],
    subject: "",
    rawSource: "",
    richTextHtml: "",
    editorMode: "rich" as const,
    composeMode: "new",
    fromAccountId: "account-1",
    editorReady: true,
    attachments: [],
    ...overrides,
  };
}

describe("useComposeDraft", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    saveDraftMock.mockReset();
  });

  it("autosaves attachments only to the backend draft", async () => {
    saveDraftMock.mockResolvedValue("draft-1");

    renderHook((props) => useComposeDraft(props), {
      initialProps: baseProps({
        attachments: [{ name: "report.pdf", path: "C:\\tmp\\report.pdf", size: 1234 }],
      }) as Parameters<typeof useComposeDraft>[0],
    });

    act(() => {
      vi.advanceTimersByTime(3000);
    });

    expect(saveDraftMock).toHaveBeenCalledOnce();
    expect(saveDraftMock).toHaveBeenCalledWith(expect.objectContaining({
      attachmentPaths: ["C:\\tmp\\report.pdf"],
    }));
    expect(localStorage.getItem("pebble-compose-draft")).toBeNull();
  });

  it("does not reuse a stale draft id after switching accounts", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    saveDraftMock
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
      .mockResolvedValue("draft-account-2-later");

    const { rerender } = renderHook((props) => useComposeDraft(props), {
      initialProps: baseProps({ subject: "A", fromAccountId: "account-1" }) as Parameters<typeof useComposeDraft>[0],
    });

    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(saveDraftMock).toHaveBeenCalledWith(expect.objectContaining({
      accountId: "account-1",
      existingDraftId: undefined,
    }));

    rerender(baseProps({ subject: "B", fromAccountId: "account-2" }) as Parameters<typeof useComposeDraft>[0]);
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(saveDraftMock).toHaveBeenLastCalledWith(expect.objectContaining({
      accountId: "account-2",
      existingDraftId: undefined,
    }));

    await act(async () => {
      first.resolve("draft-account-1");
      await first.promise;
    });

    rerender(baseProps({ subject: "B updated", fromAccountId: "account-2" }) as Parameters<typeof useComposeDraft>[0]);
    act(() => {
      vi.advanceTimersByTime(3000);
    });

    expect(saveDraftMock).toHaveBeenLastCalledWith(expect.objectContaining({
      accountId: "account-2",
      existingDraftId: undefined,
    }));
  });
});
