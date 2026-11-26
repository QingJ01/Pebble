import { PenLine } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "@/stores/ui.store";
import { useComposeStore } from "@/stores/compose.store";
import { useMailStore } from "@/stores/mail.store";

/** FAB geometry, shared with ToastContainer so toasts stack above the FAB. */
export const COMPOSE_FAB_BOTTOM = 48;
export const COMPOSE_FAB_SIZE = 48;

/** Whether the compose FAB is currently shown. */
export function useComposeFabVisible(): boolean {
  const activeView = useUIStore((s) => s.activeView);
  const selectedMessageId = useMailStore((s) => s.selectedMessageId);
  return !(activeView === "compose" || selectedMessageId);
}

export default function ComposeFAB() {
  const { t } = useTranslation();
  const openCompose = useComposeStore((s) => s.openCompose);
  const visible = useComposeFabVisible();

  if (!visible) return null;

  return (
    <button
      type="button"
      className="compose-fab"
      onClick={() => openCompose("new")}
      aria-label={t("sidebar.compose", "Compose")}
      title={t("sidebar.compose", "Compose")}
      style={{
        bottom: `${COMPOSE_FAB_BOTTOM}px`,
        width: `${COMPOSE_FAB_SIZE}px`,
        height: `${COMPOSE_FAB_SIZE}px`,
      }}
    >
      <PenLine size={20} aria-hidden="true" />
    </button>
  );
}
