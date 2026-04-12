import { useToastStore } from "@/stores/toast.store";
import { Check, X, AlertTriangle, Info } from "lucide-react";

const iconMap = {
  success: Check,
  error: AlertTriangle,
  info: Info,
};

const accentMap = {
  success: "var(--color-accent)",
  error: "#c0392b",
  info: "var(--color-text-secondary)",
};

export default function ToastContainer() {
  const toasts = useToastStore((s) => s.toasts);
  const removeToast = useToastStore((s) => s.removeToast);

  if (toasts.length === 0) return null;

  return (
    <div
      role="region"
      aria-label="Notifications"
      style={{
        position: "fixed",
        bottom: "56px",
        right: "24px",
        zIndex: 9999,
        display: "flex",
        flexDirection: "column",
        gap: "8px",
        pointerEvents: "none",
      }}
    >
      {toasts.map((toast) => {
        const Icon = iconMap[toast.type];
        const accent = accentMap[toast.type];
        return (
          <div
            key={toast.id}
            role={toast.type === "error" ? "alert" : "status"}
            aria-live={toast.type === "error" ? "assertive" : "polite"}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "10px",
              padding: "12px 16px",
              borderRadius: "8px",
              backgroundColor: "var(--color-sidebar-bg)",
              border: "1px solid var(--color-border)",
              borderLeft: `3px solid ${accent}`,
              boxShadow: "0 4px 12px rgba(0,0,0,0.1)",
              fontSize: "14px",
              color: "var(--color-text-primary)",
              pointerEvents: "auto",
              animation: "toast-in 0.2s ease-out",
              minWidth: "280px",
              maxWidth: "420px",
            }}
          >
            <Icon
              size={17}
              color={accent}
              strokeWidth={toast.type === "success" ? 2.5 : 2}
              style={{ flexShrink: 0 }}
            />
            <span style={{ flex: 1, lineHeight: 1.4 }}>{toast.message}</span>
            {toast.action && (
              <button
                onClick={() => {
                  toast.action!.onClick();
                  removeToast(toast.id);
                }}
                style={{
                  padding: "2px 8px",
                  borderRadius: "4px",
                  border: "1px solid var(--color-border)",
                  backgroundColor: "transparent",
                  color: "var(--color-accent)",
                  fontSize: "12px",
                  fontWeight: 600,
                  cursor: "pointer",
                  whiteSpace: "nowrap",
                  flexShrink: 0,
                }}
              >
                {toast.action.label}
              </button>
            )}
            <button
              aria-label="Close notification"
              onClick={() => removeToast(toast.id)}
              style={{
                background: "none",
                border: "none",
                cursor: "pointer",
                padding: "2px",
                color: "var(--color-text-secondary)",
                display: "flex",
                flexShrink: 0,
                opacity: 0.6,
              }}
            >
              <X size={14} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
