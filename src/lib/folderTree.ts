import type { Folder } from "@/lib/api";

/**
 * Turns a flat, sidebar-sorted folder list into a tree for display.
 *
 * Folders nest by `parent_id`. IMAP servers return full paths as folder names
 * ("Clients/Mara Ream"), so a child is labelled with only the part after its
 * parent's path; Outlook and local folders already have their own names.
 * IMAP folders in the "INBOX." / "INBOX/" namespace (Dovecot, Courier) stay at
 * the top level, since nesting would hide them all under Inbox; other children
 * of Inbox (Outlook "Inbox › Receipts") nest under it as usual. Gmail's
 * "[Gmail]" / "[Google Mail]" container cannot be opened, so its non-system
 * children (All Mail, Important, Starred…) are gathered under a virtual
 * "Gmail" node instead of being shown with the bracketed prefix.
 */
export interface FolderTreeNode {
  /** Stable key: the folder id, or `virtual:<path>` for the Gmail container */
  key: string;
  /** null for the virtual Gmail container, which cannot be opened */
  folder: Folder | null;
  label: string;
  children: FolderTreeNode[];
}

const GMAIL_CONTAINERS = ["[Gmail]", "[Google Mail]"];
const PATH_DELIMITERS = ["/", ".", "\\"];

export const GMAIL_CONTAINER_KEY = "virtual:[Gmail]";

function gmailContainerOf(folder: Folder): string | null {
  return GMAIL_CONTAINERS.find((c) => folder.name.startsWith(`${c}/`)) ?? null;
}

/** The part of `name` after `parentName` and a path delimiter, if it is one */
function childSegment(name: string, parentName: string): string | null {
  if (name.length <= parentName.length + 1 || !name.startsWith(parentName)) return null;
  return PATH_DELIMITERS.includes(name[parentName.length]) ? name.slice(parentName.length + 1) : null;
}

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

function byLabel(a: FolderTreeNode, b: FolderTreeNode): number {
  return collator.compare(a.label, b.label);
}

export function buildFolderTree(folders: Folder[]): FolderTreeNode[] {
  const byId = new Map(folders.map((f) => [f.id, f]));
  const byRemoteId = new Map(folders.map((f) => [f.remote_id, f]));
  const nodes = new Map<string, FolderTreeNode>();
  for (const folder of folders) {
    nodes.set(folder.id, { key: folder.id, folder, label: folder.name, children: [] });
  }

  const roots: FolderTreeNode[] = [];
  let gmailNode: FolderTreeNode | null = null;

  for (const folder of folders) {
    const node = nodes.get(folder.id)!;
    const parent = folder.parent_id
      ? (byId.get(folder.parent_id) ?? byRemoteId.get(folder.parent_id))
      : undefined;
    const validParent = parent && parent.id !== folder.id && parent.account_id === folder.account_id
      ? parent
      : undefined;
    const segment = validParent ? childSegment(folder.name, validParent.name) : null;

    // IMAP remote ids are server paths ("INBOX.Work"); Outlook and Gmail API ids aren't
    const inboxNamespace = validParent?.role === "inbox"
      && childSegment(folder.remote_id, validParent.remote_id) !== null;
    // System folders (Inbox, Sent…) always stay at the top level
    if (!folder.role && validParent && !inboxNamespace) {
      node.label = segment ?? folder.name;
      nodes.get(validParent.id)!.children.push(node);
      continue;
    }
    if (!folder.role && segment) node.label = segment;

    const container = gmailContainerOf(folder);
    if (container) {
      node.label = folder.name.slice(container.length + 1);
      if (!folder.role) {
        gmailNode ??= { key: GMAIL_CONTAINER_KEY, folder: null, label: "Gmail", children: [] };
        gmailNode.children.push(node);
        continue;
      }
    }
    roots.push(node);
  }

  // A parent cycle would leave folders unreachable from any root; surface them
  const reachable = new Set<string>();
  const visit = (node: FolderTreeNode) => {
    if (reachable.has(node.key)) return;
    reachable.add(node.key);
    node.children.forEach(visit);
  };
  roots.forEach(visit);
  if (gmailNode) visit(gmailNode);
  for (const folder of folders) {
    if (!reachable.has(folder.id)) {
      const node = nodes.get(folder.id)!;
      node.children = node.children.filter((child) => !reachable.has(child.key));
      node.label = folder.name;
      // Break the cycle: the surfaced folder is a root now, nobody's child
      for (const other of nodes.values()) other.children = other.children.filter((child) => child !== node);
      roots.push(node);
      visit(node);
    }
  }

  const sortChildren = (node: FolderTreeNode) => {
    node.children.sort(byLabel);
    node.children.forEach(sortChildren);
  };
  roots.forEach(sortChildren);
  if (gmailNode) {
    sortChildren(gmailNode);
    roots.push(gmailNode);
  }

  // Keep the incoming order for system folders, then sort custom folders by name
  const system = roots.filter((n) => n.folder?.role);
  const custom = roots.filter((n) => n.folder && !n.folder.role).sort(byLabel);
  const virtual = roots.filter((n) => !n.folder);
  return [...system, ...custom, ...virtual];
}

/** Sum a per-folder count over a node and all its descendants */
export function totalForNode(node: FolderTreeNode, countFor: (folder: Folder) => number): number {
  const own = node.folder ? countFor(node.folder) : 0;
  return node.children.reduce((sum, child) => sum + totalForNode(child, countFor), own);
}
