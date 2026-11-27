import { useCallback, useEffect, useState } from "react";
import { Keyboard, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { formatShortcut, useShortcutStore } from "@/stores/shortcut.store";
import { eventToKeyString } from "@/hooks/useKeyboard";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";
import { VIM_FIRST_KEYS_AS_BINDINGS, VIM_HELP } from "@/lib/vimKeys";
import { SettingsList, SettingsListRow, SettingsNotice, SettingsSection } from "./SettingsLayout";

const ACTION_I18N_MAP: Record<string, string> = {
  "command-palette": "shortcuts.openCommandPalette",
  "close-modal": "shortcuts.closeModal",
  "next-message": "shortcuts.nextMessage",
  "prev-message": "shortcuts.prevMessage",
  "open-message": "shortcuts.openMessage",
  "toggle-star": "shortcuts.toggleStar",
  "archive-message": "shortcuts.archiveMessage",
  "delete-message": "shortcuts.deleteMessage",
  "toggle-view-inbox": "shortcuts.toggleView",
  "toggle-view-kanban": "shortcuts.moveToKanban",
  "compose-new": "shortcuts.composeNew",
  "reply": "shortcuts.reply",
  "reply-all": "shortcuts.replyAll",
  "forward": "shortcuts.forward",
  "open-search": "shortcuts.openSearch",
  "focus-search": "shortcuts.focusSearch",
  "open-cloud-settings": "shortcuts.openCloudSettings",
  "toggle-notifications": "shortcuts.toggleNotifications",
  "translate-selection": "shortcuts.translateSelection",
  "toggle-bilingual": "shortcuts.toggleBilingual",
};

const SHORTCUT_GROUPS = [
  { categoryKey: "shortcuts.general", actions: ["command-palette", "close-modal", "open-cloud-settings", "toggle-notifications"] },
  { categoryKey: "shortcuts.navigation", actions: ["next-message", "prev-message", "open-message", "open-search", "focus-search"] },
  { categoryKey: "shortcuts.mailActions", actions: ["compose-new", "reply", "reply-all", "forward", "toggle-star", "archive-message", "delete-message", "toggle-view-inbox", "toggle-view-kanban"] },
  { categoryKey: "shortcuts.translate", actions: ["translate-selection", "toggle-bilingual"] },
];


/** Fixed (not rebindable) shortcuts that only apply inside a specific view. */
const FIXED_SHORTCUTS: Array<{ id: string; labelKey: string; fallback: string; keys: string }> = [
  { id: "compose-send", labelKey: "shortcuts.composeSend", fallback: "Send message (in compose)", keys: "Ctrl+Enter" },
];

function FixedShortcutRow({ labelKey, fallback, keys }: { labelKey: string; fallback: string; keys: string }) {
  const { t } = useTranslation();
  return (
    <SettingsListRow
      title={t(labelKey, fallback)}
      actions={
        <kbd className="settings-kbd" title={t("shortcuts.fixedBinding", "This shortcut can't be changed")}>
          {formatShortcut(keys)}
        </kbd>
      }
    />
  );
}

function VimKeysList() {
  const { t } = useTranslation();
  const vimMode = useShortcutStore((s) => s.vimMode);
  const setVimMode = useShortcutStore((s) => s.setVimMode);
  const bindings = useShortcutStore((s) => s.bindings);
  const blocking = Object.entries(bindings).filter(
    ([, keys]) => keys && VIM_FIRST_KEYS_AS_BINDINGS.has(keys.toLowerCase()),
  );
  const keysFor = (row: (typeof VIM_HELP)[number]) =>
    row.actions
      ? row.actions.map((action) => formatShortcut(bindings[action] ?? "")).join(" / ")
      : row.shortcuts
        ? row.shortcuts.map((keys) => formatShortcut(keys)).join(" / ")
        : row.keys;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
      <h4 className="settings-subheading">{t("shortcuts.vim.title", "Vim keys")}</h4>
      <SettingsList>
        <SettingsListRow
          title={
            <Checkbox
              checked={vimMode}
              onChange={(e) => setVimMode(e.target.checked)}
              label={t("shortcuts.vim.enable", "Enable vim keys")}
            />
          }
          meta={t("shortcuts.vim.description", "Press sequences like d d or g i within one second. Your own shortcuts above take priority.")}
        />
        {vimMode && blocking.map(([action, keys]) => (
          <SettingsListRow
            key={`conflict-${action}`}
            title={t("shortcuts.vim.blocked", {
              action: t(ACTION_I18N_MAP[action] ?? action),
              keys: formatShortcut(keys),
              defaultValue: "{{keys}} is your shortcut for {{action}}, so vim keys starting with it won't work.",
            })}
          />
        ))}
        {vimMode && VIM_HELP.map((row) => (
          <SettingsListRow
            key={row.labelKey}
            title={t(row.labelKey, row.fallback)}
            actions={<kbd className="settings-kbd">{keysFor(row)}</kbd>}
          />
        ))}
      </SettingsList>
    </div>
  );
}

function ShortcutRow({ actionId }: { actionId: string }) {
  const { t } = useTranslation();
  const bindings = useShortcutStore((s) => s.bindings);
  const recording = useShortcutStore((s) => s.recording);
  const startRecording = useShortcutStore((s) => s.startRecording);
  const stopRecording = useShortcutStore((s) => s.stopRecording);
  const updateShortcut = useShortcutStore((s) => s.updateShortcut);
  const detectConflict = useShortcutStore((s) => s.detectConflict);

  const [conflict, setConflict] = useState<string | null>(null);
  const isRecording = recording === actionId;
  const currentKeys = bindings[actionId] ?? "";

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (!isRecording) return;
      e.preventDefault();
      e.stopPropagation();

      // Allow Escape to cancel recording
      if (e.key === "Escape") {
        stopRecording();
        setConflict(null);
        return;
      }

      // Ignore bare modifier presses
      if (["Control", "Meta", "Shift", "Alt"].includes(e.key)) return;

      const keyString = eventToKeyString(e);
      const conflicting = detectConflict(keyString, actionId);

      if (conflicting) {
        const conflictLabel = t(ACTION_I18N_MAP[conflicting] ?? conflicting);
        setConflict(conflictLabel);
        return;
      }

      setConflict(null);
      updateShortcut(actionId, keyString);
    },
    [isRecording, actionId, stopRecording, detectConflict, updateShortcut, t],
  );

  useEffect(() => {
    if (isRecording) {
      document.addEventListener("keydown", handleKeyDown, true);
      return () => document.removeEventListener("keydown", handleKeyDown, true);
    }
  }, [isRecording, handleKeyDown]);

  // Close recording on outside click
  useEffect(() => {
    if (!isRecording) return;
    const handleClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest(`[data-action="${actionId}"]`)) {
        stopRecording();
        setConflict(null);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [isRecording, actionId, stopRecording]);

  return (
    <div data-action={actionId} className="settings-list__row">
      <div className="settings-list__main">
        <span id={`shortcut-label-${actionId}`} className="settings-list__title">
          {t(ACTION_I18N_MAP[actionId] ?? actionId)}
        </span>
      </div>
      <div className="settings-list__actions" style={{ gap: "var(--space-2)" }}>
        <span
          role="status"
          aria-live="polite"
          style={{ fontSize: "var(--font-xs)", color: "var(--color-danger)" }}
        >
          {conflict ? t("shortcuts.conflict", { action: conflict }) : ""}
        </span>
        <Button
          size="sm"
          pressed={isRecording}
          aria-labelledby={`shortcut-label-${actionId}`}
          aria-label={
            isRecording
              ? t("shortcuts.recording")
              : t("shortcuts.editBinding", { keys: formatShortcut(currentKeys) || t("shortcuts.unbound", "Unbound") })
          }
          onClick={() => {
            if (isRecording) {
              stopRecording();
              setConflict(null);
            } else {
              setConflict(null);
              startRecording(actionId);
            }
          }}
          title={t("shortcuts.edit")}
          style={{ minWidth: "64px", fontFamily: "var(--font-mono)", userSelect: "none" }}
        >
          {isRecording ? t("shortcuts.recording") : formatShortcut(currentKeys)}
        </Button>
      </div>
    </div>
  );
}

export default function ShortcutsTab() {
  const { t } = useTranslation();
  const resetToDefaults = useShortcutStore((s) => s.resetToDefaults);
  const [resetMessage, setResetMessage] = useState(false);

  const handleReset = () => {
    resetToDefaults();
    setResetMessage(true);
    setTimeout(() => setResetMessage(false), 2000);
  };

  return (
    <div>
      <SettingsSection
        title={
          <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--space-2)" }}>
            <Keyboard size={16} aria-hidden="true" />
            {t("shortcuts.title")}
          </span>
        }
        actions={
          <Button size="sm" onClick={handleReset} icon={<RotateCcw size={12} />}>
            {t("shortcuts.resetDefaults")}
          </Button>
        }
      >
        <div role="status" aria-live="polite">
          {resetMessage && <SettingsNotice tone="success">{t("shortcuts.resetConfirm")}</SettingsNotice>}
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-5)" }}>
          {SHORTCUT_GROUPS.map((group) => (
            <div key={group.categoryKey} style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
              <h4 className="settings-subheading">{t(group.categoryKey)}</h4>
              <SettingsList>
                {group.actions.map((actionId) => (
                  <ShortcutRow key={actionId} actionId={actionId} />
                ))}
              </SettingsList>
            </div>
          ))}

          <VimKeysList />

          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
            <h4 className="settings-subheading">{t("shortcuts.compose", "Compose")}</h4>
            <SettingsList>
              {FIXED_SHORTCUTS.map((shortcut) => (
                <FixedShortcutRow key={shortcut.id} labelKey={shortcut.labelKey} fallback={shortcut.fallback} keys={shortcut.keys} />
              ))}
            </SettingsList>
          </div>
        </div>
      </SettingsSection>
    </div>
  );
}
