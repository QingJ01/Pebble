import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Folder, MessageSummary } from "../../src/lib/api";

const mocks = vi.hoisted(() => ({
  archiveMessage: vi.fn(),
  batchArchive: vi.fn(),
  batchDelete: vi.fn(),
  deleteMessage: vi.fn(),
  listFolders: vi.fn(),
  listThreadMessages: vi.fn(),
  moveToFolder: vi.fn(),
  getMailMoveContext: vi.fn(),
  moveMailMessages: vi.fn(),
}));

vi.mock("../../src/lib/api", () => ({
  archiveMessage: mocks.archiveMessage,
  batchArchive: mocks.batchArchive,
  batchDelete: mocks.batchDelete,
  deleteMessage: mocks.deleteMessage,
  listFolders: mocks.listFolders,
  listThreadMessages: mocks.listThreadMessages,
  moveToFolder: mocks.moveToFolder,
}));

vi.mock("../../src/lib/folders-api", () => ({
  getMailMoveContext: mocks.getMailMoveContext,
  moveMailMessages: mocks.moveMailMessages,
}));

vi.mock("i18next", () => ({
  default: {
    t: (_key: string, options?: { defaultValue?: string; count?: number }) =>
      (options?.defaultValue ?? _key).replace("{{count}}", String(options?.count ?? "")),
  },
}));

import { executeMailAction, moveMessagesWithUndo, planMailAction, runMailAction } from "../../src/hooks/mutations/mailActions";
import { useToastStore } from "../../src/stores/toast.store";
import { useMailStore } from "../../src/stores/mail.store";

function folder(id: string, role: Folder["role"]): Folder {
  return {
    id,
    account_id: "acc",
    remote_id: id,
    name: id,
    folder_type: "folder",
    role,
    parent_id: null,
    color: null,
    is_system: true,
    sort_order: 0,
  } as Folder;
}

const FOLDERS = [folder("inbox", "inbox"), folder("archive", "archive"), folder("trash", "trash"), folder("spam", "spam")];

function message(id: string, overrides: Partial<MessageSummary> = {}): MessageSummary {
  return {
    id,
    account_id: "acc",
    remote_id: `r-${id}`,
    message_id_header: null,
    in_reply_to: null,
    references_header: null,
    thread_id: null,
    subject: id,
    snippet: "",
    from_address: "a@example.com",
    from_name: "A",
    to_list: [],
    cc_list: [],
    bcc_list: [],
    has_attachments: false,
    is_read: true,
    is_starred: false,
    is_draft: false,
    date: 1,
    remote_version: null,
    is_deleted: false,
    deleted_at: null,
    created_at: 1,
    updated_at: 1,
    ...overrides,
  };
}

function seedList(client: QueryClient, messages: MessageSummary[]) {
  client.setQueryData(["messages", "inbox"], { pages: [messages], pageParams: [null] });
}

function listIds(client: QueryClient) {
  const data = client.getQueryData<{ pages: MessageSummary[][] }>(["messages", "inbox"]);
  return data?.pages.flat().map((m) => m.id) ?? [];
}

function lastToast() {
  const toasts = useToastStore.getState().toasts;
  return toasts[toasts.length - 1];
}

describe("mail actions with undo", () => {
  let client: QueryClient;

  beforeEach(() => {
    vi.clearAllMocks();
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    useToastStore.getState().clearToasts();
    useMailStore.setState({ activeFolderId: "inbox", activeAccountId: "acc" });
    mocks.listFolders.mockResolvedValue(FOLDERS);
    mocks.getMailMoveContext.mockImplementation(async (_account: string, ids: string[]) => ({
      provider: "imap",
      folder_ids_by_message: Object.fromEntries(ids.map((id) => [id, ["inbox"]])),
    }));
    mocks.archiveMessage.mockResolvedValue("archived");
    mocks.deleteMessage.mockResolvedValue(undefined);
    mocks.moveToFolder.mockResolvedValue(undefined);
    mocks.batchArchive.mockImplementation(async (ids: string[]) => ids.length);
    mocks.moveMailMessages.mockImplementation(async (_a: string, ids: string[]) => ({ moved_ids: ids, failures: [] }));
  });

  it("archives with an undo action that moves the message back to its folder", async () => {
    seedList(client, [message("m1"), message("m2")]);

    const result = await runMailAction(client, "archive", [message("m1")]);

    expect(result.ok).toBe(true);
    expect(mocks.archiveMessage).toHaveBeenCalledWith("m1");
    expect(listIds(client)).toEqual(["m2"]);
    const toast = lastToast();
    expect(toast.message).toBe("Message archived");
    expect(toast.action?.label).toBe("Undo");

    toast.action!.onClick();
    await vi.waitFor(() => expect(mocks.moveToFolder).toHaveBeenCalledWith("m1", "inbox"));
    await vi.waitFor(() => expect(lastToast().message).toBe("Undone"));
  });

  it("ignores a second archive of the same message while the first is in flight", async () => {
    seedList(client, [message("m1"), message("m2")]);

    const first = runMailAction(client, "archive", [message("m1")]);
    const second = runMailAction(client, "archive", [message("m1")]);
    const [a, b] = await Promise.all([first, second]);

    expect(a.ok).toBe(true);
    expect(b.ok).toBe(false);
    expect(mocks.archiveMessage).toHaveBeenCalledTimes(1);
  });

  it("keeps one undo per archive so undoing the toasts restores both messages", async () => {
    seedList(client, [message("m1"), message("m2")]);

    await runMailAction(client, "archive", [message("m1")]);
    await runMailAction(client, "archive", [message("m2")]);

    const undoable = useToastStore.getState().toasts.filter((t) => t.action);
    expect(undoable).toHaveLength(2);
    undoable.forEach((t) => t.action!.onClick());
    await vi.waitFor(() => expect(mocks.moveToFolder).toHaveBeenCalledWith("m1", "inbox"));
    await vi.waitFor(() => expect(mocks.moveToFolder).toHaveBeenCalledWith("m2", "inbox"));
  });

  it("moves to trash without confirmation and undoes back to the source folder", async () => {
    const confirm = vi.fn();
    seedList(client, [message("m1")]);

    await runMailAction(client, "trash", [message("m1")], { confirm });

    expect(confirm).not.toHaveBeenCalled();
    expect(mocks.deleteMessage).toHaveBeenCalledWith("m1");
    const toast = lastToast();
    expect(toast.message).toBe("Moved to Trash");
    toast.action!.onClick();
    await vi.waitFor(() => expect(mocks.moveToFolder).toHaveBeenCalledWith("m1", "inbox"));
  });

  it("asks before permanently deleting from Trash and offers no undo", async () => {
    mocks.getMailMoveContext.mockResolvedValue({ provider: "imap", folder_ids_by_message: { m1: ["trash"] } });
    useMailStore.setState({ activeFolderId: "trash" });
    const confirm = vi.fn().mockResolvedValue(true);

    await runMailAction(client, "trash", [message("m1")], { confirm });

    expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ permanent: true, undoable: false }));
    expect(mocks.deleteMessage).toHaveBeenCalledWith("m1");
    expect(lastToast().action).toBeUndefined();
  });

  it("does nothing when a permanent delete is cancelled", async () => {
    const confirm = vi.fn().mockResolvedValue(false);
    const draft = message("d1", { is_draft: true, remote_id: "" });
    seedList(client, [draft]);

    const result = await runMailAction(client, "trash", [draft], { confirm });

    expect(result.ok).toBe(false);
    expect(mocks.deleteMessage).not.toHaveBeenCalled();
    expect(listIds(client)).toEqual(["d1"]);
  });

  it("moves a batch to Trash with move_mail_messages and undoes the whole batch", async () => {
    const messages = [message("m1"), message("m2")];
    seedList(client, messages);

    await runMailAction(client, "trash", messages, { confirm: vi.fn() });

    expect(mocks.moveMailMessages).toHaveBeenCalledWith("acc", ["m1", "m2"], "inbox", "trash");
    expect(mocks.batchDelete).not.toHaveBeenCalled();
    expect(lastToast().message).toBe("2 messages moved to Trash");

    lastToast().action!.onClick();
    await vi.waitFor(() =>
      expect(mocks.moveMailMessages).toHaveBeenLastCalledWith("acc", ["m1", "m2"], "trash", "inbox"));
  });

  it("undoes a batch archive per source folder", async () => {
    const messages = [message("m1"), message("m2")];
    await runMailAction(client, "archive", messages);

    expect(mocks.batchArchive).toHaveBeenCalledWith(["m1", "m2"]);
    lastToast().action!.onClick();
    await vi.waitFor(() =>
      expect(mocks.moveMailMessages).toHaveBeenCalledWith("acc", ["m1", "m2"], "archive", "inbox"));
  });

  it("after a partly failed batch archive, undoes only the archived messages and warns", async () => {
    let archived = false;
    mocks.getMailMoveContext.mockImplementation(async (_account: string, ids: string[]) => ({
      provider: "imap",
      folder_ids_by_message: Object.fromEntries(ids.map((id) => [id, [archived && id === "m1" ? "archive" : "inbox"]])),
    }));
    mocks.batchArchive.mockImplementation(async () => { archived = true; return 1; });

    await runMailAction(client, "archive", [message("m1"), message("m2")]);

    const toasts = useToastStore.getState().toasts;
    expect(toasts.some((t) => t.type === "warning" && t.message.includes("could not be archived"))).toBe(true);
    toasts.find((t) => t.action)!.action!.onClick();
    await vi.waitFor(() => expect(mocks.moveToFolder).toHaveBeenCalledWith("m1", "inbox"));
    expect(mocks.moveToFolder).not.toHaveBeenCalledWith("m2", expect.anything());
    expect(mocks.moveMailMessages).not.toHaveBeenCalled();
  });

  it("treats a batch archive where nothing succeeded as a failure", async () => {
    seedList(client, [message("m1"), message("m2")]);
    mocks.batchArchive.mockResolvedValueOnce(0);

    const result = await runMailAction(client, "archive", [message("m1"), message("m2")]);

    expect(result.ok).toBe(false);
    expect(listIds(client)).toEqual(["m1", "m2"]);
  });

  it("says which part of a mixed draft + message delete was permanent", async () => {
    mocks.batchDelete.mockResolvedValue(2);
    const draft = message("d1", { is_draft: true, remote_id: "" });

    await runMailAction(client, "trash", [draft, message("m1")], { confirm: vi.fn().mockResolvedValue(true) });

    expect(lastToast().message).toContain("local drafts permanently, the rest moved to Trash");
  });

  it("batch-deletes from Gmail Sent with batch_delete instead of a managed move", async () => {
    mocks.listFolders.mockResolvedValue([...FOLDERS, folder("sent", "sent")]);
    mocks.getMailMoveContext.mockImplementation(async (_account: string, ids: string[]) => ({
      provider: "gmail",
      folder_ids_by_message: Object.fromEntries(ids.map((id) => [id, ["sent"]])),
    }));
    useMailStore.setState({ activeFolderId: "sent" });
    mocks.batchDelete.mockResolvedValue(2);
    const confirm = vi.fn().mockResolvedValue(true);

    const result = await runMailAction(client, "trash", [message("m1"), message("m2")], { confirm });

    expect(result.ok).toBe(true);
    expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ undoable: false }));
    expect(mocks.batchDelete).toHaveBeenCalledWith(["m1", "m2"]);
    expect(mocks.moveMailMessages).not.toHaveBeenCalled();
  });

  it("still batch-moves Gmail Inbox mail to Trash with undo", async () => {
    mocks.getMailMoveContext.mockImplementation(async (_account: string, ids: string[]) => ({
      provider: "gmail",
      folder_ids_by_message: Object.fromEntries(ids.map((id) => [id, ["inbox"]])),
    }));

    await runMailAction(client, "trash", [message("m1"), message("m2")], { confirm: vi.fn() });

    expect(mocks.moveMailMessages).toHaveBeenCalledWith("acc", ["m1", "m2"], "inbox", "trash");
    expect(mocks.batchDelete).not.toHaveBeenCalled();
  });

  it("treats a batch delete where nothing was deleted as a failure", async () => {
    const draft = message("d1", { is_draft: true, remote_id: "" });
    seedList(client, [draft, message("m1")]);
    mocks.batchDelete.mockResolvedValue(0);

    const result = await runMailAction(client, "trash", [draft, message("m1")], { confirm: vi.fn().mockResolvedValue(true) });

    expect(result.ok).toBe(false);
    expect(listIds(client)).toEqual(["d1", "m1"]);
    expect(lastToast().type).toBe("error");
  });

  it("reports the messages a partial batch delete left behind", async () => {
    const draft = message("d1", { is_draft: true, remote_id: "" });
    mocks.batchDelete.mockResolvedValue(1);
    // After the call m1 is still in the inbox; the local draft is gone.
    mocks.getMailMoveContext.mockImplementation(async (_account: string, ids: string[]) => ({
      provider: "imap",
      folder_ids_by_message: Object.fromEntries(ids.map((id) => [id, ["inbox"]])),
    }));

    const result = await runMailAction(client, "trash", [draft, message("m1")], { confirm: vi.fn().mockResolvedValue(true) });

    expect(result.ok).toBe(true);
    expect(result.failedIds).toContain("m1");
    expect(useToastStore.getState().toasts.some((t) => t.type === "warning")).toBe(true);
  });

  it("a failed action doesn't bring back messages another action removed meanwhile", async () => {
    seedList(client, [message("m1"), message("m2"), message("m3")]);
    let failB!: (error: unknown) => void;
    mocks.archiveMessage.mockImplementation((id: string) =>
      id === "m2" ? new Promise((_resolve, reject) => { failB = reject; }) : Promise.resolve("archived"));

    const b = runMailAction(client, "archive", [message("m2")]);
    await vi.waitFor(() => expect(mocks.archiveMessage).toHaveBeenCalledWith("m2"));
    await runMailAction(client, "archive", [message("m1")]);
    expect(listIds(client)).toEqual(["m3"]);

    failB({ kind: "Network", message: "offline" });
    await b;

    expect(listIds(client)).toEqual(["m2", "m3"]);
  });

  it("shows Undoing… until a Move To undo finishes", async () => {
    let finishUndo!: () => void;
    mocks.moveMailMessages
      .mockResolvedValueOnce({ moved_ids: ["m1"], failures: [] })
      .mockImplementationOnce(() => new Promise((resolve) => { finishUndo = () => resolve({ moved_ids: ["m1"], failures: [] }); }));

    await moveMessagesWithUndo(client, { accountId: "acc", messageIds: ["m1"], sourceFolderId: "inbox", targetFolderId: "projects", targetFolderName: "Project" });
    lastToast().action!.onClick();

    await vi.waitFor(() => expect(mocks.moveMailMessages).toHaveBeenLastCalledWith("acc", ["m1"], "projects", "inbox"));
    expect(useToastStore.getState().toasts.some((t) => t.message === "Undoing…" && !t.exiting)).toBe(true);

    finishUndo();
    await vi.waitFor(() => expect(lastToast().message).toBe("Undone"));
    expect(useToastStore.getState().toasts.some((t) => t.message === "Undoing…" && !t.exiting)).toBe(false);
  });

  it("keeps Undo for the messages a partly failed spam batch did move", async () => {
    seedList(client, [message("m1"), message("m2"), message("m3")]);
    mocks.moveToFolder.mockImplementation(async (id: string) => {
      if (id === "m2") throw { kind: "Network", message: "offline" };
    });

    const result = await runMailAction(client, "spam", [message("m1"), message("m2"), message("m3")], { spamFolderId: "spam" });

    expect(result).toMatchObject({ ok: true, count: 2, failedIds: ["m2"] });
    expect(listIds(client)).toEqual(["m2"]);
    const toasts = useToastStore.getState().toasts;
    expect(toasts.some((t) => t.type === "warning")).toBe(true);
    expect(toasts.find((t) => t.action)).toBeTruthy();
  });

  it("keeps Undo for a batch archive that includes an already archived message", async () => {
    mocks.getMailMoveContext.mockImplementation(async (_account: string, ids: string[]) => ({
      provider: "imap",
      folder_ids_by_message: Object.fromEntries(ids.map((id) => [id, [id === "m3" ? "archive" : "inbox"]])),
    }));

    await runMailAction(client, "archive", [message("m1"), message("m2"), message("m3")]);

    lastToast().action!.onClick();
    await vi.waitFor(() =>
      expect(mocks.moveMailMessages).toHaveBeenCalledWith("acc", ["m1", "m2"], "archive", "inbox"));
  });

  it("reports messages busy with another action instead of dropping them", async () => {
    let finishFirst!: () => void;
    mocks.archiveMessage.mockImplementationOnce(() => new Promise((resolve) => { finishFirst = () => resolve("archived"); }));
    const first = runMailAction(client, "archive", [message("m1")]);
    await vi.waitFor(() => expect(mocks.archiveMessage).toHaveBeenCalled());

    mocks.batchDelete.mockResolvedValue(1);
    const result = await runMailAction(client, "trash", [message("m1"), message("m2")], { confirm: vi.fn().mockResolvedValue(true) });

    expect(result.ok).toBe(true);
    expect(result.failedIds).toContain("m1");
    finishFirst();
    await first;
  });

  it("offers no Undo when a message had several labels (Undo would restore only one)", async () => {
    mocks.listFolders.mockResolvedValue([...FOLDERS, { ...folder("work", null), is_system: false }]);
    mocks.getMailMoveContext.mockImplementation(async (_account: string, ids: string[]) => ({
      provider: "gmail",
      folder_ids_by_message: Object.fromEntries(ids.map((id) => [id, ["inbox", "work"]])),
    }));

    await runMailAction(client, "archive", [message("m1")]);

    expect(mocks.archiveMessage).toHaveBeenCalledWith("m1");
    expect(lastToast().action).toBeUndefined();
  });

  it("marks spam with undo back to the original folder", async () => {
    await runMailAction(client, "spam", [message("m1")], { spamFolderId: "spam" });

    expect(mocks.moveToFolder).toHaveBeenCalledWith("m1", "spam");
    lastToast().action!.onClick();
    await vi.waitFor(() => expect(mocks.moveToFolder).toHaveBeenLastCalledWith("m1", "inbox"));
  });

  it("restores the list and shows an error toast when the action fails", async () => {
    seedList(client, [message("m1")]);
    mocks.archiveMessage.mockRejectedValueOnce({ kind: "Network", message: "offline" });
    const onFailed = vi.fn();

    const plan = await planMailAction(client, "archive", [message("m1")]);
    const result = await executeMailAction(client, plan, { onFailed });

    expect(result.ok).toBe(false);
    expect(listIds(client)).toEqual(["m1"]);
    expect(onFailed).toHaveBeenCalledWith(["m1"]);
    expect(lastToast().type).toBe("error");
  });

  it("offers no undo when the account has no archive folder", async () => {
    mocks.listFolders.mockResolvedValue([folder("inbox", "inbox")]);

    await runMailAction(client, "archive", [message("m1")]);

    expect(lastToast().action).toBeUndefined();
  });
  it("moves to a folder and undoes back to the chosen source folder", async () => {
    const result = await moveMessagesWithUndo(client, {
      accountId: "acc",
      messageIds: ["m1", "m2"],
      sourceFolderId: "inbox",
      targetFolderId: "projects",
      targetFolderName: "Projects",
    });

    expect(result.moved_ids).toEqual(["m1", "m2"]);
    expect(mocks.moveMailMessages).toHaveBeenCalledWith("acc", ["m1", "m2"], "inbox", "projects");
    const toast = lastToast();
    expect(toast.type).toBe("success");
    expect(toast.message).toMatch(/^Moved to/);
    expect(toast.action?.label).toBe("Undo");

    toast.action!.onClick();
    await vi.waitFor(() =>
      expect(mocks.moveMailMessages).toHaveBeenLastCalledWith("acc", ["m1", "m2"], "projects", "inbox"));
    await vi.waitFor(() => expect(lastToast().message).toBe("Undone"));
    // A single-folder undo of "Move to" never falls back to move_to_folder.
    expect(mocks.moveToFolder).not.toHaveBeenCalled();
  });

  it("undoes only the moved messages, each back to its own folder", async () => {
    mocks.moveMailMessages.mockResolvedValueOnce({
      moved_ids: ["m1", "m2"],
      failures: [{ message_id: "m3", error: "busy" }],
    });

    await moveMessagesWithUndo(client, {
      accountId: "acc",
      messageIds: ["m1", "m2", "m3"],
      sourceFolderId: null,
      targetFolderId: "projects",
      sourceByMessage: { m1: "inbox", m2: "archive", m3: "inbox" },
    });

    lastToast().action!.onClick();
    await vi.waitFor(() => expect(mocks.moveMailMessages).toHaveBeenCalledTimes(3));
    expect(mocks.moveMailMessages).toHaveBeenCalledWith("acc", ["m1"], "projects", "inbox");
    expect(mocks.moveMailMessages).toHaveBeenCalledWith("acc", ["m2"], "projects", "archive");
  });

  it("offers no undo when nothing was moved", async () => {
    mocks.moveMailMessages.mockResolvedValueOnce({ moved_ids: [], failures: [{ message_id: "m1", error: "x" }] });

    await moveMessagesWithUndo(client, {
      accountId: "acc", messageIds: ["m1"], sourceFolderId: "inbox", targetFolderId: "projects",
    });

    expect(useToastStore.getState().toasts).toHaveLength(0);
  });
});
