import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useMailStore } from "@/stores/mail.store";
import { useToastStore } from "@/stores/toast.store";
import { listTrustedSenders, removeTrustedSender } from "@/lib/api";
import type { TrustedSender } from "@/lib/api";
import {
  PRIVACY_MODE_KEY,
  readStoredPrivacyMode,
  type StoredPrivacyMode,
} from "@/lib/privacyMode";
import { Trash2 } from "lucide-react";

export default function PrivacyTab() {
  const { t } = useTranslation();
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const [trustedSenders, setTrustedSenders] = useState<TrustedSender[]>([]);
  const [privacyMode, setPrivacyMode] = useState<StoredPrivacyMode>(() =>
    readStoredPrivacyMode(),
  );

  useEffect(() => {
    if (!activeAccountId) {
      setTrustedSenders((prev) => prev.length === 0 ? prev : []);
      return;
    }

    let cancelled = false;
    listTrustedSenders(activeAccountId)
      .then((senders) => {
        if (!cancelled) setTrustedSenders(senders);
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn("Failed to load trusted senders", err);
        useToastStore.getState().addToast({
          message: t("privacy.loadTrustedFailed", "Failed to load trusted senders"),
          type: "error",
        });
      });

    return () => { cancelled = true; };
  }, [activeAccountId, t]);

  function handlePrivacyModeChange(mode: StoredPrivacyMode) {
    setPrivacyMode(mode);
    localStorage.setItem(PRIVACY_MODE_KEY, mode);
  }

  async function handleRemoveTrust(email: string) {
    if (!activeAccountId) return;
    try {
      await removeTrustedSender(activeAccountId, email);
      setTrustedSenders((prev) => prev.filter((s) => s.email !== email));
    } catch (err) {
      console.warn("Failed to remove trusted sender", err);
      useToastStore.getState().addToast({
        message: t("privacy.removeTrustFailed", "Failed to remove trusted sender"),
        type: "error",
      });
    }
  }

  return (
    <div>
      <h2 style={{ fontSize: "18px", fontWeight: 600, marginBottom: "20px" }}>
        {t("privacy.settingsTitle", "Privacy & Tracking")}
      </h2>

      {/* Global privacy mode */}
      <div style={{ marginBottom: "24px" }}>
        <label style={{ fontSize: "13px", fontWeight: 600, display: "block", marginBottom: "8px" }}>
          {t("privacy.defaultMode", "Default privacy mode")}
        </label>
        <div style={{ display: "flex", gap: "8px" }}>
          <button
            onClick={() => handlePrivacyModeChange("strict")}
            style={{
              padding: "8px 16px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: privacyMode === "strict" ? "var(--color-accent)" : "var(--color-bg)",
              color: privacyMode === "strict" ? "#fff" : "var(--color-text-primary)",
              cursor: "pointer",
              fontSize: "13px",
            }}
          >
            {t("privacy.strict", "Strict")}
          </button>
          <button
            onClick={() => handlePrivacyModeChange("relaxed")}
            style={{
              padding: "8px 16px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: privacyMode === "relaxed" ? "var(--color-accent)" : "var(--color-bg)",
              color: privacyMode === "relaxed" ? "#fff" : "var(--color-text-primary)",
              cursor: "pointer",
              fontSize: "13px",
            }}
          >
            {t("privacy.relaxed", "Relaxed")}
          </button>
          <button
            onClick={() => handlePrivacyModeChange("off")}
            style={{
              padding: "8px 16px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: privacyMode === "off" ? "var(--color-accent)" : "var(--color-bg)",
              color: privacyMode === "off" ? "#fff" : "var(--color-text-primary)",
              cursor: "pointer",
              fontSize: "13px",
            }}
          >
            {t("privacy.off", "Off")}
          </button>
        </div>
        <p style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginTop: "6px" }}>
          {privacyMode === "strict"
            ? t("privacy.strictDesc", "Block all external images and trackers by default. You can load images per-message.")
            : privacyMode === "relaxed"
            ? t("privacy.relaxedDesc", "Load external images by default. Trackers are still blocked.")
            : t("privacy.offDesc", "No blocking. All external images and trackers are loaded directly.")}
        </p>
      </div>

      {/* Tracker blocking info */}
      <div style={{
        padding: "12px 16px",
        borderRadius: "6px",
        backgroundColor: "var(--color-bg-hover)",
        marginBottom: "24px",
        fontSize: "13px",
      }}>
        <strong>{t("privacy.trackerBlocking", "Tracker blocking")}</strong>
        <p style={{ margin: "4px 0 0", color: "var(--color-text-secondary)", fontSize: "12px" }}>
          {privacyMode === "off"
            ? t("privacy.trackerBlockingOff", "Tracker blocking is disabled in Off mode. All images and trackers are loaded directly.")
            : t("privacy.trackerBlockingDesc", "Known tracking pixels and tracker domains are blocked unless privacy is Off or the sender is fully trusted.")}
        </p>
      </div>

      {/* Trusted senders */}
      <div>
        <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "12px" }}>
          {t("privacy.trustedSenders", "Trusted Senders")}
        </h3>
        {trustedSenders.length === 0 ? (
          <p style={{ fontSize: "13px", color: "var(--color-text-secondary)" }}>
            {t("privacy.noTrustedSenders", "No trusted senders yet. Trust a sender from the privacy banner in a message.")}
          </p>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
            {trustedSenders.map((sender) => (
              <div
                key={sender.email}
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "8px 12px",
                  borderRadius: "6px",
                  border: "1px solid var(--color-border)",
                  fontSize: "13px",
                }}
              >
                <div>
                  <span>{sender.email}</span>
                  <span style={{
                    marginLeft: "8px",
                    fontSize: "11px",
                    padding: "2px 6px",
                    borderRadius: "4px",
                    backgroundColor: sender.trust_type === "all" ? "var(--color-accent)" : "var(--color-bg-hover)",
                    color: sender.trust_type === "all" ? "#fff" : "var(--color-text-secondary)",
                  }}>
                    {sender.trust_type === "all"
                      ? t("privacy.trustAll", "Trust sender")
                      : t("privacy.trustImages", "Trust images")}
                  </span>
                </div>
                <button
                  onClick={() => handleRemoveTrust(sender.email)}
                  title={t("common.delete", "Delete")}
                  style={{
                    background: "none",
                    border: "none",
                    cursor: "pointer",
                    color: "var(--color-text-secondary)",
                    padding: "4px",
                  }}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
