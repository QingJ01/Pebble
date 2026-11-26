import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Check, X } from "lucide-react";
import { translateText } from "@/lib/api";
import { profileLocalStorage } from "@/lib/profileStorage";
import { formatError } from "@/lib/extractErrorMessage";

interface Props {
  text: string;
  position: { x: number; y: number };
  onClose: () => void;
}

export default function TranslatePopover({ text, position, onClose }: Props) {
  const { t } = useTranslation();
  const [translated, setTranslated] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const uiLang = profileLocalStorage.getItem("pebble-language") || "zh";
  const [targetLang, setTargetLang] = useState(uiLang === "en" ? "zh" : "en");

  const [privacyAcked, setPrivacyAcked] = useState(() =>
    profileLocalStorage.getItem("pebble-translate-privacy-ack") === "1",
  );

  useEffect(() => {
    if (!privacyAcked) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError("");
    translateText(text, "auto", targetLang)
      .then((result) => {
        if (!cancelled) setTranslated(result.translated);
      })
      .catch((err) => {
        if (!cancelled) setError(formatError(err, { fallbackKey: "errors.translate" }));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, [text, targetLang, privacyAcked]);

  function handleAcceptPrivacy() {
    profileLocalStorage.setItem("pebble-translate-privacy-ack", "1");
    setPrivacyAcked(true);
  }

  function handleCopy() {
    navigator.clipboard.writeText(translated);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div
      style={{
        position: "fixed",
        left: Math.min(position.x, window.innerWidth - 340),
        top: Math.min(position.y + 10, window.innerHeight - 200),
        width: "320px",
        padding: "12px",
        borderRadius: "8px",
        border: "1px solid var(--color-border)",
        backgroundColor: "var(--color-bg)",
        boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
        zIndex: 1000,
        color: "var(--color-text-primary)",
      }}
      onClick={(e) => e.stopPropagation()}
    >
      {/* Header with target lang selector */}
      <div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "8px" }}>
        <span style={{ fontSize: "11px", fontWeight: 600, color: "var(--color-text-secondary)", textTransform: "uppercase", flex: 1 }}>
          {t("translate.title")}
        </span>
        <select
          value={targetLang}
          onChange={(e) => setTargetLang(e.target.value)}
          style={{
            fontSize: "11px",
            padding: "2px 4px",
            border: "1px solid var(--color-border)",
            borderRadius: "4px",
            backgroundColor: "var(--color-bg)",
            color: "var(--color-text-primary)",
          }}
        >
          <option value="zh">{t("languages.chinese", "Chinese")}</option>
          <option value="en">{t("languages.english", "English")}</option>
          <option value="ja">{t("languages.japanese", "Japanese")}</option>
          <option value="ko">{t("languages.korean", "Korean")}</option>
          <option value="fr">{t("languages.french", "French")}</option>
          <option value="de">{t("languages.german", "German")}</option>
          <option value="es">{t("languages.spanish", "Spanish")}</option>
        </select>
        <button
          onClick={onClose}
          style={{
            background: "none",
            border: "none",
            cursor: "pointer",
            padding: "2px",
            color: "var(--color-text-secondary)",
            display: "flex",
            alignItems: "center",
          }}
        >
          <X size={14} />
        </button>
      </div>

      {/* Privacy notice for first-time use */}
      {!privacyAcked ? (
        <div style={{ fontSize: "13px", lineHeight: 1.5, padding: "8px 0" }}>
          <p style={{ margin: "0 0 8px", color: "var(--color-warning, #e67e22)" }}>
            {t(
              "translate.privacyNotice",
              "Translation will send the selected text to a third-party translation service. Your email content will leave this device.",
            )}
          </p>
          <button
            onClick={handleAcceptPrivacy}
            style={{
              padding: "6px 14px",
              border: "1px solid var(--color-border)",
              borderRadius: "4px",
              backgroundColor: "var(--color-accent, #3b82f6)",
              color: "#fff",
              fontSize: "12px",
              cursor: "pointer",
            }}
          >
            {t("translate.acceptAndContinue", "I understand, continue")}
          </button>
        </div>
      ) : /* Content */
      loading ? (
        <div style={{ fontSize: "13px", color: "var(--color-text-secondary)", padding: "8px 0" }}>
          {t("common.translating")}
        </div>
      ) : error ? (
        <div style={{ fontSize: "13px", color: "#ef4444", padding: "8px 0" }}>
          {error}
        </div>
      ) : (
        <div>
          <div style={{ fontSize: "13px", lineHeight: "1.5", marginBottom: "8px" }}>
            {translated}
          </div>
          <button
            onClick={handleCopy}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "4px",
              padding: "4px 8px",
              border: "1px solid var(--color-border)",
              borderRadius: "4px",
              backgroundColor: "transparent",
              color: "var(--color-text-secondary)",
              fontSize: "11px",
              cursor: "pointer",
            }}
          >
            {copied ? <Check size={12} /> : <Copy size={12} />}
            {copied ? t("common.copied") : t("common.copy")}
          </button>
        </div>
      )}
    </div>
  );
}
