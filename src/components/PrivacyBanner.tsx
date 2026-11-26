import { Shield } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { RenderedHtml } from "@/lib/api";
import Button from "./ui/Button";
import "./message-reader.css";

interface Props {
  rendered: RenderedHtml;
  onLoadImages: () => void;
  onTrustSender: (trustType: "images" | "all") => void;
}

/**
 * Blocked-content notice above the message body. Text and actions wrap as two
 * groups: at narrow widths the buttons move below the text instead of
 * squeezing it into a one-character column.
 */
export default function PrivacyBanner({ rendered, onLoadImages, onTrustSender }: Props) {
  const { t } = useTranslation();
  const totalBlocked = rendered.trackers_blocked.length + rendered.images_blocked;

  if (totalBlocked === 0) {
    return null;
  }

  const parts: string[] = [];
  if (rendered.trackers_blocked.length > 0) {
    parts.push(t("privacy.tracker", { count: rendered.trackers_blocked.length }));
  }
  if (rendered.images_blocked > 0) {
    parts.push(t("privacy.image", { count: rendered.images_blocked }));
  }

  return (
    <div role="region" aria-label={t("privacy.bannerLabel", "Blocked content")} className="privacy-banner">
      <div className="privacy-banner__text">
        <Shield size={14} aria-hidden="true" className="privacy-banner__icon" />
        <span>{t("privacy.blocked", { items: parts.join(` ${t("privacy.and")} `) })}</span>
      </div>
      {/* Hierarchy: one secondary (show images once) + ghost trust options, all 28px. */}
      <div className="privacy-banner__actions">
        {rendered.images_blocked > 0 && (
          <>
            <Button size="sm" variant="secondary" onClick={onLoadImages}>
              {t("privacy.loadImages")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => onTrustSender("images")}>
              {t("privacy.trustImages", "Trust images")}
            </Button>
          </>
        )}
        <Button size="sm" variant="ghost" onClick={() => onTrustSender("all")}>
          {t("privacy.trustSender", "Trust sender")}
        </Button>
      </div>
    </div>
  );
}
