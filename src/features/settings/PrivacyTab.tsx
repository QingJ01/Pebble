import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useMailStore } from "@/stores/mail.store";
import { useToastStore } from "@/stores/toast.store";
import { listTrustedSenders, removeTrustedSender } from "@/lib/api";
import type { TrustedSender } from "@/lib/api";
import { profileLocalStorage } from "@/lib/profileStorage";
import {
  PRIVACY_MODE_KEY,
  readStoredPrivacyMode,
  type StoredPrivacyMode,
} from "@/lib/privacyMode";
import { Trash2 } from "lucide-react";
import IconButton from "@/components/ui/IconButton";
import {
  OptionCard,
  OptionCardGroup,
  SettingsEmpty,
  SettingsList,
  SettingsListRow,
  SettingsNotice,
  SettingsSection,
  StatusChip,
} from "./SettingsLayout";

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
    profileLocalStorage.setItem(PRIVACY_MODE_KEY, mode);
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

  const modes: Array<{ mode: StoredPrivacyMode; label: string }> = [
    { mode: "strict", label: t("privacy.strict", "Strict") },
    { mode: "relaxed", label: t("privacy.relaxed", "Relaxed") },
    { mode: "off", label: t("privacy.off", "Off") },
  ];

  return (
    <div>
      <SettingsSection
        title={t("privacy.defaultMode", "Default privacy mode")}
        description={privacyMode === "strict"
          ? t("privacy.strictDesc", "Block all external images and trackers by default. You can load images per-message.")
          : privacyMode === "relaxed"
          ? t("privacy.relaxedDesc", "Load external images by default. Trackers are still blocked.")
          : t("privacy.offDesc", "No blocking. All external images and trackers are loaded directly.")}
      >
        <OptionCardGroup label={t("privacy.defaultMode", "Default privacy mode")} columns={3}>
          {modes.map((option) => (
            <OptionCard
              key={option.mode}
              selected={privacyMode === option.mode}
              onClick={() => handlePrivacyModeChange(option.mode)}
              title={option.label}
            />
          ))}
        </OptionCardGroup>
        <SettingsNotice>
          <strong>{t("privacy.trackerBlocking", "Tracker blocking")}</strong>
          <div style={{ marginTop: "var(--space-1)" }}>
            {privacyMode === "off"
              ? t("privacy.trackerBlockingOff", "Tracker blocking is disabled in Off mode. All images and trackers are loaded directly.")
              : t("privacy.trackerBlockingDesc", "Known tracking pixels and tracker domains are blocked unless privacy is Off or the sender is fully trusted.")}
          </div>
        </SettingsNotice>
      </SettingsSection>

      <SettingsSection title={t("privacy.trustedSenders", "Trusted Senders")}>
        {trustedSenders.length === 0 ? (
          <SettingsEmpty>
            {t("privacy.noTrustedSenders", "No trusted senders yet. Trust a sender from the privacy banner in a message.")}
          </SettingsEmpty>
        ) : (
          <SettingsList>
            {trustedSenders.map((sender) => (
              <SettingsListRow
                key={sender.email}
                title={
                  <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--space-2)", maxWidth: "100%" }}>
                    <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{sender.email}</span>
                    <StatusChip tone={sender.trust_type === "all" ? "info" : "neutral"}>
                      {sender.trust_type === "all"
                        ? t("privacy.trustAll", "Trust sender")
                        : t("privacy.trustImages", "Trust images")}
                    </StatusChip>
                  </span>
                }
                actions={
                  <IconButton
                    variant="danger"
                    label={t("common.delete", "Delete")}
                    onClick={() => handleRemoveTrust(sender.email)}
                    icon={<Trash2 size={14} />}
                  />
                }
              />
            ))}
          </SettingsList>
        )}
      </SettingsSection>
    </div>
  );
}
