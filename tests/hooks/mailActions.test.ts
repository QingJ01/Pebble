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

import { executeMailAction, planMailAction, runMailAction } from "../../src/hooks/mutations/mailActions";
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
});
