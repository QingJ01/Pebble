import { useState, useEffect } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { getRenderedHtml } from "@/lib/api";
import type { Message, RenderedHtml } from "@/lib/api";
import { defaultPrivacyMode } from "@/lib/privacyMode";
import { sanitizeHtml } from "@/lib/sanitizeHtml";
import { formatDateTime } from "@/lib/formatDate";
import { ShadowDomEmail } from "./ShadowDomEmail";
import ContactAddressAction from "./ContactAddressAction";
import MessageParticipants from "./MessageParticipants";
import AttachmentList from "./AttachmentList";
import ResumeDraftButton from "./ResumeDraftButton";
import MoveToButton from "./MoveToButton";
import "./message-reader.css";

interface Props {
  message: Message;
  defaultExpanded?: boolean;
}

function formatFullDate(timestamp: number, language?: string): string {
  return formatDateTime(timestamp, {
    year: "numeric", month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit",
  }, language);
}

export default function ThreadMessageBubble({ message, defaultExpanded = false }: Props) {
  const { t, i18n } = useTranslation();
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [rendered, setRendered] = useState<RenderedHtml | null>(null);
  const contactParticipantsGroups = [
    { label: t("thread.to", "To:"), participants: message.to_list ?? [] },
    { label: t("thread.cc", "Cc:"), participants: message.cc_list ?? [] },
  ];

  useEffect(() => {
    if (expanded && !rendered) {
      getRenderedHtml(message.id, defaultPrivacyMode())
        .then((html) => setRendered({ ...html, html: sanitizeHtml(html.html) }))
        .catch((err) => console.warn("Failed to render thread message HTML", err));
    }
  }, [expanded, rendered, message.id]);

  return (
    <div className="thread-bubble">
      {message.is_draft && (
        <div className="thread-bubble__draft">
          <ResumeDraftButton messageId={message.id} />
        </div>
      )}
      {/* Header - always visible (full-width disclosure row) */}
      <button
        type="button"
        className="thread-bubble__header"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
      >
        {expanded
          ? <ChevronDown size={14} className="thread-bubble__chevron" aria-hidden="true" />
          : <ChevronRight size={14} className="thread-bubble__chevron" aria-hidden="true" />}
        <span className="thread-bubble__sender">
          {message.from_name || message.from_address}
        </span>
        <span className="thread-bubble__date">
          {formatFullDate(message.date, i18n?.language)}
        </span>
      </button>

      {/* Body - only when expanded */}
      {expanded && (
        <div className="thread-bubble__body">
          <MoveToButton messages={[message]} showLabel/>
          <div className="reader-sender-card">
            <div className="reader-sender-card__line">
              <span className="reader-sender-card__name">
                {message.from_name || message.from_address}
              </span>
              {message.from_name && (
                <span className="reader-sender-card__address">
                  &lt;{message.from_address}&gt;
                </span>
              )}
              <ContactAddressAction
                accountId={message.account_id}
                name={message.from_name}
                address={message.from_address}
              />
            </div>
            <MessageParticipants accountId={message.account_id} groups={contactParticipantsGroups} />
          </div>
          {rendered?.html ? (
            <ShadowDomEmail html={rendered.html} />
          ) : (
            <pre className="reader-plain-text">
              {message.body_text}
            </pre>
          )}
          {message.has_attachments && <AttachmentList messageId={message.id} />}
        </div>
      )}
    </div>
  );
}
