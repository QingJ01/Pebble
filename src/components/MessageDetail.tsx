import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Clock, Languages } from "lucide-react";
import { trustSender } from "@/lib/api";
import { useTranslation } from "react-i18next";
import type { PrivacyMode, TranslateResult } from "@/lib/api";
import { useClickOutside } from "@/hooks/useClickOutside";
import { MessageDetailSkeleton } from "./Skeleton";
import PrivacyBanner from "./PrivacyBanner";
import AttachmentList from "./AttachmentList";
import SnoozePopover from "../features/inbox/SnoozePopover";
import { ShadowDomEmail } from "./ShadowDomEmail";
import TranslatePopover from "../features/translate/TranslatePopover";
import MessageActionToolbar from "./MessageActionToolbar";
import { useMessageLoader } from "@/hooks/useMessageLoader";
import { useBilingualTranslation } from "@/hooks/useBilingualTranslation";

interface Props {
  messageId: string;
  onBack: () => void;
  folderRole?: string | null;
}

function formatFullDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function MessageDetail({ messageId, onBack, folderRole }: Props) {
  const { t } = useTranslation();
  const [privacyMode, setPrivacyMode] = useState<PrivacyMode>(() => {
    const saved = localStorage.getItem("pebble-privacy-mode");
    if (saved === "off") return "Off";
    if (saved === "strict") return "Strict";
    if (saved === "relaxed") return "LoadOnce";
    return "Strict";
  });
  const [showSnooze, setShowSnooze] = useState(false);
  const [showTranslate, setShowTranslate] = useState<{ text: string; position: { x: number; y: number } } | null>(null);

  const snoozeRef = useRef<HTMLDivElement>(null);
  const translateRef = useRef<HTMLDivElement>(null);

  const { message, setMessage, rendered, loading } = useMessageLoader(messageId, privacyMode);
  const { bilingualMode, bilingualResult, bilingualLoading, handleBilingualToggle, resetBilingual } = useBilingualTranslation(messageId, rendered, message);

  useClickOutside(snoozeRef, showSnooze, () => setShowSnooze(false));
  useClickOutside(translateRef, !!showTranslate, () => setShowTranslate(null));

  // Reset bilingual state when messageId changes
  useEffect(() => {
    resetBilingual();
  }, [messageId]); // eslint-disable-line react-hooks/exhaustive-deps

  function handleLoadImages() {
    setPrivacyMode("LoadOnce");
  }

  async function handleTrustSender(trustType: "images" | "all") {
    if (message) {
      if (trustType === "all") {
        setPrivacyMode({ TrustSender: message.from_address });
      } else {
        setPrivacyMode("LoadOnce");
      }
      try {
        await trustSender(message.account_id, message.from_address, trustType);
      } catch (err) {
        console.error("Failed to persist trusted sender:", err);
      }
    }
  }

  function handleMouseUp(e: React.MouseEvent) {
    const selection = window.getSelection();
    const selectedText = selection?.toString().trim() || "";
    if (selectedText.length > 5) {
      setShowTranslate({ text: selectedText, position: { x: e.clientX, y: e.clientY } });
    }
  }

  if (loading) {
    return <MessageDetailSkeleton />;
  }

  if (!message) {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          height: "100%",
          color: "var(--color-text-secondary)",
          fontSize: "14px",
        }}
      >
        {t("common.messageNotFound", "Message not found")}
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        backgroundColor: "var(--color-bg)",
      }}
    >
      {/* Header */}
      <div
        style={{
          padding: "12px 16px",
          borderBottom: "1px solid var(--color-border)",
          flexShrink: 0,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "10px", marginBottom: "8px" }}>
          <button
            onClick={onBack}
            aria-label={t("compose.back", "Back")}
            style={{
              background: "none",
              border: "none",
              cursor: "pointer",
              padding: "4px",
              borderRadius: "4px",
              color: "var(--color-text-secondary)",
              display: "flex",
              alignItems: "center",
            }}
          >
            <ArrowLeft size={18} />
          </button>
          <h2
            style={{
              fontSize: "15px",
              fontWeight: "600",
              color: "var(--color-text-primary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              margin: 0,
            }}
          >
            {message.subject || t("inbox.noSubject", "(no subject)")}
          </h2>
          <div ref={snoozeRef} style={{ position: "relative", marginLeft: "auto", flexShrink: 0 }}>
            <button
              onClick={() => setShowSnooze(!showSnooze)}
              style={{
                background: "none",
                border: "none",
                cursor: "pointer",
                padding: "4px",
                borderRadius: "4px",
                color: "var(--color-text-secondary)",
                display: "flex",
                alignItems: "center",
              }}
              title={t("messageActions.snooze", "Snooze message")}
              aria-label={t("messageActions.snooze", "Snooze message")}
            >
              <Clock size={16} />
            </button>
            {showSnooze && (
              <SnoozePopover
                messageId={messageId}
                onClose={() => setShowSnooze(false)}
                onSnoozed={() => {
                  setShowSnooze(false);
                  onBack();
                }}
              />
            )}
          </div>
          <button
            onClick={handleBilingualToggle}
            aria-pressed={bilingualMode}
            style={{
              background: bilingualMode ? "var(--color-bg-hover)" : "none",
              border: "none",
              cursor: "pointer",
              padding: "4px",
              borderRadius: "4px",
              color: bilingualMode ? "var(--color-accent)" : "var(--color-text-secondary)",
              display: "flex",
              alignItems: "center",
              flexShrink: 0,
            }}
            title={t("messageActions.bilingualView", "Toggle bilingual view")}
            aria-label={t("messageActions.bilingualView", "Toggle bilingual view")}
          >
            <Languages size={16} />
          </button>
        </div>
        {/* Action Toolbar */}
        <MessageActionToolbar
          message={message}
          folderRole={folderRole}
          onBack={onBack}
          onMessageUpdate={setMessage}
        />
        <div style={{ paddingLeft: "32px" }}>
          <div style={{ fontSize: "13px", color: "var(--color-text-primary)", marginBottom: "2px" }}>
            <span style={{ fontWeight: "500" }}>
              {message.from_name || message.from_address}
            </span>
            {message.from_name && (
              <span style={{ color: "var(--color-text-secondary)", marginLeft: "6px" }}>
                &lt;{message.from_address}&gt;
              </span>
            )}
          </div>
          <div style={{ fontSize: "12px", color: "var(--color-text-secondary)" }}>
            {formatFullDate(message.date)}
          </div>
        </div>
      </div>

      {/* Privacy Banner */}
      {rendered && (
        <PrivacyBanner
          rendered={rendered}
          onLoadImages={handleLoadImages}
          onTrustSender={handleTrustSender}
        />
      )}

      {/* Body */}
      <div style={{ flex: 1, overflow: "auto", padding: "16px" }} onMouseUp={handleMouseUp}>
        {bilingualMode && bilingualLoading ? (
            <div style={{ fontSize: "13px", color: "var(--color-text-secondary)" }}>{t("common.translating", "Translating...")}</div>
        ) : bilingualMode && bilingualResult ? (
          (bilingualResult as TranslateResult & { _isHtml?: boolean })._isHtml ? (
            <ShadowDomEmail html={bilingualResult.translated} />
          ) : (
            <pre
              style={{
                fontSize: "14px",
                color: "var(--color-text-primary)",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
                margin: 0,
                fontFamily: "inherit",
                lineHeight: 1.7,
              }}
            >
              {bilingualResult.translated}
            </pre>
          )
        ) : bilingualMode && !bilingualLoading ? (
            <div style={{ fontSize: "13px", color: "var(--color-text-secondary)" }}>{t("common.translationFailed", "Translation failed")}</div>
        ) : rendered && rendered.html ? (
          <ShadowDomEmail html={rendered.html} />
        ) : (
          <pre
            style={{
              fontSize: "13px",
              color: "var(--color-text-primary)",
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              margin: 0,
              fontFamily: "inherit",
            }}
          >
            {message.body_text}
          </pre>
        )}
      </div>

      {/* Attachments */}
      {message.has_attachments && <AttachmentList messageId={message.id} />}

      {showTranslate && (
        <div ref={translateRef}>
          <TranslatePopover
            text={showTranslate.text}
            position={showTranslate.position}
            onClose={() => setShowTranslate(null)}
          />
        </div>
      )}
    </div>
  );
}
