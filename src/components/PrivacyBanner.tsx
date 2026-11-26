import { Shield } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { RenderedHtml } from "@/lib/api";
import Button from "./ui/Button";

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
    <div
      role="region"
      aria-label={t("privacy.bannerLabel", "Blocked content")}
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        columnGap: "12px",
        rowGap: "6px",
        padding: "6px 14px",
        backgroundColor: "var(--color-warning-bg)",
        borderBottom: "1px solid var(--color-border)",
        fontSize: "var(--font-sm)",
        color: "var(--color-text-secondary)",
        flexShrink: 0,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: "8px", flex: "1 1 240px", minWidth: 0 }}>
        <Shield size={14} aria-hidden="true" style={{ color: "var(--color-warning)", flexShrink: 0 }} />
        <span style={{ minWidth: 0, lineHeight: 1.5 }}>
          {t("privacy.blocked", { items: parts.join(` ${t("privacy.and")} `) })}
        </span>
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", gap: "6px", marginLeft: "auto" }}>
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
