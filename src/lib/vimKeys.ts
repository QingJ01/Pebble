/**
 * Vim-style keyboard layer.
 *
 * These sit on top of the user's rebindable shortcuts: a user binding for the
 * same key always wins. Multi-key sequences are written as space-separated,
 * keys. Printable keys are written as the character typed (`KeyboardEvent.key`:
 * "G", "?"), so Caps Lock and AltGr layouts work; others use the lower-cased
 * `eventToKeyString` format ("ctrl+d").
 */
export const VIM_SEQUENCE_TIMEOUT_MS = 1000;

/**
 * Event the mail list's batch toolbar listens for: run its Archive / Delete on
 * the bulk selection, with the toolbar's own busy guard and confirmation.
 */
export const BATCH_MOVE_EVENT = "pebble:batch-move";

/** How many messages Ctrl+d / Ctrl+u move the selection by */
export const VIM_PAGE_STEP = 10;

export const VIM_BINDINGS: Readonly<Record<string, string>> = {
  "d d": "delete-message",
  "g g": "first-message",
  "G": "last-message",
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
  "?": "show-shortcuts",
};

/** First keys of multi-key sequences ("d", "g"), which wait for a second key */
export const VIM_PREFIXES: ReadonlySet<string> = new Set(
  Object.keys(VIM_BINDINGS)
    .filter((keys) => keys.includes(" "))
    .map((keys) => keys.split(" ")[0]),
);

/**
 * Bindings that stop a vim key from working: a user shortcut on the first key
 * of a vim entry wins over it. Lower-cased `eventToKeyString` format.
 */
export const VIM_FIRST_KEYS_AS_BINDINGS: ReadonlySet<string> = new Set(
  Object.keys(VIM_BINDINGS).map((keys) => {
    const first = keys.split(" ")[0];
    if (first.length !== 1 || /[a-z]/.test(first)) return first;
    return `shift+${first.toLowerCase()}`;
  }),
);

/**
 * Rows shown in Settings → Shortcuts, in display order. `actions` rows show
 * the user's current bindings; `shortcuts` rows are formatted for the platform.
 */
export const VIM_HELP: ReadonlyArray<{
  keys?: string;
  actions?: string[];
  shortcuts?: string[];
  labelKey: string;
  fallback: string;
}> = [
  { actions: ["next-message", "prev-message"], labelKey: "shortcuts.vim.nextPrev", fallback: "Next / previous message" },
  { keys: "g g / G", labelKey: "shortcuts.vim.firstLast", fallback: "First / last message" },
  { shortcuts: ["Ctrl+D", "Ctrl+U"], labelKey: "shortcuts.vim.page", fallback: "Move down / up 10 messages" },
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
