import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { FolderCog } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Account, Folder } from "@/lib/api";
import { useClickOutside } from "@/hooks/useClickOutside";
import { FolderManagementModal } from "@/components/FolderManagementButton";
import "./sidebar-folder-management.css";

interface Args {
  accounts: Account[];
  folders: Folder[];
  accountId: string | null;
}

function isInteractiveTarget(target: EventTarget | null) {
  return target instanceof Element
    && Boolean(target.closest("button, a, input, select, textarea, [role='button'], [role='menu'], [role='option']"));
}

export function useSidebarFolderManagement({ accounts, folders, accountId }: Args) {
  const { t } = useTranslation();
  const [menuPosition, setMenuPosition] = useState<{ x: number; y: number } | null>(null);
  const [managerOpen, setManagerOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const keyboardButtonRef = useRef<HTMLButtonElement>(null);

  useClickOutside(menuRef, Boolean(menuPosition), () => setMenuPosition(null));

  useEffect(() => {
    if (!menuPosition) return;
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") setMenuPosition(null);
    }
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [menuPosition]);

  function handleContextMenu(event: React.MouseEvent) {
    if (!accounts.length || isInteractiveTarget(event.target)) return;
    event.preventDefault();
    setMenuPosition({ x: event.clientX, y: event.clientY });
  }

  const menuLabel = t("folderManagement.manage", "Manage folders");
  const overlay = <>
    {menuPosition && createPortal(
      <div
        ref={menuRef}
        role="menu"
        aria-label={menuLabel}
        style={{
          position: "fixed",
          left: Math.min(menuPosition.x, window.innerWidth - 190),
          top: Math.min(menuPosition.y, window.innerHeight - 64),
          minWidth: "170px",
          padding: "4px",
          borderRadius: "8px",
          border: "1px solid var(--color-border)",
          backgroundColor: "var(--color-bg)",
          boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
          zIndex: 1000,
        }}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          event.stopPropagation();
          if (event.key === "Escape") setMenuPosition(null);
        }}
        onContextMenu={(event) => event.preventDefault()}
      >
        <button
          type="button"
          role="menuitem"
          autoFocus
          onClick={() => {
            keyboardButtonRef.current?.focus();
            setMenuPosition(null);
            setManagerOpen(true);
          }}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "8px",
            width: "100%",
            padding: "7px 9px",
            border: "none",
            borderRadius: "6px",
            backgroundColor: "transparent",
            color: "var(--color-text-primary)",
            cursor: "pointer",
            fontSize: "12.5px",
            textAlign: "left",
          }}
        >
          <FolderCog size={14} />
          <span>{menuLabel}</span>
        </button>
      </div>,
      document.body,
    )}
    {accounts.length > 0 && (
      <button
        ref={keyboardButtonRef}
        type="button"
        className="sidebar-keyboard-only-action"
        onKeyDown={(event) => event.stopPropagation()}
        onClick={() => setManagerOpen(true)}
      >
        {menuLabel}
      </button>
    )}
    {managerOpen && accounts.length > 0 && (
      <FolderManagementModal
        accounts={accounts}
        folders={folders}
        accountId={accountId}
        onClose={() => setManagerOpen(false)}
      />
    )}
  </>;

  return { handleContextMenu, overlay };
}
