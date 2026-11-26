import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Clock, Languages } from "lucide-react";
import { trustSender } from "@/lib/api";
import ResumeDraftButton from "./ResumeDraftButton";
import { useTranslation } from "react-i18next";
import type { PrivacyMode, TranslateResult } from "@/lib/api";
import { MessageDetailSkeleton } from "./Skeleton";
import PrivacyBanner from "./PrivacyBanner";
import AttachmentList from "./AttachmentList";
import SnoozePopover from "../features/inbox/SnoozePopover";
import { ShadowDomEmail } from "./ShadowDomEmail";
import TranslatePopover from "../features/translate/TranslatePopover";
import MessageActionToolbar from "./MessageActionToolbar";
import MoveToButton from "./MoveToButton";
import { useMessageLoader } from "@/hooks/useMessageLoader";
import { useBilingualTranslation } from "@/hooks/useBilingualTranslation";
import { defaultPrivacyMode } from "@/lib/privacyMode";
import { isMessageInKanban } from "@/hooks/queries/useKanbanCardsQuery";
import { queryClient } from "@/lib/query-client";
import { addKanbanCard, setKanbanContextNoteInCache } from "@/hooks/mutations/useKanbanMutations";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useUIStore } from "@/stores/ui.store";
import SelectionActionPopover from "./SelectionActionPopover";
import ContactAddressAction from "./ContactAddressAction";
import MessageParticipants from "./MessageParticipants";
import IconButton from "./ui/IconButton";
import { useDelayedFlag } from "@/hooks/useDelayedFlag";
import { formatDateTime } from "@/lib/formatDate";
import "./ui/overlay.css";

/** Skeleton appears only for loads slower than this… */
const SKELETON_DELAY_MS = 200;
/** …and then stays at least this long, so it never flickers. */
const SKELETON_MIN_VISIBLE_MS = 300;

type PopoverAnchor = { text: string; position: { x: number; y: number } };

/** Keep the last non-null value while `open` is false so exit animations have content. */
function useLatched<T>(value: T | null): { value: T | null; open: boolean } {
  const ref = useRef<T | null>(value);
  if (value !== null) ref.current = value;
  return { value: ref.current, open: value !== null };
}

interface Props {
  messageId: string;
  onBack: () => void;
  folderRole?: string | null;
}

function formatFullDate(timestamp: number, language?: string): string {
  return formatDateTime(timestamp, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }, language);
}

export default function MessageDetail({ messageId, onBack, folderRole }: Props) {
  const { t, i18n } = useTranslation();
  const [privacyOverride, setPrivacyOverride] = useState<{
    messageId: string;
    mode: PrivacyMode;
  }>(() => ({ messageId, mode: defaultPrivacyMode() }));
  const privacyMode = privacyOverride.messageId === messageId
    ? privacyOverride.mode
    : defaultPrivacyMode();
  const [showSnooze, setShowSnooze] = useState(false);
  const [showSelectionActions, setShowSelectionActions] = useState<{ text: string; position: { x: number; y: number } } | null>(null);
  const [showTranslate, setShowTranslate] = useState<{ text: string; position: { x: number; y: number } } | null>(null);

  const snoozeButtonRef = useRef<HTMLButtonElement>(null);
  const selectionPopover = useLatched<PopoverAnchor>(showSelectionActions);
  const translatePopover = useLatched<PopoverAnchor>(showTranslate);

  const { message, setMessage, rendered, loading, isStale, error, errorInfo } = useMessageLoader(messageId, privacyMode);
  const { bilingualMode, bilingualResult, bilingualLoading, handleBilingualToggle, resetBilingual } = useBilingualTranslation(messageId, rendered, message);
  const showSkeleton = useDelayedFlag(loading, { delayMs: SKELETON_DELAY_MS, minVisibleMs: SKELETON_MIN_VISIBLE_MS });

  // Reset bilingual state when messageId changes
  useEffect(() => {
    setPrivacyOverride({ messageId, mode: defaultPrivacyMode() });
    resetBilingual();
    setShowSnooze(false);
    setShowSelectionActions(null);
    setShowTranslate(null);
  }, [messageId]); // eslint-disable-line react-hooks/exhaustive-deps

  function handleLoadImages() {
    setPrivacyOverride({ messageId, mode: "LoadOnce" });
  }

  async function handleTrustSender(trustType: "images" | "all") {
    if (message) {
      try {
        await trustSender(message.account_id, message.from_address, trustType);
        if (trustType === "all") {
          setPrivacyOverride({ messageId, mode: { TrustSender: message.from_address } });
        } else {
          setPrivacyOverride({ messageId, mode: "LoadOnce" });
        }
      } catch (err) {
        console.error("Failed to persist trusted sender:", err);
        toastError(err, "privacy.trustSenderFailed");
      }
    }
  }

  function getCurrentSelectedText() {
    const selection = window.getSelection();
    return selection?.toString().trim() || "";
  }

  function getCurrentSelectionPosition() {
    const selection = window.getSelection();
    const range = selection?.rangeCount ? selection.getRangeAt(0) : null;
    const rect = range?.getBoundingClientRect();
    const x = rect && rect.width > 0 ? rect.left + rect.width / 2 : window.innerWidth / 2;
    const y = rect && rect.height > 0 ? rect.bottom : window.innerHeight / 2;
    return { x, y };
  }

  function openSelectionActionsForSelection(position?: { x: number; y: number }, selectedText = getCurrentSelectedText()) {
    if (selectedText.length <= 5) return false;
    setShowTranslate(null);
    setShowSelectionActions({
      text: selectedText,
      position: position ?? getCurrentSelectionPosition(),
    });
    return true;
  }

  function openTranslateForSelection(position?: { x: number; y: number }) {
    const selectedText = getCurrentSelectedText();
    if (selectedText.length <= 5) return;
    setShowSelectionActions(null);
    setShowTranslate({ text: selectedText, position: position ?? getCurrentSelectionPosition() });
  }

  function handleTranslateSelectedText(text: string, position: { x: number; y: number }) {
    setShowSelectionActions(null);
    setShowTranslate({ text, position });
  }

  function handleSearchSelectedText(text: string) {
    useUIStore.getState().setSearchQuery(text);
    useUIStore.getState().setActiveView("search");
    setShowSelectionActions(null);
  }

  function handleCreateRuleFromSelection(text: string) {
    const ui = useUIStore.getState();
    ui.setPendingRuleDraftText(text);
    ui.setSettingsTab("rules");
    ui.setActiveView("settings");
    setShowSelectionActions(null);
  }

  async function handleAddSelectionToKanbanNote(text: string) {
    setShowSelectionActions(null);
    try {
      if (!isMessageInKanban(messageId, queryClient)) {
        await addKanbanCard(messageId, "todo", queryClient);
      }
      await setKanbanContextNoteInCache(queryClient, messageId, text);
      useUIStore.getState().setActiveView("kanban");
      useToastStore.getState().addToast({
        message: t("kanban.contextNoteAdded", "Added selected text to Kanban note"),
        type: "success",
      });
    } catch (err) {
      toastError(err, "kanban.contextNoteFailed");
    }
  }

  function handleContextMenu(e: React.MouseEvent) {
    const selectedText = getCurrentSelectedText();
    if (selectedText.length <= 5) return;
    e.preventDefault();
    openSelectionActionsForSelection({ x: e.clientX, y: e.clientY }, selectedText);
  }

  useEffect(() => {
    const onTranslate = () => openTranslateForSelection();
    const onBilingual = () => handleBilingualToggle();
    document.addEventListener("pebble:translate-selection", onTranslate);
    document.addEventListener("pebble:toggle-bilingual", onBilingual);
    return () => {
      document.removeEventListener("pebble:translate-selection", onTranslate);
      document.removeEventListener("pebble:toggle-bilingual", onBilingual);
    };
  });

  if (showSkeleton) {
    return <MessageDetailSkeleton />;
  }

  if (loading && !message) {
    // First load: stay blank for a moment instead of flashing a skeleton.
    return <div aria-busy="true" style={{ height: "100%" }} />;
  }

  if (error) {
    return (
      <div
        role="alert"
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          height: "100%",
          color: "var(--color-danger)",
          fontSize: "var(--font-md)",
          padding: "24px",
          textAlign: "center",
        }}
        title={errorInfo?.detail || error}
      >
        {errorInfo?.title ?? t("common.messageLoadFailed", "Failed to load message")}
      </div>
    );
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

  const contactParticipantsGroups = [
    { label: t("messageDetail.to", "To:"), participants: message.to_list },
    { label: t("messageDetail.cc", "Cc:"), participants: message.cc_list },
  ];

  return (
    <div
      className={isStale ? "message-detail-container message-detail--stale" : "message-detail-container"}
      aria-busy={isStale || undefined}
      inert={isStale || undefined}
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
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
          <IconButton
            label={t("compose.back", "Back")}
            icon={<ArrowLeft size={16} />}
            onClick={onBack}
          />
          <h2
            style={{
              fontSize: "16px",
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
          <div style={{ display: "flex", alignItems: "center", gap: "2px", marginLeft: "auto", flexShrink: 0 }}>
            <IconButton
              ref={snoozeButtonRef}
              label={t("messageActions.snooze", "Snooze message")}
              icon={<Clock size={16} />}
              aria-haspopup="menu"
              aria-expanded={showSnooze}
              onClick={() => setShowSnooze((value) => !value)}
            />
            <SnoozePopover
              open={showSnooze}
              anchorRef={snoozeButtonRef}
              messageId={messageId}
              onClose={() => setShowSnooze(false)}
              onSnoozed={() => {
                setShowSnooze(false);
                onBack();
              }}
            />
            <IconButton
              label={t("messageActions.bilingualView", "Toggle bilingual view")}
              icon={<Languages size={16} />}
              pressed={bilingualMode}
              onClick={handleBilingualToggle}
            />
          </div>
        </div>
        {/* Action Toolbar */}
        {message.is_draft && <ResumeDraftButton messageId={messageId} />}
        <MessageActionToolbar
          message={message}
          folderRole={folderRole}
          onBack={onBack}
          onMessageUpdate={setMessage}
        />
        <div style={{ paddingLeft: "48px" }}><MoveToButton messages={[message]} showLabel onMoved={(ids) => {if (ids.includes(message.id)) onBack();}}/></div>
        <div style={{ paddingLeft: "32px" }}>
          <div
            style={{
              backgroundColor: "var(--color-bg-hover)",
              borderRadius: "8px",
              padding: "8px 12px",
              color: "var(--color-text-primary)",
            }}
          >
            <div
              style={{
                fontSize: "13px",
                marginBottom: "2px",
                display: "flex",
                alignItems: "center",
                flexWrap: "wrap",
                gap: "4px",
              }}
            >
              <span style={{ fontWeight: "600" }}>
                {message.from_name || message.from_address}
              </span>
              {message.from_name && (
                <span style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginLeft: "2px" }}>
                  &lt;{message.from_address}&gt;
                </span>
              )}
              <ContactAddressAction
                accountId={message.account_id}
                name={message.from_name}
                address={message.from_address}
              />
              <span
                style={{
                  fontSize: "12px",
                  color: "var(--color-text-secondary)",
                  marginLeft: "auto",
                  flexShrink: 0,
                }}
              >
                {formatFullDate(message.date, i18n?.language)}
              </span>
            </div>
            <MessageParticipants accountId={message.account_id} groups={contactParticipantsGroups} />
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
      <div
        className="scroll-region message-body-scroll"
        tabIndex={0}
        role="region"
        aria-label={t("messageDetail.body", "Message body")}
        style={{ flex: 1, overflow: "auto", padding: "16px" }}
        onContextMenu={handleContextMenu}
      >
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

      {translatePopover.value && (
        <TranslatePopover
          open={translatePopover.open}
          text={translatePopover.value.text}
          position={translatePopover.value.position}
          onClose={() => setShowTranslate(null)}
        />
      )}

      {selectionPopover.value && (
        <SelectionActionPopover
          open={selectionPopover.open}
          text={selectionPopover.value.text}
          position={selectionPopover.value.position}
          onTranslate={handleTranslateSelectedText}
          onSearch={handleSearchSelectedText}
          onCreateRule={handleCreateRuleFromSelection}
          onAddToKanbanNote={handleAddSelectionToKanbanNote}
          onClose={() => setShowSelectionActions(null)}
        />
      )}
    </div>
  );
}
