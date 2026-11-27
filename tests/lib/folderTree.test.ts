import { describe, expect, it } from "vitest";
import type { Folder } from "../../src/lib/api";
import { buildFolderTree, GMAIL_CONTAINER_KEY, totalForNode, type FolderTreeNode } from "../../src/lib/folderTree";

let order = 0;
function folder(id: string, name: string, extra: Partial<Folder> = {}): Folder {
  return {
    id,
    account_id: "acc",
    remote_id: name,
    name,
    folder_type: "folder",
    role: null,
    parent_id: null,
    color: null,
    is_system: false,
    sort_order: order++,
    ...extra,
  } as Folder;
}

/** Compact "label(children)" view of a tree for readable assertions */
function shape(nodes: FolderTreeNode[]): string[] {
  return nodes.map((n) => (n.children.length ? `${n.label}(${shape(n.children).join(",")})` : n.label));
}

const gmailAccount = [
  folder("inbox", "INBOX", { role: "inbox", is_system: true }),
  folder("sent", "[Gmail]/Sent Mail", { role: "sent", parent_id: "[Gmail]", is_system: true }),
  folder("trash", "[Gmail]/Trash", { role: "trash", parent_id: "[Gmail]", is_system: true }),
  folder("clients", "Clients"),
  folder("amy", "Clients/Amy & Todd", { parent_id: "clients", remote_id: "Clients/Amy &- Todd" }),
  folder("mara", "Clients/Mara Ream", { parent_id: "clients" }),
  folder("colleen", "Clients/Mara Ream/Colleen Erickson", { parent_id: "mara" }),
  folder("coach", "Clients/Mara Ream/Coach Rusty Burnside", { parent_id: "mara" }),
  folder("notes", "Notes"),
  folder("all", "[Gmail]/All Mail", { parent_id: "[Gmail]", is_system: true }),
  folder("important", "[Gmail]/Important", { parent_id: "[Gmail]" }),
  folder("trash-services", "[Gmail]/Trash/Services", { parent_id: "trash" }),
];

describe("buildFolderTree", () => {
  it("nests folders by parent and labels them with their own name only", () => {
    expect(shape(buildFolderTree(gmailAccount))).toEqual([
      "INBOX",
      "Sent Mail",
      "Trash(Services)",
      "Clients(Amy & Todd,Mara Ream(Coach Rusty Burnside,Colleen Erickson))",
      "Notes",
      "Gmail(All Mail,Important)",
    ]);
  });

  it("gathers [Gmail] extras under one virtual node that has no folder", () => {
    const gmail = buildFolderTree(gmailAccount).find((n) => n.key === GMAIL_CONTAINER_KEY)!;
    expect(gmail.folder).toBeNull();
    expect(gmail.children.map((c) => c.folder?.id)).toEqual(["all", "important"]);
  });

  it("supports the [Google Mail] container used in some regions", () => {
    const tree = buildFolderTree([folder("x", "[Google Mail]/Starred", { parent_id: "[Google Mail]" })]);
    expect(shape(tree)).toEqual(["Gmail(Starred)"]);
  });

  it("keeps system folders at the top level even when they have a parent", () => {
    const tree = buildFolderTree([
      folder("parent", "Mail"),
      folder("sent", "Mail/Sent", { parent_id: "parent", role: "sent" }),
    ]);
    expect(shape(tree)).toEqual(["Mail/Sent", "Mail"]);
  });

  it("resolves parents given by remote id and supports '.' delimiters", () => {
    const tree = buildFolderTree([
      folder("p", "INBOX.Projects"),
      folder("c", "INBOX.Projects.Acme", { parent_id: "INBOX.Projects" }),
    ]);
    expect(shape(tree)).toEqual(["INBOX.Projects(Acme)"]);
  });

  it("nests Outlook and local folders, whose names have no path prefix", () => {
    const tree = buildFolderTree([folder("p", "Work"), folder("c", "Projects", { parent_id: "p" })]);
    expect(shape(tree)).toEqual(["Work(Projects)"]);
  });

  it("keeps INBOX.-namespace folders at the top level instead of under Inbox", () => {
    const tree = buildFolderTree([
      folder("inbox", "INBOX", { role: "inbox" }),
      folder("work", "INBOX.Work", { parent_id: "INBOX" }),
      folder("q1", "INBOX.Work.Q1", { parent_id: "INBOX.Work" }),
    ]);
    expect(shape(tree)).toEqual(["INBOX", "Work(Q1)"]);
  });

  it("nests other children of Inbox under it (Outlook, opaque remote ids)", () => {
    const tree = buildFolderTree([
      folder("inbox", "Inbox", { role: "inbox", remote_id: "AAMk-inbox" }),
      folder("receipts", "Receipts", { parent_id: "inbox", remote_id: "AAMk-receipts" }),
    ]);
    expect(shape(tree)).toEqual(["Inbox(Receipts)"]);
  });

  it("strips backslash-delimited paths too", () => {
    const tree = buildFolderTree([folder("c", "Clients"), folder("m", "Clients\\Mara", { parent_id: "c" })]);
    expect(shape(tree)).toEqual(["Clients(Mara)"]);
  });

  it("nests sub-folders of other system folders under them", () => {
    const tree = buildFolderTree([folder("trash", "Trash", { role: "trash" }), folder("old", "Trash/Old", { parent_id: "trash" })]);
    expect(shape(tree)).toEqual(["Trash(Old)"]);
  });

  it("does not lose folders caught in a parent cycle", () => {
    const tree = buildFolderTree([
      folder("a", "A/B", { parent_id: "b" }),
      folder("b", "A/B/C", { parent_id: "a" }),
    ]);
    const ids: string[] = [];
    const walk = (nodes: FolderTreeNode[]) => nodes.forEach((n) => { ids.push(n.key); walk(n.children); });
    walk(tree);
    expect(ids.sort()).toEqual(["a", "b"]);
  });

  it("sorts custom folders naturally regardless of case", () => {
    const tree = buildFolderTree([folder("b", "beta"), folder("a2", "Item 10"), folder("a1", "Item 9"), folder("c", "Alpha")]);
    expect(shape(tree)).toEqual(["Alpha", "beta", "Item 9", "Item 10"]);
  });
});

describe("totalForNode", () => {
  it("adds a node's own count to all of its descendants", () => {
    const clients = buildFolderTree(gmailAccount).find((n) => n.key === "clients")!;
    const counts: Record<string, number> = { clients: 1, amy: 2, colleen: 4 };
    expect(totalForNode(clients, (f) => counts[f.id] ?? 0)).toBe(7);
  });
});
