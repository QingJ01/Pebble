import { useEffect, useRef, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useCommandStore } from "@/stores/command.store";
import { useOverlayLayer } from "@/hooks/useOverlayLayer";
import OverlayPortal from "@/components/ui/OverlayPortal";

export default function CommandPalette() {
  const { t } = useTranslation();
  const { isOpen, query, filteredCommands, close, setQuery, execute } = useCommandStore();
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

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
      className="anim-backdrop-in"
      role="dialog"
      aria-modal="true"
      aria-label={t("commandPalette.title", "Command Palette")}
      style={{
        position: "fixed",
        inset: 0,
        backgroundColor: "var(--overlay-backdrop)",
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "center",
        paddingTop: "20vh",
        zIndex: "var(--z-modal)",
      }}
    >
      <div
        ref={dialogRef}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
        className="anim-dialog-in"
        style={{
          width: "100%",
          maxWidth: "480px",
          backgroundColor: "var(--color-dialog-bg)",
          borderRadius: "var(--radius-xl)",
          border: "1px solid var(--color-border)",
          boxShadow: "var(--shadow-dialog)",
          overflow: "hidden",
        }}
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
          style={{
            width: "100%",
            padding: "16px",
            border: "none",
            borderBottom: "1px solid var(--color-border)",
            backgroundColor: "transparent",
            fontSize: "15px",
            color: "var(--color-text-primary)",
          }}
        />
        <div
          id="command-listbox"
          className="scroll-region command-list-scroll"
          role="listbox"
          aria-label={t("commandPalette.results", "Commands")}
          style={{ maxHeight: "320px", overflowY: "auto" }}
        >
          {filteredCommands.map((cmd, i) => (
            <div
              key={cmd.id}
              id={`cmd-${cmd.id}`}
              role="option"
              aria-selected={i === selectedIndex}
              onClick={() => execute(cmd.id)}
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                padding: "10px 16px",
                cursor: "pointer",
                backgroundColor: i === selectedIndex ? "var(--color-bg-hover, rgba(0,0,0,0.05))" : "transparent",
                color: "var(--color-text-primary)",
              }}
            >
              <div>
                <span style={{ fontSize: "14px" }}>{cmd.name}</span>
                <span
                  style={{
                    fontSize: "11px",
                    color: "var(--color-text-secondary)",
                    marginLeft: "8px",
                  }}
                >
                  {cmd.category}
                </span>
              </div>
              {cmd.shortcut && (
                <span
                  style={{
                    fontSize: "12px",
                    color: "var(--color-text-secondary)",
                    backgroundColor: "var(--color-bg-secondary, rgba(0,0,0,0.06))",
                    padding: "2px 6px",
                    borderRadius: "4px",
                  }}
                >
                  {cmd.shortcut}
                </span>
              )}
            </div>
          ))}
          {filteredCommands.length === 0 && (
            <div
              style={{
                padding: "24px 16px",
                textAlign: "center",
                color: "var(--color-text-secondary)",
                fontSize: "13px",
              }}
            >
              {t("commandPalette.noResults")}
            </div>
          )}
        </div>
      </div>
    </div>
    </OverlayPortal>
  );
}
