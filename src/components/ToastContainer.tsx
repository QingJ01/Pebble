import { useState } from "react";
import { Check, X, AlertTriangle, Info, AlertCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { selectVisibleToasts, useToastStore, type Toast } from "@/stores/toast.store";
import { COMPOSE_FAB_BOTTOM, COMPOSE_FAB_SIZE, useComposeFabVisible } from "./ComposeFAB";
import OverlayPortal from "./ui/OverlayPortal";
import Button from "./ui/Button";
import IconButton from "./ui/IconButton";

const iconMap = {
  success: Check,
  error: AlertTriangle,
  warning: AlertCircle,
  info: Info,
};

/** Above the status bar; lifted above the compose FAB when it is visible. */
const BASE_BOTTOM = 56;
const ABOVE_FAB_BOTTOM = COMPOSE_FAB_BOTTOM + COMPOSE_FAB_SIZE + 16;

export default function ToastContainer() {
  const { t } = useTranslation();
  const toasts = useToastStore((s) => s.toasts);
  const pauseToasts = useToastStore((s) => s.pauseToasts);
  const resumeToasts = useToastStore((s) => s.resumeToasts);
  const fabVisible = useComposeFabVisible();
  const visible = selectVisibleToasts(toasts);

  if (visible.length === 0) return null;

  return (
    <OverlayPortal>
      <div
        role="region"
        aria-label={t("common.notifications", "Notifications")}
        className="toast-region"
        style={{ bottom: `${fabVisible ? ABOVE_FAB_BOTTOM : BASE_BOTTOM}px` }}
        onMouseEnter={pauseToasts}
        onMouseLeave={resumeToasts}
        onFocus={pauseToasts}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) resumeToasts();
        }}
      >
        {visible.map((toast) => <ToastItem key={toast.id} toast={toast} />)}
      </div>
    </OverlayPortal>
  );
}

function ToastItem({ toast }: { toast: Toast }) {
  const { t } = useTranslation();
  const removeToast = useToastStore((s) => s.removeToast);
  const [showDetail, setShowDetail] = useState(false);
  const Icon = iconMap[toast.type] ?? Info;
  const isError = toast.type === "error";

  return (
    <div
      role={isError ? "alert" : "status"}
      aria-live={isError ? "assertive" : "polite"}
      aria-atomic="true"
      className={`toast toast--${toast.type} ${toast.exiting ? "anim-toast-out" : "anim-toast-in"}`}
    >
      <Icon className="toast__icon" size={16} strokeWidth={toast.type === "success" ? 2.5 : 2} aria-hidden="true" />
      <div className="toast__body">
        <span>{toast.message}</span>
        {toast.count > 1 && (
          <span className="toast__count" aria-label={t("common.repeatedCount", { count: toast.count, defaultValue: "Repeated {{count}} times" })}>
            ×{toast.count}
          </span>
        )}
        {toast.detail && (
          <div>
            <Button
              variant="link"
              size="sm"
              className="toast__detail-toggle"
              aria-expanded={showDetail}
              onClick={() => setShowDetail((v) => !v)}
            >
              {showDetail ? t("common.hideDetails", "Hide details") : t("common.showDetails", "Show details")}
            </Button>
            {showDetail && <pre className="toast__detail">{toast.detail}</pre>}
          </div>
        )}
      </div>
      <div className="toast__actions">
        {toast.action && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              toast.action!.onClick();
              removeToast(toast.id);
            }}
          >
            {toast.action.label}
          </Button>
        )}
        <IconButton
          label={t("common.closeNotification", "Close notification")}
          icon={<X size={14} />}
          onClick={() => removeToast(toast.id)}
        />
      </div>
    </div>
  );
}
