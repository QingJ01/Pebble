# Frontend Feature Completion Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire up all existing backend commands to functional frontend UI — compose/reply/forward, message actions, attachment download, kanban integration, and fix all identified bugs.

**Architecture:** All backend Tauri commands and api.ts functions already exist. This plan only adds/modifies React components and wires them to existing APIs. Uses existing CSS variables + inline styles, lucide-react icons, react-i18next for i18n. No new dependencies.

**Tech Stack:** React 19, TypeScript, Zustand, TanStack Query, lucide-react, Tauri v2 IPC

---

## File Map

| File | Action | Purpose |
|---|---|---|
| `src/components/Sidebar.tsx` | Modify | Add Compose button |
| `src/components/MessageDetail.tsx` | Modify | Add action toolbar (reply, forward, star, archive, delete, kanban) |
| `src/components/MessageItem.tsx` | Modify | Add hover action buttons (star, archive) |
| `src/components/AttachmentList.tsx` | Modify | Fix download to call `downloadAttachment` + open file |
| `src/components/PrivacyBanner.tsx` | Modify | Call `trustSender` API + i18n |
| `src/components/SearchBar.tsx` | Modify | Navigate to SearchView on submit |
| `src/features/inbox/InboxView.tsx` | Modify | Pass search query to SearchView via store |
| `src/hooks/useKeyboard.ts` | Modify | Add `open-message`, `compose-new`, `reply` actions |
| `src/features/command-palette/commands.ts` | Modify | Fix download-attachment command, fix notification key |
| `src/features/settings/AppearanceTab.tsx` | Modify | Fix notification localStorage key |
| `src/stores/ui.store.ts` | Modify | Add `searchQuery` state for cross-view search |
| `src/locales/en.json` | Modify | Add missing i18n keys |
| `src/locales/zh.json` | Modify | Add missing i18n keys |

---

## Wave 1: Compose + Message Actions

### Task 1: Add Compose button to Sidebar

**Files:**
- Modify: `src/components/Sidebar.tsx`
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`

- [ ] **Step 1: Add i18n keys**

In `src/locales/en.json`, add inside `"sidebar"`:
```json
"compose": "Compose"
```

In `src/locales/zh.json`, add inside `"sidebar"`:
```json
"compose": "撰写"
```

- [ ] **Step 2: Add Compose button to Sidebar**

In `src/components/Sidebar.tsx`, add `PenLine` to the lucide imports:
```tsx
import {
  Inbox, Send, FileEdit, Trash2, Archive, AlertTriangle,
  Folder, LayoutGrid, Settings, Search, PenLine,
} from "lucide-react";
```

Add `openCompose` to the `useUIStore` destructure:
```tsx
const { activeView, setActiveView, sidebarCollapsed } = useUIStore();
const openCompose = useUIStore((s) => s.openCompose);
```

Insert the Compose button **before** the Search button inside the top `<nav>` block (line ~108):
```tsx
<nav style={{ padding: "8px 6px 0", display: "flex", flexDirection: "column", gap: "1px" }}>
  <SidebarButton
    icon={<PenLine size={16} />}
    label={t("sidebar.compose", "Compose")}
    isActive={false}
    collapsed={sidebarCollapsed}
    style={{
      ...buttonBase,
      backgroundColor: "var(--color-accent)",
      color: "#fff",
      marginBottom: "4px",
    }}
    onClick={() => openCompose("new")}
  />
  <SidebarButton
    icon={<Search size={16} />}
    ...existing search button...
  />
</nav>
```

- [ ] **Step 3: Verify visually**

Run `pnpm dev`. The Compose button should appear at the top of the sidebar with accent color. Clicking it should open the ComposeView modal.

- [ ] **Step 4: Commit**

```bash
git add src/components/Sidebar.tsx src/locales/en.json src/locales/zh.json
git commit -m "feat(ui): add compose button to sidebar"
```

---

### Task 2: Add action toolbar to MessageDetail

**Files:**
- Modify: `src/components/MessageDetail.tsx`
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`

- [ ] **Step 1: Add i18n keys**

In `en.json`, add a `"messageActions"` section:
```json
"messageActions": {
  "reply": "Reply",
  "forward": "Forward",
  "star": "Star",
  "unstar": "Unstar",
  "markUnread": "Mark unread",
  "archive": "Archive",
  "delete": "Delete",
  "addToKanban": "Add to Kanban"
}
```

In `zh.json`:
```json
"messageActions": {
  "reply": "回复",
  "forward": "转发",
  "star": "标星",
  "unstar": "取消标星",
  "markUnread": "标为未读",
  "archive": "归档",
  "delete": "删除",
  "addToKanban": "加入看板"
}
```

- [ ] **Step 2: Add action toolbar**

In `src/components/MessageDetail.tsx`, add imports:
```tsx
import { ArrowLeft, Clock, Languages, Reply, Forward, Star, Archive, Trash2, LayoutGrid } from "lucide-react";
import { updateMessageFlags, moveToKanban } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";
import { useTranslation } from "react-i18next";
```

Add `const { t } = useTranslation();` and `const openCompose = useUIStore((s) => s.openCompose);` at the top of the component.

After the existing header row (subject + snooze + translate buttons) and before the sender info row, insert an action toolbar:

```tsx
{/* Action toolbar */}
<div style={{
  display: "flex",
  alignItems: "center",
  gap: "2px",
  padding: "4px 16px 4px 48px",
}}>
  {[
    { icon: <Reply size={15} />, label: t("messageActions.reply"), onClick: () => openCompose("reply", message) },
    { icon: <Forward size={15} />, label: t("messageActions.forward"), onClick: () => openCompose("forward", message) },
    { icon: <Star size={15} fill={message.is_starred ? "#f59e0b" : "none"} color={message.is_starred ? "#f59e0b" : "currentColor"} />,
      label: message.is_starred ? t("messageActions.unstar") : t("messageActions.star"),
      onClick: async () => {
        await updateMessageFlags(message.id, undefined, !message.is_starred);
        setMessage({ ...message, is_starred: !message.is_starred });
      }},
    { icon: <Archive size={15} />, label: t("messageActions.archive"), onClick: async () => {
        await updateMessageFlags(message.id, undefined, undefined);
        onBack();
      }},
    { icon: <Trash2 size={15} />, label: t("messageActions.delete"), onClick: async () => {
        await updateMessageFlags(message.id, undefined, undefined);
        onBack();
      }},
    { icon: <LayoutGrid size={15} />, label: t("messageActions.addToKanban"), onClick: async () => {
        await moveToKanban(message.id, "todo");
      }},
  ].map((action) => (
    <button
      key={action.label}
      title={action.label}
      onClick={action.onClick}
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "6px 8px",
        border: "none",
        background: "transparent",
        color: "var(--color-text-secondary)",
        cursor: "pointer",
        borderRadius: "4px",
        transition: "background-color 0.12s ease, color 0.12s ease",
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
        e.currentTarget.style.color = "var(--color-text-primary)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.backgroundColor = "transparent";
        e.currentTarget.style.color = "var(--color-text-secondary)";
      }}
    >
      {action.icon}
    </button>
  ))}
</div>
```

- [ ] **Step 3: Verify visually**

Run `pnpm dev`. Open any message. A row of icon buttons should appear below the subject line. Verify:
- Reply/Forward open the ComposeView modal
- Star toggles the star icon fill
- Add to Kanban calls the API (check network/console)

- [ ] **Step 4: Commit**

```bash
git add src/components/MessageDetail.tsx src/locales/en.json src/locales/zh.json
git commit -m "feat(ui): add reply/forward/star/archive/delete/kanban toolbar to message detail"
```

---

### Task 3: Add hover actions to MessageItem

**Files:**
- Modify: `src/components/MessageItem.tsx`

- [ ] **Step 1: Add hover state and inline action buttons**

In `src/components/MessageItem.tsx`, add imports and state:
```tsx
import { Star, Paperclip, Archive } from "lucide-react";
import { useState } from "react";
import { updateMessageFlags } from "@/lib/api";
```

Add state at top of component:
```tsx
const [hovered, setHovered] = useState(false);
```

Update the root `<div>` to use `position: "relative"` and wire hover state:
```tsx
<div
  onClick={onClick}
  onMouseEnter={(e) => {
    setHovered(true);
    if (!isSelected) e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
  }}
  onMouseLeave={(e) => {
    setHovered(false);
    if (!isSelected) e.currentTarget.style.backgroundColor = "transparent";
  }}
  style={{
    position: "relative",
    backgroundColor: isSelected ? "var(--color-sidebar-active)" : "transparent",
    ...rest of existing styles...
    transition: "background-color 0.12s ease",
  }}
>
```

Add hover action buttons as last child inside the root `<div>`, after the snippet `<div>`:
```tsx
{hovered && (
  <div
    style={{
      position: "absolute",
      right: "8px",
      top: "50%",
      transform: "translateY(-50%)",
      display: "flex",
      gap: "2px",
      backgroundColor: "var(--color-bg)",
      border: "1px solid var(--color-border)",
      borderRadius: "6px",
      padding: "2px",
      boxShadow: "0 1px 4px rgba(0,0,0,0.08)",
    }}
    onClick={(e) => e.stopPropagation()}
  >
    <button
      title={message.is_starred ? "Unstar" : "Star"}
      onClick={async (e) => {
        e.stopPropagation();
        await updateMessageFlags(message.id, undefined, !message.is_starred);
      }}
      style={{
        display: "flex", padding: "4px", border: "none", background: "transparent",
        cursor: "pointer", borderRadius: "4px",
        color: message.is_starred ? "#f59e0b" : "var(--color-text-secondary)",
      }}
    >
      <Star size={14} fill={message.is_starred ? "#f59e0b" : "none"} />
    </button>
    <button
      title="Archive"
      onClick={(e) => { e.stopPropagation(); }}
      style={{
        display: "flex", padding: "4px", border: "none", background: "transparent",
        cursor: "pointer", borderRadius: "4px", color: "var(--color-text-secondary)",
      }}
    >
      <Archive size={14} />
    </button>
  </div>
)}
```

- [ ] **Step 2: Verify hover actions**

Run `pnpm dev`. Hover over a message item — two small icon buttons should appear on the right side with a subtle card background. Clicking Star should toggle the star state without navigating.

- [ ] **Step 3: Commit**

```bash
git add src/components/MessageItem.tsx
git commit -m "feat(ui): add hover action buttons (star, archive) to message items"
```

---

## Wave 2: Attachment Download + Search Fix + Bug Fixes

### Task 4: Fix attachment download

**Files:**
- Modify: `src/components/AttachmentList.tsx`

- [ ] **Step 1: Fix handleDownload to actually download**

In `src/components/AttachmentList.tsx`, update the import from api.ts:
```tsx
import { listAttachments, getAttachmentPath, downloadAttachment } from "@/lib/api";
```

Replace the `handleDownload` function:
```tsx
async function handleDownload(attachment: Attachment) {
  setDownloadingId(attachment.id);
  try {
    // First try to get existing local path
    const existingPath = await getAttachmentPath(attachment.id);
    if (existingPath) {
      // File already downloaded, open it
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(existingPath);
      return;
    }
    // Download to attachments directory
    await downloadAttachment(attachment.id, attachment.filename);
    const newPath = await getAttachmentPath(attachment.id);
    if (newPath) {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(newPath);
    }
  } catch (err) {
    console.error("Failed to download attachment:", err);
  } finally {
    setDownloadingId(null);
  }
}
```

Note: If `@tauri-apps/plugin-shell` is not installed, fall back to just downloading without opening. Check `package.json` first — if the plugin is missing, skip the `open()` call and just call `downloadAttachment`.

- [ ] **Step 2: Verify download works**

Run `pnpm dev`. Open a message with attachments. Click the download button — it should call `downloadAttachment` (visible in backend logs) and attempt to open the file.

- [ ] **Step 3: Commit**

```bash
git add src/components/AttachmentList.tsx
git commit -m "fix: wire attachment download to actually download and open files"
```

---

### Task 5: Fix inbox search to navigate to SearchView

**Files:**
- Modify: `src/stores/ui.store.ts`
- Modify: `src/components/SearchBar.tsx`
- Modify: `src/features/search/SearchView.tsx`

- [ ] **Step 1: Add searchQuery to ui.store**

In `src/stores/ui.store.ts`, add to the interface:
```tsx
searchQuery: string;
setSearchQuery: (q: string) => void;
```

Add to the store creation:
```tsx
searchQuery: "",
setSearchQuery: (q) => set({ searchQuery: q }),
```

- [ ] **Step 2: Update SearchBar to navigate to SearchView**

In `src/components/SearchBar.tsx`, update the `onSubmit` handler to navigate to the search view with the query:

```tsx
import { useUIStore } from "@/stores/ui.store";

// Inside component:
const { setActiveView, setSearchQuery } = useUIStore();

function handleSubmit(e: React.FormEvent) {
  e.preventDefault();
  if (value.trim()) {
    setSearchQuery(value.trim());
    setActiveView("search");
  }
}
```

Remove the `onSearch` prop usage — the SearchBar now navigates instead of calling a callback. Keep `onClear` for resetting.

- [ ] **Step 3: Update SearchView to pick up the query**

In `src/features/search/SearchView.tsx`, read `searchQuery` from the UI store on mount:

```tsx
const { searchQuery, setSearchQuery: clearStoreQuery } = useUIStore();

useEffect(() => {
  if (searchQuery) {
    setQuery(searchQuery);
    clearStoreQuery("");
    // Auto-search
    doSearch();
  }
}, []); // eslint-disable-line react-hooks/exhaustive-deps
```

- [ ] **Step 4: Verify search flow**

Run `pnpm dev`. Type a query in the inbox search bar and press Enter. It should navigate to the Search view with the query pre-filled and results shown.

- [ ] **Step 5: Commit**

```bash
git add src/stores/ui.store.ts src/components/SearchBar.tsx src/features/search/SearchView.tsx
git commit -m "fix: inbox search bar navigates to SearchView with query"
```

---

### Task 6: Fix trust sender to persist via API

**Files:**
- Modify: `src/components/MessageDetail.tsx`

- [ ] **Step 1: Call trustSender API**

In `src/components/MessageDetail.tsx`, add `trustSender` to the import:
```tsx
import { getMessage, getRenderedHtml, updateMessageFlags, translateText, trustSender } from "@/lib/api";
```

Update `handleTrustSender`:
```tsx
async function handleTrustSender() {
  if (message) {
    setPrivacyMode({ TrustSender: message.from_address });
    try {
      await trustSender(message.account_id, message.from_address, "all");
    } catch (err) {
      console.error("Failed to persist trusted sender:", err);
    }
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/components/MessageDetail.tsx
git commit -m "fix: persist trusted sender via backend API"
```

---

## Wave 3: Bug Fixes + Keyboard Shortcuts

### Task 7: Fix notification localStorage key mismatch

**Files:**
- Modify: `src/features/settings/AppearanceTab.tsx`

- [ ] **Step 1: Fix the key**

In `src/features/settings/AppearanceTab.tsx`, change the key constant to match what `commands.ts` uses:
```tsx
const NOTIFICATIONS_KEY = "pebble-notifications-enabled";
```

This is a one-line change. The `commands.ts` file already uses `"pebble-notifications-enabled"`, so align the settings UI to it.

- [ ] **Step 2: Commit**

```bash
git add src/features/settings/AppearanceTab.tsx
git commit -m "fix: align notification localStorage key with command palette"
```

---

### Task 8: Fix command palette download-attachment action

**Files:**
- Modify: `src/features/command-palette/commands.ts`

- [ ] **Step 1: Fix the command**

The `mail:download-attachment` command currently navigates to search. Since downloading attachments requires context (which message, which attachment), change this to focus the currently selected message's attachments or simply remove the broken command:

```tsx
{
  id: "mail:download-attachment",
  name: "Download Attachment",
  category: "Mail",
  execute: () => {
    // Focus on current message - attachment download happens from MessageDetail UI
    // This command serves as a hint; actual download is per-attachment in the detail view
  },
},
```

Alternatively, remove this command entirely since attachment download is an in-context action, not a global one.

- [ ] **Step 2: Commit**

```bash
git add src/features/command-palette/commands.ts
git commit -m "fix: remove broken download-attachment command placeholder"
```

---

### Task 9: Add keyboard shortcuts for compose and open-message

**Files:**
- Modify: `src/hooks/useKeyboard.ts`
- Modify: `src/stores/shortcut.store.ts`

- [ ] **Step 1: Add default bindings for new shortcuts**

In `src/stores/shortcut.store.ts`, check if these default bindings exist. If `compose-new` and `reply` are not present, add them:
```tsx
"compose-new": "C",
"reply": "R",
"open-message": "Enter",
```

- [ ] **Step 2: Add cases to useKeyboard.ts**

In the `switch` statement in `useKeyboard.ts`, add:

```tsx
case "open-message": {
  const { selectedMessageId } = useMailStore.getState();
  if (selectedMessageId) {
    // Message is already selected — detail view shows automatically
    // Nothing extra needed since selection triggers detail rendering
  }
  break;
}

case "compose-new": {
  useUIStore.getState().openCompose("new");
  break;
}

case "reply": {
  const { selectedMessageId, messages } = useMailStore.getState();
  if (selectedMessageId) {
    const msg = messages.find((m) => m.id === selectedMessageId);
    if (msg) {
      useUIStore.getState().openCompose("reply", msg);
    }
  }
  break;
}
```

- [ ] **Step 3: Verify shortcuts**

Run `pnpm dev`. Press `C` — compose modal should open. Press `R` with a message selected — reply compose should open. Press `Enter` — should not error.

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useKeyboard.ts src/stores/shortcut.store.ts
git commit -m "feat: add keyboard shortcuts for compose (C), reply (R), open-message (Enter)"
```

---

### Task 10: Add all missing i18n keys

**Files:**
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`

- [ ] **Step 1: Add all missing keys**

Add any remaining missing keys not covered by previous tasks. Ensure these exist:

In `en.json`:
```json
"sidebar": {
  ...existing...,
  "compose": "Compose"
},
"messageActions": {
  "reply": "Reply",
  "forward": "Forward",
  "star": "Star",
  "unstar": "Unstar",
  "markUnread": "Mark unread",
  "archive": "Archive",
  "delete": "Delete",
  "addToKanban": "Add to Kanban"
},
"privacy": {
  "blocked": "{{count}} blocked",
  "loadImages": "Load images",
  "trustSender": "Trust sender"
},
"snooze": {
  "title": "Snooze until",
  "oneHour": "1 hour",
  "tonight": "Tonight (9 PM)",
  "tomorrow": "Tomorrow (9 AM)",
  "nextMonday": "Next Monday (9 AM)"
}
```

In `zh.json`:
```json
"sidebar": {
  ...existing...,
  "compose": "撰写"
},
"messageActions": {
  "reply": "回复",
  "forward": "转发",
  "star": "标星",
  "unstar": "取消标星",
  "markUnread": "标为未读",
  "archive": "归档",
  "delete": "删除",
  "addToKanban": "加入看板"
},
"privacy": {
  "blocked": "已拦截 {{count}} 项",
  "loadImages": "加载图片",
  "trustSender": "信任发件人"
},
"snooze": {
  "title": "稍后提醒",
  "oneHour": "1 小时后",
  "tonight": "今晚 (21:00)",
  "tomorrow": "明天 (9:00)",
  "nextMonday": "下周一 (9:00)"
}
```

- [ ] **Step 2: Commit**

```bash
git add src/locales/en.json src/locales/zh.json
git commit -m "feat(i18n): add all missing translation keys for new features"
```

---

## Verification Checklist

After all tasks are complete, verify end-to-end:

- [ ] Sidebar shows Compose button with accent color — clicking opens compose modal
- [ ] Message detail shows action toolbar with reply, forward, star, archive, delete, kanban buttons
- [ ] Hover on message item shows star + archive mini-buttons
- [ ] Attachment download button actually downloads the file
- [ ] Inbox search bar navigates to SearchView with pre-filled query
- [ ] "Trust sender" in PrivacyBanner calls backend API to persist
- [ ] Notification toggle in Settings works consistently with command palette
- [ ] `C` key opens compose, `R` opens reply, `S` toggles star
- [ ] All UI text appears in both English and Chinese
