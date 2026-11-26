import { useEffect, useLayoutEffect, useRef, useState } from "react";
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
import "./mail-list.css";
import "./message-reader.css";

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
  /** Reopen a message after an archive/delete from the toolbar failed. */
  onReopen?: (messageId: string) => void;
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

export default function MessageDetail({ messageId, onBack, onReopen, folderRole }: Props) {
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
  const backButtonRef = useRef<HTMLButtonElement>(null);
  const toolbarScopeRef = useRef<HTMLDivElement>(null);
  const bodyScopeRef = useRef<HTMLDivElement>(null);
  const selectionPopover = useLatched<PopoverAnchor>(showSelectionActions);
  const translatePopover = useLatched<PopoverAnchor>(showTranslate);

  const { message, setMessage, rendered, loading, isStale, error, errorInfo } = useMessageLoader(messageId, privacyMode);
  const { bilingualMode, bilingualResult, bilingualLoading, handleBilingualToggle, resetBilingual } = useBilingualTranslation(messageId, rendered, message);
  const showSkeleton = useDelayedFlag(loading, { delayMs: SKELETON_DELAY_MS, minVisibleMs: SKELETON_MIN_VISIBLE_MS });

  // Going stale makes the toolbar and body inert, which would drop focus to
  // <body>; keep it in the reader on Back (still usable) instead.
  useLayoutEffect(() => {
    if (!isStale) return;
    const active = document.activeElement;
    if (!active) return;
    if (toolbarScopeRef.current?.contains(active) || bodyScopeRef.current?.contains(active)) {
      backButtonRef.current?.focus();
    }
  }, [isStale]);

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
    return <div aria-busy="true" className="reader-state" />;
  }

  // Error and not-found states keep a Back button: the user needs a way out.
  const backOnlyToolbar = (
    <div className="reader-toolbar">
      <IconButton
        size="md"
        label={t("compose.back", "Back")}
        icon={<ArrowLeft size={16} />}
        onClick={onBack}
      />
    </div>
  );

  if (error) {
    return (
      <div className="message-detail-container">
        {backOnlyToolbar}
        <div role="alert" className="reader-state reader-state--error" title={errorInfo?.detail || error}>
          {errorInfo?.title ?? t("common.messageLoadFailed", "Failed to load message")}
        </div>
      </div>
    );
  }

  if (!message) {
    return (
      <div className="message-detail-container">
        {backOnlyToolbar}
        <div className="reader-state">
          {t("common.messageNotFound", "Message not found")}
        </div>
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
    >
      {/* Toolbar row: back · message actions · snooze / bilingual (all 32px) */}
      <div className="reader-toolbar">
        <IconButton
          ref={backButtonRef}
          size="md"
          label={t("compose.back", "Back")}
          icon={<ArrowLeft size={16} />}
          onClick={onBack}
        />
        {/* While the next message loads, only Back stays usable: the other
            actions would act on the previous (stale) message. */}
        <div ref={toolbarScopeRef} className="contents" inert={isStale || undefined}>
        <span className="toolbar-separator" aria-hidden="true" />
        <MessageActionToolbar
          message={message}
          folderRole={folderRole}
          onBack={onBack}
          onReopen={onReopen}
          onMessageUpdate={setMessage}
        />
        <div className="reader-toolbar__end">
          <IconButton
            ref={snoozeButtonRef}
            size="md"
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
            size="md"
            label={t("messageActions.bilingualView", "Toggle bilingual view")}
            icon={<Languages size={16} />}
            pressed={bilingualMode}
            onClick={handleBilingualToggle}
          />
        </div>
        </div>
      </div>

      <div ref={bodyScopeRef} className="contents" inert={isStale || undefined}>

      {/* Header: subject, draft resume, sender card — on the content edge */}
      <div className="reader-header">
        <h2 className="reader-title" title={message.subject || undefined}>
          {message.subject || t("inbox.noSubject", "(no subject)")}
        </h2>
        {message.is_draft && <ResumeDraftButton messageId={messageId} />}
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
            <span className="reader-sender-card__date">
              {formatFullDate(message.date, i18n?.language)}
            </span>
          </div>
          <MessageParticipants accountId={message.account_id} groups={contactParticipantsGroups} />
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
        className="scroll-region message-body-scroll reader-body"
        tabIndex={0}
        role="region"
        aria-label={t("messageDetail.body", "Message body")}
        onContextMenu={handleContextMenu}
      >
        {bilingualMode && bilingualLoading ? (
          <div className="reader-body__status">{t("common.translating", "Translating...")}</div>
        ) : bilingualMode && bilingualResult ? (
          (bilingualResult as TranslateResult & { _isHtml?: boolean })._isHtml ? (
            <ShadowDomEmail html={bilingualResult.translated} />
          ) : (
            <pre className="reader-plain-text reader-plain-text--translated">
              {bilingualResult.translated}
            </pre>
          )
        ) : bilingualMode && !bilingualLoading ? (
          <div className="reader-body__status">{t("common.translationFailed", "Translation failed")}</div>
        ) : rendered && rendered.html ? (
          <ShadowDomEmail html={rendered.html} />
        ) : (
          <pre className="reader-plain-text">
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
    </div>
  );
}
