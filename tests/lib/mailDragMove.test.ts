import { describe, expect, it, vi } from "vitest";

vi.mock("@/hooks/mutations/mailActions", () => ({ moveMessagesWithUndo: vi.fn() }));

import { canDropOnFolder, planDragMove, type DraggedMessage } from "@/lib/mailDragMove";
import type { Folder } from "@/lib/api";
import type { MailMoveContext } from "@/lib/folders-api";

const folder = (id: string, extra: Partial<Folder> = {}): Folder => ({
  id, account_id: "a1", remote_id: id, name: id, folder_type: "folder", role: null,
  parent_id: null, color: null, is_system: false, sort_order: 0, ...extra,
});
const folders = [
  folder("inbox", { role: "inbox", is_system: true }),
  folder("drafts", { role: "drafts", is_system: true }),
  folder("work"),
  folder("local", { remote_id: "__local_folder_1" }),
  folder("other-inbox", { account_id: "a2", role: "inbox", is_system: true }),
];
const msg = (id: string, extra: Partial<DraggedMessage> = {}): DraggedMessage =>
  ({ id, account_id: "a1", is_draft: false, ...extra });
const ctx = (memberships: Record<string, string[]>, provider: MailMoveContext["provider"] = "imap"): MailMoveContext =>
  ({ provider, folder_ids_by_message: memberships });

describe("planDragMove", () => {
  it("moves from the viewed folder into a folder of the same account", () => {
    const plan = planDragMove([msg("m1"), msg("m2")], "work", folders, ctx({ m1: ["inbox"], m2: ["inbox"] }), "inbox");
    expect(plan).toMatchObject({ ok: true, accountId: "a1", sourceFolderId: "inbox" });
    expect(plan.ok && plan.target.id).toBe("work");
  });

  it("resolves an all-accounts folder to the messages' account", () => {
    const plan = planDragMove([msg("m1")], "all:inbox", folders, ctx({ m1: ["work"] }), "work");
    expect(plan.ok && plan.target.id).toBe("inbox");
  });

  it("refuses other accounts, drafts, current folders and protected targets", () => {
    const m = ctx({ m1: ["inbox"] });
    expect(planDragMove([msg("m1")], "other-inbox", folders, m, "inbox")).toMatchObject({ ok: false, reasonKey: "folderManagement.dragOtherAccount" });
    expect(planDragMove([msg("m1", { is_draft: true })], "work", folders, m, "inbox")).toMatchObject({ ok: false, reasonKey: "folderManagement.draftsProtected" });
    expect(planDragMove([msg("m1")], "inbox", folders, m, "inbox")).toMatchObject({ ok: false, reasonKey: "folderManagement.dragAlreadyThere" });
    expect(planDragMove([msg("m1")], "drafts", folders, m, "inbox")).toMatchObject({ ok: false, reasonKey: "folderManagement.dragNotDestination" });
    expect(planDragMove([msg("m1")], "local", folders, m, "inbox")).toMatchObject({ ok: false, reasonKey: "folderManagement.dragLocalBoundary" });
  });

  it("needs the viewed folder as source for multi-folder messages", () => {
    const m = ctx({ m1: ["inbox", "work"] }, "gmail");
    expect(planDragMove([msg("m1")], "trash-x", [...folders, folder("trash-x", { role: "trash", is_system: true })], m, "inbox"))
      .toMatchObject({ ok: true, sourceFolderId: "inbox" });
    expect(planDragMove([msg("m1")], "all:inbox", folders, ctx({ m1: ["work", "local"] }), "all:starred"))
      .toMatchObject({ ok: false, reasonKey: "folderManagement.dragChooseSource" });
  });
});

describe("canDropOnFolder", () => {
  it("lights up only folders the drop could move into", () => {
    expect(canDropOnFolder([msg("m1")], "work", folders, "imap", "inbox")).toBe(true);
    expect(canDropOnFolder([msg("m1")], "all:inbox", folders, "imap", "work")).toBe(true);
    expect(canDropOnFolder([msg("m1")], "drafts", folders, "imap", "inbox")).toBe(false);
    expect(canDropOnFolder([msg("m1")], "inbox", folders, "imap", "inbox")).toBe(false);
    expect(canDropOnFolder([msg("m1")], "local", folders, "imap", "inbox")).toBe(false);
    expect(canDropOnFolder([msg("m1")], "other-inbox", folders, "imap", "inbox")).toBe(false);
    expect(canDropOnFolder([msg("m1", { is_draft: true })], "work", folders, "imap", "inbox")).toBe(false);
  });
});
