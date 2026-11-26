import { useEffect, useRef, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useCommandStore } from "@/stores/command.store";
import { useOverlayLayer } from "@/hooks/useOverlayLayer";
import OverlayPortal from "@/components/ui/OverlayPortal";
import { formatShortcut, useShortcutStore } from "@/stores/shortcut.store";
import { COMMAND_SHORTCUT_ACTIONS } from "./commands";
import "./command-palette.css";

export default function CommandPalette() {
  const { t } = useTranslation();
  const { isOpen, query, filteredCommands, close, setQuery, execute } = useCommandStore();
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const bindings = useShortcutStore((s) => s.bindings);

  useOverlayLayer({ open: isOpen, modal: true, onEscape: close, containerRef: dialogRef });

  useEffect(() => {
    if (isOpen) {
      previousFocusRef.current = document.activeElement as HTMLElement;
      setSelectedIndex(0);
    } else if (previousFocusRef.current) {
      previousFocusRef.current.focus();
      previousFocusRef.current = null;
    }
  }, [isOpen]);

  // Focus input when dialog opens
  useEffect(() => {
    if (isOpen) {
      inputRef.current?.focus();
    }
  }, [isOpen]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  // Keep the keyboard-selected command visible.
  useEffect(() => {
    if (!isOpen) return;
    const option = listRef.current?.querySelector<HTMLElement>(`[data-index="${selectedIndex}"]`);
    option?.scrollIntoView?.({ block: "nearest" });
  }, [selectedIndex, isOpen]);

  // Focus trap: keep Tab cycling within the dialog
  const handleTabTrap = useCallback((e: React.KeyboardEvent) => {
    if (e.key !== "Tab") return;
    const dialog = dialogRef.current;
    if (!dialog) return;

    const focusable = dialog.querySelectorAll<HTMLElement>(
      'input, button, [tabindex]:not([tabindex="-1"])',
    );
    if (focusable.length === 0) return;

    const first = focusable[0];
    const last = focusable[focusable.length - 1];

    if (e.shiftKey) {
      if (document.activeElement === first) {
        e.preventDefault();
        last.focus();
      }
    } else {
      if (document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }, []);

  if (!isOpen) return null;

  function handleKeyDown(e: React.KeyboardEvent) {
    handleTabTrap(e);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filteredCommands.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filteredCommands[selectedIndex]) {
        execute(filteredCommands[selectedIndex].id);
      }
    } else if (e.key === "Escape") {
      close();
    }
  }

  return (
    <OverlayPortal>
    <div
      onClick={close}
      className="anim-backdrop-in command-palette-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label={t("commandPalette.title", "Command Palette")}
    >
      <div
        ref={dialogRef}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
        className="anim-dialog-in command-palette"
      >
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("commandPalette.placeholder")}
          aria-label={t("commandPalette.placeholder", "Type a command...")}
          aria-controls="command-listbox"
          aria-activedescendant={
            filteredCommands[selectedIndex]
              ? `cmd-${filteredCommands[selectedIndex].id}`
              : undefined
          }
          role="combobox"
          aria-expanded="true"
          aria-autocomplete="list"
          className="command-palette__input"
        />
        <div
          ref={listRef}
          id="command-listbox"
          className="scroll-region command-list-scroll command-palette__list"
          role="listbox"
          aria-label={t("commandPalette.results", "Commands")}
        >
          {filteredCommands.map((cmd, i) => (
            <div
              key={cmd.id}
              id={`cmd-${cmd.id}`}
              role="option"
              data-index={i}
              aria-selected={i === selectedIndex}
              onClick={() => execute(cmd.id)}
              // Pointer movement (not mere hover while the list scrolls under a
              // still pointer) moves the single highlight.
              onMouseMove={() => { if (i !== selectedIndex) setSelectedIndex(i); }}
              className="command-palette__option"
            >
              <div>
                <span className="command-palette__name">{cmd.name}</span>
                <span className="command-palette__category">{cmd.category}</span>
              </div>
              {(() => {
                const action = COMMAND_SHORTCUT_ACTIONS[cmd.id];
                const keys = action ? bindings[action] : cmd.shortcut;
                if (!keys) return null;
                return (
                  <kbd className="command-palette__kbd">
                    {formatShortcut(keys)}
                  </kbd>
                );
              })()}
            </div>
          ))}
          {filteredCommands.length === 0 && (
            <div className="command-palette__empty">
              {t("commandPalette.noResults")}
            </div>
          )}
        </div>
      </div>
    </div>
    </OverlayPortal>
  );
}
