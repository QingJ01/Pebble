/**
 * Vim-style keyboard layer.
 *
 * These sit on top of the user's rebindable shortcuts: a user binding for the
 * same key always wins. Multi-key sequences are written as space-separated,
 * lower-cased key strings in the format produced by `eventToKeyString`
 * (e.g. "d d", "g i", "shift+g").
 */
export const VIM_SEQUENCE_TIMEOUT_MS = 1000;

/** How many messages Ctrl+d / Ctrl+u move the selection by */
export const VIM_PAGE_STEP = 10;

export const VIM_BINDINGS: Readonly<Record<string, string>> = {
  "d d": "delete-message",
  "g g": "first-message",
  "shift+g": "last-message",
  "l": "open-message",
  "h": "close-message",
  "g i": "go-inbox",
  "g s": "go-sent",
  "g d": "go-drafts",
  "g a": "go-archive",
  "g t": "go-trash",
  "ctrl+d": "page-down",
  "ctrl+u": "page-up",
  "x": "toggle-select",
  "u": "undo",
  "shift+?": "show-shortcuts",
};

/** First keys of multi-key sequences ("d", "g"), which wait for a second key */
export const VIM_PREFIXES: ReadonlySet<string> = new Set(
  Object.keys(VIM_BINDINGS)
    .filter((keys) => keys.includes(" "))
    .map((keys) => keys.split(" ")[0]),
);

/** Rows shown in Settings → Shortcuts, in display order */
export const VIM_HELP: ReadonlyArray<{ keys: string; labelKey: string; fallback: string }> = [
  { keys: "j / k", labelKey: "shortcuts.vim.nextPrev", fallback: "Next / previous message" },
  { keys: "g g / G", labelKey: "shortcuts.vim.firstLast", fallback: "First / last message" },
  { keys: "Ctrl+d / Ctrl+u", labelKey: "shortcuts.vim.page", fallback: "Move down / up 10 messages" },
  { keys: "l", labelKey: "shortcuts.vim.open", fallback: "Open message" },
  { keys: "h", labelKey: "shortcuts.vim.close", fallback: "Close message" },
  { keys: "d d", labelKey: "shortcuts.vim.delete", fallback: "Move message to trash" },
  { keys: "x", labelKey: "shortcuts.vim.select", fallback: "Select message (d d, Delete and e then act on the selection)" },
  { keys: "u", labelKey: "shortcuts.vim.undo", fallback: "Undo the last delete or archive" },
  { keys: "?", labelKey: "shortcuts.vim.help", fallback: "Show keyboard shortcuts" },
  { keys: "g i", labelKey: "shortcuts.vim.goInbox", fallback: "Go to Inbox" },
  { keys: "g s", labelKey: "shortcuts.vim.goSent", fallback: "Go to Sent" },
  { keys: "g d", labelKey: "shortcuts.vim.goDrafts", fallback: "Go to Drafts" },
  { keys: "g a", labelKey: "shortcuts.vim.goArchive", fallback: "Go to Archive" },
  { keys: "g t", labelKey: "shortcuts.vim.goTrash", fallback: "Go to Trash" },
];
