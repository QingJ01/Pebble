import { useEffect, useRef, useState } from "react";
import { Check, Copy, Languages, LayoutGrid, MoreHorizontal, Search, ShieldCheck, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import Popover, { handleMenuKeyDown } from "./ui/Popover";
import Button from "./ui/Button";
import IconButton from "./ui/IconButton";
import "./message-reader.css";

interface Props {
  text: string;
  position: { x: number; y: number };
  open?: boolean;
  onTranslate: (text: string, position: { x: number; y: number }) => void;
  onSearch: (text: string) => void;
  onCreateRule: (text: string) => void;
  onAddToKanbanNote: (text: string) => void;
  onClose: () => void;
}

/**
 * Floating toolbar for selected message text. Escape closes the "more" menu
 * first, then the toolbar; focus returns to where it was before opening.
 */
export default function SelectionActionPopover({
  text,
  position,
  open = true,
  onTranslate,
  onSearch,
  onCreateRule,
  onAddToKanbanNote,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [showSecondaryActions, setShowSecondaryActions] = useState(false);
  const moreRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

  useEffect(() => {
    if (!open) setShowSecondaryActions(false);
  }, [open]);

  async function handleCopy() {
    await navigator.clipboard.writeText(text);
    setCopied(true);
  }

  const copyLabel = copied
    ? t("selection.copiedSelectedText", "Copied selected text")
    : t("selection.copySelectedText", "Copy selected text");

  return (
    <Popover
      open={open}
      onClose={onClose}
      point={position}
      placement="bottom-start"
      role="toolbar"
      aria-label={t("selection.actions", "Selected text actions")}
      className="selection-actions"
    >
      <Button
        variant="primary"
        size="sm"
        onClick={handleCopy}
        aria-label={copyLabel}
        title={copyLabel}
        icon={copied ? <Check size={14} /> : <Copy size={14} />}
      >
        {copied ? t("common.copied", "Copied") : t("common.copy", "Copy")}
      </Button>

      <IconButton
        ref={moreRef}
        label={t("selection.moreActions", "More selected-text actions")}
        icon={<MoreHorizontal size={14} />}
        aria-haspopup="menu"
        aria-expanded={showSecondaryActions}
        onClick={() => setShowSecondaryActions((value) => !value)}
      />

      <IconButton label={t("common.close", "Close")} icon={<X size={14} />} onClick={onClose} />

      <Popover
        open={open && showSecondaryActions}
        onClose={() => setShowSecondaryActions(false)}
        anchorRef={moreRef}
        placement="bottom-end"
        role="menu"
        aria-label={t("selection.moreActions", "More selected-text actions")}
        className="menu"
        onKeyDown={handleMenuKeyDown}
      >
        <button
          type="button"
          role="menuitem"
          className="menu-item"
          onClick={() => onTranslate(text, position)}
          aria-label={t("selection.translateSelectedText", "Translate selected text")}
        >
          <Languages size={14} aria-hidden="true" />
          <span>{t("selection.translate", "Translate")}</span>
        </button>
        <button
          type="button"
          role="menuitem"
          className="menu-item"
          onClick={() => onSearch(text)}
          aria-label={t("selection.searchSelectedText", "Search selected text")}
        >
          <Search size={14} aria-hidden="true" />
          <span>{t("selection.search", "Search")}</span>
        </button>
        <button
          type="button"
          role="menuitem"
          className="menu-item"
          onClick={() => onCreateRule(text)}
          aria-label={t("selection.createRuleFromSelection", "Create rule from selected text")}
        >
          <ShieldCheck size={14} aria-hidden="true" />
          <span>{t("selection.createRule", "Create rule")}</span>
        </button>
        <button
          type="button"
          role="menuitem"
          className="menu-item"
          onClick={() => onAddToKanbanNote(text)}
          aria-label={t("selection.addToKanbanNote", "Add selected text as kanban note")}
        >
          <LayoutGrid size={14} aria-hidden="true" />
          <span>{t("selection.addToKanbanNoteLabel", "Add to Kanban note")}</span>
        </button>
      </Popover>
    </Popover>
  );
}
