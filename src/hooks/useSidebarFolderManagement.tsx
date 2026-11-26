import { useRef, useState } from "react";
import { FolderCog } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Account, Folder } from "@/lib/api";
import { FolderManagementModal } from "@/components/FolderManagementButton";
import ContextMenu, { type ContextMenuPosition } from "@/components/ContextMenu";
import { buttonClassName } from "@/components/ui/Button";
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
  const [menuPosition, setMenuPosition] = useState<ContextMenuPosition | null>(null);
  const [managerOpen, setManagerOpen] = useState(false);
  // Account preselected in the manager when it is opened for one account (rail menu).
  const [managerAccountId, setManagerAccountId] = useState<string | null>(null);
  const keyboardButtonRef = useRef<HTMLButtonElement>(null);
  const triggerRef = useRef<HTMLElement | null>(null);

  function handleContextMenu(event: React.MouseEvent) {
    if (!accounts.length || isInteractiveTarget(event.target)) return;
    event.preventDefault();
    triggerRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // Keyboard-generated contextmenu events (Shift+F10 / menu key) have no pointer position.
    if (event.clientX === 0 && event.clientY === 0 && event.currentTarget instanceof HTMLElement) {
      const rect = event.currentTarget.getBoundingClientRect();
      setMenuPosition({ x: rect.left + 16, y: rect.top + 16 });
    } else {
      setMenuPosition({ x: event.clientX, y: event.clientY });
    }
  }

  const menuLabel = t("folderManagement.manage", "Manage folders");
  const overlay = <>
    {menuPosition && (
      <ContextMenu
        label={menuLabel}
        position={menuPosition}
        onClose={() => setMenuPosition(null)}
        restoreFocusTo={triggerRef.current}
        items={[{
          id: "manage",
          label: menuLabel,
          icon: <FolderCog size={14} />,
          onSelect: () => {
            keyboardButtonRef.current?.focus();
            setManagerAccountId(null);
            setManagerOpen(true);
          },
        }]}
      />
    )}
    {accounts.length > 0 && (
      <button
        ref={keyboardButtonRef}
        type="button"
        className={buttonClassName("secondary", "sm", "sidebar-keyboard-only-action")}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") event.stopPropagation();
        }}
        onClick={() => { setManagerAccountId(null); setManagerOpen(true); }}
      >
        {menuLabel}
      </button>
    )}
    {managerOpen && accounts.length > 0 && (
      <FolderManagementModal
        accounts={accounts}
        folders={folders}
        accountId={managerAccountId ?? accountId}
        onClose={() => setManagerOpen(false)}
      />
    )}
  </>;

  /** Open the folder manager directly, preselecting `forAccountId` when given. */
  function openManager(forAccountId?: string | null) {
    setManagerAccountId(forAccountId ?? null);
    setManagerOpen(true);
  }

  return { handleContextMenu, overlay, openManager };
}
