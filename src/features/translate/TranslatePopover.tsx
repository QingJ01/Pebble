import { useState, useEffect, useId } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Check, X } from "lucide-react";
import { translateText } from "@/lib/api";
import { profileLocalStorage } from "@/lib/profileStorage";
import { describeError, type DescribedError } from "@/lib/extractErrorMessage";
import Popover from "@/components/ui/Popover";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import { selectClassName } from "@/components/ui/Select";
import "./translate.css";

interface Props {
  text: string;
  position: { x: number; y: number };
  open?: boolean;
  onClose: () => void;
}

export default function TranslatePopover({ text, position, open = true, onClose }: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const [translated, setTranslated] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<DescribedError | null>(null);
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
    setError(null);
    translateText(text, "auto", targetLang)
      .then((result) => {
        if (!cancelled) setTranslated(result.translated);
      })
      .catch((err) => {
        if (!cancelled) setError(describeError(err, { fallbackKey: "errors.translate" }));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, [text, targetLang, privacyAcked]);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

  function handleAcceptPrivacy() {
    profileLocalStorage.setItem("pebble-translate-privacy-ack", "1");
    setPrivacyAcked(true);
  }

  function handleCopy() {
    void navigator.clipboard.writeText(translated).then(() => setCopied(true)).catch(() => {});
  }

  return (
    <Popover
      open={open}
      onClose={onClose}
      point={position}
      placement="bottom-start"
      role="dialog"
      aria-labelledby={titleId}
      initialFocus="container"
      className="translate-popover"
    >
      {/* Header with target lang selector */}
      <div className="translate-popover__header">
        <h2 id={titleId} className="translate-popover__title">
          {t("translate.title")}
        </h2>
        <select
          value={targetLang}
          onChange={(e) => setTargetLang(e.target.value)}
          aria-label={t("translate.targetLanguage", "Target language")}
          className={selectClassName("sm")}
        >
          <option value="zh">{t("languages.chinese", "Chinese")}</option>
          <option value="en">{t("languages.english", "English")}</option>
          <option value="ja">{t("languages.japanese", "Japanese")}</option>
          <option value="ko">{t("languages.korean", "Korean")}</option>
          <option value="fr">{t("languages.french", "French")}</option>
          <option value="de">{t("languages.german", "German")}</option>
          <option value="es">{t("languages.spanish", "Spanish")}</option>
        </select>
        <IconButton label={t("common.close", "Close")} icon={<X size={14} />} onClick={onClose} />
      </div>

      {/* Privacy notice for first-time use */}
      {!privacyAcked ? (
        <div className="translate-popover__notice">
          <p>
            {t(
              "translate.privacyNotice",
              "Translation will send the selected text to a third-party translation service. Your email content will leave this device.",
            )}
          </p>
          <Button variant="primary" size="sm" onClick={handleAcceptPrivacy}>
            {t("translate.acceptAndContinue", "I understand, continue")}
          </Button>
        </div>
      ) : loading ? (
        <div role="status" className="translate-popover__status">
          {t("common.translating")}
        </div>
      ) : error ? (
        <div role="alert" className="dialog-error translate-popover__error" title={error.detail || undefined}>
          {error.title}
        </div>
      ) : (
        <div>
          <div className="translate-popover__result">
            {translated}
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={handleCopy}
            icon={copied ? <Check size={14} /> : <Copy size={14} />}
          >
            {copied ? t("common.copied") : t("common.copy")}
          </Button>
        </div>
      )}
    </Popover>
  );
}
