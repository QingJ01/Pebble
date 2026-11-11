import { senderIdentityLabel } from "@/lib/accountIdentity";
import { useState, useEffect, useRef } from "react";
import { EditorContent } from "@tiptap/react";
import {
  ArrowLeft, Send, X, AlertCircle,
  Type, FileCode2, Hash, Eye, EyeOff,
  Paperclip, FileText, Trash2, BookTemplate,
  ChevronDown, ChevronRight,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useMailStore } from "@/stores/mail.store";
import { useComposeStore } from "@/stores/compose.store";
import { useAccountsQuery } from "@/hooks/queries";
import { useSendEmailMutation } from "@/hooks/mutations";
import ContactAutocomplete from "@/components/ContactAutocomplete";
import ConfirmDialog from "@/components/ConfirmDialog";
import { listTemplates, saveTemplate, deleteTemplate } from "@/lib/templates";
import type { EmailTemplate } from "@/lib/templates";
import { useComposeRecipients } from "@/hooks/useComposeRecipients";
import { useComposeDraft, loadDraftFromStorage, clearDraftStorage } from "@/hooks/useComposeDraft";
import { cleanupStagedComposeAttachment, deleteDraft, stageComposeAttachment } from "@/lib/api";
import { appendReplyQuoteHtml, useComposeEditor } from "@/hooks/useComposeEditor";
import { useConfirmStore } from "@/stores/confirm.store";
import { useToastStore } from "@/stores/toast.store";
import type { Account } from "@/lib/ipc-types";
import type { ComposeAttachment } from "./compose-draft";
import { ModeButton, EditorToolbar, MarkdownToolbar, composeStyles } from "./ComposeToolbar";
import { subjectAfterAddingAttachments } from "./attachment-subject";
import { isValidEmailAddress, mergePendingRecipient } from "./recipient-utils";
import { queryClient } from "@/lib/query-client";

export default function ComposeView() {
  const composeMode = useComposeStore((s) => s.composeMode);
  const { data: accounts = [], isLoading, isSuccess } = useAccountsQuery();

  if (composeMode === "new" && isLoading) {
    return <div style={{ height: "100%" }} />;
  }

  return <ComposeViewInner accounts={accounts} accountsLoaded={isSuccess} />;
}

function ComposeViewInner({ accounts, accountsLoaded }: { accounts: Account[]; accountsLoaded: boolean }) {
  const { t } = useTranslation();
  const composeMode = useComposeStore((s) => s.composeMode);
  const composeReplyTo = useComposeStore((s) => s.composeReplyTo);
  const composePrefill = useComposeStore((s) => s.composePrefill);
  const closeCompose = useComposeStore((s) => s.closeCompose);
  const showComposeLeaveConfirm = useComposeStore((s) => s.showComposeLeaveConfirm);
  const confirmCloseCompose = useComposeStore((s) => s.confirmCloseCompose);
  const cancelCloseCompose = useComposeStore((s) => s.cancelCloseCompose);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const discardInProgressRef = useRef(false);
  const [isDiscarding, setIsDiscarding] = useState(false);
  const sendInProgressRef = useRef(false);
  const [sendInProgress, setSendInProgress] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const composeDraft = useComposeStore((s) => s.composeDraft);

  const isReply = composeMode === "reply" || composeMode === "reply-all";
  const restoredDraft = useRef(
    composeDraft ?? (composeMode === "new"
      ? loadDraftFromStorage(accountsLoaded ? accounts.map((account) => account.id) : undefined)
      : null),
  );

  // ─── Recipients ──────────────────────────────────────────────────────────────
  const {
    fromAccountId, setFromAccountId,
    to, setTo, cc, setCc, bcc, setBcc,
    showCc, setShowCc, showBcc, setShowBcc,
  } = useComposeRecipients({
    composeMode, composeReplyTo, accounts, activeAccountId,
    restoredDraft: restoredDraft.current,
    composePrefill,
  });
  const [toInputValue, setToInputValue] = useState("");
  const [ccInputValue, setCcInputValue] = useState("");
  const [bccInputValue, setBccInputValue] = useState("");

  // ─── Subject ─────────────────────────────────────────────────────────────────
  const [subject, setSubject] = useState(() => {
    if (restoredDraft.current) return restoredDraft.current.subject;
    if (composePrefill?.subject) return composePrefill.subject;
    if (!composeReplyTo) return "";
    if (isReply) return `Re: ${composeReplyTo.subject.replace(/^(Re:\s*|Fwd:\s*)+/i, "")}`;
    if (composeMode === "forward") return `Fwd: ${composeReplyTo.subject.replace(/^(Re:\s*|Fwd:\s*)+/i, "")}`;
    return "";
  });
  const subjectRef = useRef(subject);
  subjectRef.current = subject;
  const [sendError, setSendError] = useState<string | null>(null);
  const sendMutation = useSendEmailMutation();

  // ─── Editor ──────────────────────────────────────────────────────────────────
  const {
    editor, editorMode, rawSource, setRawSource,
    richTextHtml, htmlPreview, setHtmlPreview,
    switchMode, textareaRef, quotedReplyHtml,
  } = useComposeEditor({
    fromAccountId, composeMode, composeReplyTo, isReply, t,
    restoredDraft: restoredDraft.current,
    prefillBody: composePrefill?.body,
  });

  // ─── Draft persistence ───────────────────────────────────────────────────────
  // Delay the dirty-snapshot until the editor has run its initial setContent
  // cycle — otherwise the snapshot captures an empty richTextHtml and then
  // flips dirty when signature/quoted-reply text populates.
  // For "new" composes with no signature the editor stays empty but is still
  // "ready" once mounted, so we gate on editor presence plus one effect tick.
  // Attachments are part of the draft snapshot and must be declared before
  // the draft persistence hook.
  const attachInputRef = useRef<HTMLInputElement>(null);
  const [attachments, setAttachments] = useState<ComposeAttachment[]>(() =>
    restoredDraft.current?.attachments ?? [],
  );
  const attachmentsRef = useRef(attachments);
  const stagedAttachmentPathsRef = useRef(new Set(
    restoredDraft.current?.attachments.map((attachment) => attachment.path) ?? [],
  ));
  const stagedCleanupPromisesRef = useRef(new Map<string, Promise<void>>());
  const pendingAttachmentStagingRef = useRef(new Set<Promise<void>>());
  const acceptsStagedAttachmentsRef = useRef(true);
  const [pendingAttachmentStagingCount, setPendingAttachmentStagingCount] = useState(0);
  const [isDragging, setIsDragging] = useState(false);

  const [editorReady, setEditorReady] = useState(false);
  useEffect(() => {
    if (editor && !editorReady) setEditorReady(true);
  }, [editor, editorReady]);
  const { cancelPendingDraftSaves, flushDraftSave, resumeDraftSaves, draftIdsByAccountRef } = useComposeDraft({
    to, cc, bcc, subject, rawSource, richTextHtml, editorMode, composeMode, fromAccountId,
    attachments,
    editorReady,
    migrateLegacyDraft: Boolean(restoredDraft.current && !composeDraft),
    existingDraftId: composeDraft?.id,
    inReplyTo: composeDraft?.inReplyTo ?? (isReply ? composeReplyTo?.message_id_header ?? undefined : undefined),
  });

  // ─── Templates ───────────────────────────────────────────────────────────────
  const [showTemplates, setShowTemplates] = useState(false);
  const [templates, setTemplates] = useState<EmailTemplate[]>([]);
  const [showSaveTemplate, setShowSaveTemplate] = useState(false);
  const [templateName, setTemplateName] = useState("");
  const [showQuotedReply, setShowQuotedReply] = useState(false);

  async function refreshTemplates() {
    try {
      const next = await listTemplates();
      setTemplates(next);
      return next;
    } catch (err) {
      console.warn("Failed to load templates:", err);
      return [];
    }
  }

  function commitStagedAttachments(staged: ComposeAttachment[]) {
    if (staged.length === 0) return;

    const currentAttachments = attachmentsRef.current;
    const nextAttachments = [...currentAttachments, ...staged];
    attachmentsRef.current = nextAttachments;
    setSubject((currentSubject) => {
      const nextSubject = subjectAfterAddingAttachments(
        currentSubject,
        currentAttachments.length,
        staged.map((attachment) => attachment.name),
      );
      subjectRef.current = nextSubject;
      return nextSubject;
    });
    setAttachments(nextAttachments);
  }

  function removeAttachment(attachmentToRemove: ComposeAttachment) {
    const nextAttachments = attachmentsRef.current.filter(
      (attachment) => attachment !== attachmentToRemove,
    );
    attachmentsRef.current = nextAttachments;
    setAttachments(nextAttachments);
    void cleanupStagedAttachment(attachmentToRemove.path).catch((err) => {
      console.warn("Failed to remove staged compose attachment:", err);
    });
  }

  function cleanupStagedAttachment(path: string): Promise<void> {
    if (!stagedAttachmentPathsRef.current.has(path)) return Promise.resolve();
    const existingCleanup = stagedCleanupPromisesRef.current.get(path);
    if (existingCleanup) return existingCleanup;
    const cleanup = cleanupStagedComposeAttachment(path)
      .then(() => {
        stagedAttachmentPathsRef.current.delete(path);
      })
      .finally(() => {
        stagedCleanupPromisesRef.current.delete(path);
      });
    stagedCleanupPromisesRef.current.set(path, cleanup);
    return cleanup;
  }

  async function cleanupAllStagedAttachments() {
    const cleanupResults = await Promise.allSettled(
      Array.from(stagedAttachmentPathsRef.current, cleanupStagedAttachment),
    );
    for (const result of cleanupResults) {
      if (result.status === "rejected") {
        console.warn("Failed to remove staged compose attachment:", result.reason);
      }
    }
  }

  async function waitForPendingAttachmentStaging() {
    while (pendingAttachmentStagingRef.current.size > 0) {
      await Promise.allSettled(Array.from(pendingAttachmentStagingRef.current));
    }
  }

  useEffect(() => {
    acceptsStagedAttachmentsRef.current = true;
    return () => {
      acceptsStagedAttachmentsRef.current = false;
    };
  }, []);

  async function cleanupSavedDrafts(context: "send" | "discard") {
    const draftIdsToDelete = await cancelPendingDraftSaves();
    const deletionResults = await Promise.allSettled(
      Object.entries(draftIdsToDelete).map(([draftAccountId, draftId]) =>
        deleteDraft(draftAccountId, draftId),
      ),
    );
    Object.entries(draftIdsToDelete).forEach(([accountId, draftId], index) => {
      if (deletionResults[index].status === "fulfilled" && draftIdsByAccountRef.current[accountId] === draftId) {
        delete draftIdsByAccountRef.current[accountId];
      }
    });
    void queryClient.invalidateQueries({ queryKey: ["messages"] });
    void queryClient.invalidateQueries({ queryKey: ["threads"] });
    const failed = deletionResults.some((result) => result.status === "rejected");
    if (failed) {
      for (const result of deletionResults) {
        if (result.status === "rejected") {
          console.warn(`Failed to delete draft after ${context}:`, result.reason);
        }
      }
      useToastStore.getState().addToast({
        type: "error",
        message: context === "send"
          ? t(
              "compose.draftCleanupFailed",
              "Sent, but failed to remove the saved draft. You can delete it from Drafts.",
            )
          : t(
              "compose.discardFailed",
              "Could not remove the saved draft. Your draft is still open; please try again.",
            ),
      });
    }
    return !failed;
  }

  async function stageAttachmentFiles(files: FileList | File[]) {
    const staging = (async () => {
      const staged: ComposeAttachment[] = [];
      try {
        for (const file of Array.from(files)) {
          const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
          if (!acceptsStagedAttachmentsRef.current) return;
          const path = await stageComposeAttachment(file.name, bytes);
          stagedAttachmentPathsRef.current.add(path);
          if (!acceptsStagedAttachmentsRef.current) {
            await cleanupStagedAttachment(path);
            return;
          }
          staged.push({ name: file.name, path, size: file.size });
        }
      } finally {
        if (acceptsStagedAttachmentsRef.current) {
          commitStagedAttachments(staged);
        } else {
          await Promise.allSettled(staged.map((attachment) =>
            cleanupStagedAttachment(attachment.path)));
        }
      }
    })();

    pendingAttachmentStagingRef.current.add(staging);
    setPendingAttachmentStagingCount(pendingAttachmentStagingRef.current.size);
    try {
      await staging;
    } finally {
      pendingAttachmentStagingRef.current.delete(staging);
      if (acceptsStagedAttachmentsRef.current) {
        setPendingAttachmentStagingCount(pendingAttachmentStagingRef.current.size);
      }
    }
  }

  async function handleSaveTemplate() {
    if (!templateName.trim()) return;
    const bodyContent = editorMode === "rich" && editor ? editor.getHTML() : rawSource;
    await saveTemplate({ name: templateName.trim(), subject, body: bodyContent });
    setTemplateName("");
    setShowSaveTemplate(false);
    void refreshTemplates();
  }

  useEffect(() => {
    if (!sendError) return;
    const timer = setTimeout(() => setSendError(null), 5000);
    return () => clearTimeout(timer);
  }, [sendError]);

  async function handleSend() {
    if (sendInProgressRef.current || discardInProgressRef.current) return;
    const finalTo = mergePendingRecipient(to, toInputValue).filter(Boolean);
    const finalCc = mergePendingRecipient(cc, ccInputValue).filter(Boolean);
    const finalBcc = mergePendingRecipient(bcc, bccInputValue).filter(Boolean);

    if (!fromAccountId || finalTo.length === 0) return;

    sendInProgressRef.current = true;
    setSendInProgress(true);
    if (pendingAttachmentStagingRef.current.size > 0) {
      await waitForPendingAttachmentStaging();
    }
    if (!acceptsStagedAttachmentsRef.current) return;
    const finalSubject = subjectRef.current;

    setTo(finalTo);
    setCc(finalCc);
    setBcc(finalBcc);
    if (isValidEmailAddress(toInputValue)) setToInputValue("");
    if (isValidEmailAddress(ccInputValue)) setCcInputValue("");
    if (isValidEmailAddress(bccInputValue)) setBccInputValue("");

    if (!finalSubject.trim()) {
      const confirmed = await useConfirmStore.getState().confirm({
        title: t("compose.noSubjectTitle", "No subject"),
        message: t("compose.noSubjectConfirm", "Send without a subject?"),
        confirmLabel: t("common.send", "Send"),
      });
      if (!confirmed) {
        sendInProgressRef.current = false;
        setSendInProgress(false);
        return;
      }
    }

    setSendError(null);

    let bodyHtml = "";
    let bodyText = "";

    if (editorMode === "rich" && editor) {
      bodyHtml = editor.getHTML();
      bodyText = editor.getText();
    } else if (editorMode === "html") {
      bodyHtml = rawSource;
      const tmp = document.createElement("div");
      tmp.innerHTML = rawSource;
      bodyText = tmp.textContent || tmp.innerText || "";
    } else {
      if (editor) {
        editor.commands.setContent(rawSource);
        bodyHtml = editor.getHTML();
        bodyText = rawSource;
      }
    }
    const outgoingBodyHtml = appendReplyQuoteHtml(bodyHtml, quotedReplyHtml);

    const inReplyTo = composeDraft?.inReplyTo ?? (
      isReply && composeReplyTo?.message_id_header
        ? composeReplyTo.message_id_header
        : undefined);

    sendMutation.mutate(
      {
        accountId: fromAccountId,
        to: finalTo,
        cc: finalCc,
        bcc: finalBcc,
        subject: finalSubject,
        bodyText,
        bodyHtml: outgoingBodyHtml || undefined,
        inReplyTo: inReplyTo || undefined,
        attachmentPaths: attachmentsRef.current.length > 0
          ? attachmentsRef.current.map((attachment) => attachment.path)
          : undefined,
      },
      {
        onSuccess: () => {
          void (async () => {
            try {
              await cleanupSavedDrafts("send");
              await cleanupAllStagedAttachments();
            } finally {
              clearDraftStorage();
              useComposeStore.getState().setComposeDirty(false);
              closeCompose();
            }
          })();
        },
        onError: (e) => {
          sendInProgressRef.current = false;
          setSendInProgress(false);
          const msg = e instanceof Error ? e.message : String(e);
          setSendError(msg || t("compose.sendError", "Failed to send"));
        },
      },
    );
  }

  async function handleDiscard() {
    if (discardInProgressRef.current || sendInProgressRef.current) return;
    discardInProgressRef.current = true;
    setIsDiscarding(true);
    if (pendingAttachmentStagingRef.current.size > 0) {
      await waitForPendingAttachmentStaging();
    }
    if (!acceptsStagedAttachmentsRef.current) return;
    if (!(await cleanupSavedDrafts("discard"))) {
      resumeDraftSaves();
      discardInProgressRef.current = false;
      setIsDiscarding(false);
      return;
    }
    await cleanupAllStagedAttachments();
    clearDraftStorage();
    useComposeStore.getState().setComposeDirty(false);
    discardInProgressRef.current = false;
    setIsDiscarding(false);
    confirmCloseCompose();
  }

  async function handleSaveAndClose() {
    if (isSaving || discardInProgressRef.current || sendInProgressRef.current) return;
    setIsSaving(true);
    try {
      await flushDraftSave({
        to: mergePendingRecipient(to, toInputValue),
        cc: mergePendingRecipient(cc, ccInputValue),
        bcc: mergePendingRecipient(bcc, bccInputValue),
      });
      await cancelPendingDraftSaves();
      await cleanupAllStagedAttachments();
      clearDraftStorage();
      useComposeStore.getState().setComposeDirty(false);
      confirmCloseCompose();
    } catch (error) {
      useToastStore.getState().addToast({ type: "error", message: String(error) });
    } finally {
      setIsSaving(false);
    }
  }

  const title =
    composeMode === "reply"
      ? t("compose.reply", "Reply")
      : composeMode === "reply-all"
        ? t("compose.replyAll", "Reply All")
        : composeMode === "forward"
          ? t("compose.forward", "Forward")
          : t("compose.newMessage", "New Message");
  const hasToRecipient = to.some(Boolean) || isValidEmailAddress(toInputValue);
  const sendDisabled = sendInProgress
    || sendMutation.isPending
    || pendingAttachmentStagingCount > 0
    || !fromAccountId
    || !hasToRecipient;

  return (
    <div
      aria-busy={sendInProgress}
      inert={sendInProgress}
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        pointerEvents: sendInProgress ? "none" : undefined,
      }}
    >
      {/* Header */}
      <div
        style={{
          display: "flex", alignItems: "center", justifyContent: "space-between",
          padding: "10px 20px",
          borderBottom: "1px solid var(--color-border)",
          flexShrink: 0,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "12px" }}>
          <button
            onClick={closeCompose}
            style={composeStyles.backBtn}
            onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = "var(--color-bg-hover, rgba(0,0,0,0.04))"; }}
            onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = "transparent"; }}
          >
            <ArrowLeft size={16} />
            {t("compose.back", "Back")}
          </button>
          <span style={{ fontWeight: 600, fontSize: "15px", color: "var(--color-text-primary)" }}>
            {title}
          </span>
        </div>
        <button
          onClick={handleSend}
          disabled={sendDisabled}
          style={{
            display: "flex", alignItems: "center", gap: "6px",
            padding: "7px 20px",
            backgroundColor: sendInProgress || sendMutation.isPending ? "var(--color-text-secondary)" : "var(--color-accent, #2563eb)",
            color: "#fff", border: "none", borderRadius: "6px",
            cursor: sendDisabled ? "default" : "pointer",
            opacity: hasToRecipient ? 1 : 0.5,
            fontSize: "13px", fontWeight: 500,
          }}
        >
          <Send size={14} />
          {sendInProgress || sendMutation.isPending ? t("compose.sending", "Sending...") : t("compose.send", "Send")}
        </button>
      </div>

      {/* Error banner */}
      {sendError && (
        <div role="alert" aria-live="assertive" style={{
          display: "flex", alignItems: "center", gap: "8px",
          padding: "8px 20px",
          backgroundColor: "var(--color-error-bg, #fef2f2)",
          color: "var(--color-error, #dc2626)",
          fontSize: "13px",
          borderBottom: "1px solid var(--color-border)",
        }}>
          <AlertCircle size={14} />
          <span style={{ flex: 1 }}>{sendError}</span>
          <button
            onClick={() => setSendError(null)}
            aria-label={t("common.close", "Close")}
            style={{ background: "none", border: "none", cursor: "pointer", color: "inherit", display: "flex" }}
          >
            <X size={14} />
          </button>
        </div>
      )}

      {/* Fields + Editor */}
      <div className="scroll-region compose-scroll" style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, overflow: "auto" }}>
        <div className="compose-form-shell">
          {/* From */}
          {accounts.length > 0 && (
            <div style={composeStyles.fieldRow}>
              <label htmlFor="compose-from-account" style={composeStyles.fieldLabel}>
                {t("compose.from", "From")}
              </label>
              <select
                id="compose-from-account"
                name="from"
                value={fromAccountId}
                onChange={(e) => setFromAccountId(e.target.value)}
                style={{
                  flex: 1, padding: "6px 0", border: "none",
                  backgroundColor: "transparent", fontSize: "13px",
                  color: "var(--color-text-primary)", cursor: "pointer",
                }}
              >
                {accounts.map((acc) => (
                  <option key={acc.id} value={acc.id}>
                    {senderIdentityLabel(acc)}{acc.account_label?.trim() ? ` · ${acc.account_label.trim()}` : ""}
                  </option>
                ))}
              </select>
            </div>
          )}

          {accounts.find((acc) => acc.id === fromAccountId)?.provider === "outlook" && (
            <p style={{ margin: "0 0 8px", fontSize: "12px", color: "var(--color-text-secondary)" }}>
              {t("accountSetup.providerManagedName", "The sender name is managed by your email provider.")}
            </p>
          )}

          {/* To */}
          <div style={composeStyles.fieldRow}>
            <label id="compose-to-label" htmlFor="compose-to-input" style={composeStyles.fieldLabel}>
              {t("compose.to", "To")}
            </label>
            <ContactAutocomplete
              id="compose-to-input"
              name="to"
              ariaLabelledBy="compose-to-label"
              value={to}
              onChange={setTo}
              accountId={fromAccountId}
              inputValue={toInputValue}
              onInputValueChange={setToInputValue}
              placeholder="recipient@example.com"
            />
            <div style={{ display: "flex", gap: "4px", padding: "0 8px", flexShrink: 0 }}>
              {!showCc && <button onClick={() => setShowCc(true)} style={composeStyles.toggleBtn}>{t("compose.cc", "Cc")}</button>}
              {!showBcc && <button onClick={() => setShowBcc(true)} style={composeStyles.toggleBtn}>{t("compose.bcc", "Bcc")}</button>}
            </div>
          </div>

          {showCc && (
            <div style={composeStyles.fieldRow}>
              <label id="compose-cc-label" htmlFor="compose-cc-input" style={composeStyles.fieldLabel}>
                {t("compose.cc", "Cc")}
              </label>
              <ContactAutocomplete
                id="compose-cc-input"
                name="cc"
                ariaLabelledBy="compose-cc-label"
                value={cc}
                onChange={setCc}
                accountId={fromAccountId}
                inputValue={ccInputValue}
                onInputValueChange={setCcInputValue}
                placeholder="cc@example.com"
              />
            </div>
          )}

          {showBcc && (
            <div style={composeStyles.fieldRow}>
              <label id="compose-bcc-label" htmlFor="compose-bcc-input" style={composeStyles.fieldLabel}>
                {t("compose.bcc", "Bcc")}
              </label>
              <ContactAutocomplete
                id="compose-bcc-input"
                name="bcc"
                ariaLabelledBy="compose-bcc-label"
                value={bcc}
                onChange={setBcc}
                accountId={fromAccountId}
                inputValue={bccInputValue}
                onInputValueChange={setBccInputValue}
                placeholder="bcc@example.com"
              />
            </div>
          )}

          {/* Subject */}
          <div style={composeStyles.fieldRow}>
            <label htmlFor="compose-subject" style={composeStyles.fieldLabel}>
              {t("compose.subject", "Subject")}
            </label>
              <input
                id="compose-subject"
                name="subject"
                type="text" value={subject} onChange={(e) => {
                  subjectRef.current = e.target.value;
                  setSubject(e.target.value);
                }}
                placeholder=""
                autoComplete="off"
                style={{ flex: 1, padding: "8px 0", border: "none", backgroundColor: "transparent", fontSize: "13px", color: "var(--color-text-primary)" }}
              />
          </div>

          {/* Toolbar */}
          <div className="compose-editor-toolbar-row">
            <div className="compose-toolbar-tools">
              <div className="compose-toolbar-actions">
                <button
                  type="button"
                  onClick={() => attachInputRef.current?.click()}
                  title={t("compose.attach", "Attach file")}
                  aria-label={t("compose.attach", "Attach file")}
                  className="compose-toolbar-icon-button"
                >
                  <Paperclip size={13} />
                </button>
                <input
                  ref={attachInputRef}
                  type="file"
                  multiple
                  style={{ position: "absolute", width: 1, height: 1, padding: 0, margin: -1, overflow: "hidden", clip: "rect(0,0,0,0)", border: 0 }}
                  tabIndex={-1}
                  aria-hidden="true"
                  onChange={(e) => {
                    const files = e.target.files;
                    if (!files) return;
                    void stageAttachmentFiles(files).catch((err) => {
                      console.warn("Failed to stage attachment:", err);
                      if (acceptsStagedAttachmentsRef.current) {
                        setSendError(t("compose.attachmentStageError", "Failed to attach file"));
                      }
                    });
                    e.target.value = "";
                  }}
                />
                <div style={{ position: "relative" }}>
                  <button
                    type="button"
                    onClick={() => { void refreshTemplates(); setShowTemplates((v) => !v); }}
                    aria-haspopup="listbox"
                    aria-expanded={showTemplates}
                    aria-label={t("compose.templates", "Templates")}
                    title={t("compose.templates", "Templates")}
                    className={`compose-toolbar-icon-button${showTemplates ? " is-active" : ""}`}
                  >
                    <BookTemplate size={13} />
                  </button>
                  {showTemplates && (
                    <div className="scroll-region compose-template-scroll" style={{
                      position: "absolute", top: "100%", left: 0, zIndex: 100,
                      backgroundColor: "var(--color-bg)", border: "1px solid var(--color-border)",
                      borderRadius: "8px", boxShadow: "0 8px 24px rgba(0,0,0,0.12)",
                      minWidth: "220px", maxHeight: "300px", overflowY: "auto",
                    }}>
                      <div style={{ padding: "8px", borderBottom: "1px solid var(--color-border)", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                        <span id="compose-templates-label" style={{ fontSize: "12px", fontWeight: 600 }}>{t("compose.templates", "Templates")}</span>
                        <button
                          type="button"
                          onClick={() => { setShowSaveTemplate(true); setShowTemplates(false); }}
                          style={{ fontSize: "11px", border: "none", background: "none", cursor: "pointer", color: "var(--color-accent)" }}
                        >
                          {t("compose.saveAsTemplate", "Save current")}
                        </button>
                      </div>
                      {templates.length === 0 ? (
                        <div style={{ padding: "16px", textAlign: "center", fontSize: "12px", color: "var(--color-text-secondary)" }}>
                          {t("compose.noTemplates", "No templates saved")}
                        </div>
                      ) : (
                        <ul
                          role="listbox"
                          aria-labelledby="compose-templates-label"
                          style={{ listStyle: "none", margin: 0, padding: 0 }}
                        >
                          {templates.map((tpl) => {
                            const applyTemplate = () => {
                              subjectRef.current = tpl.subject;
                              setSubject(tpl.subject);
                              setRawSource(tpl.body);
                              if (editor) editor.commands.setContent(tpl.body);
                              setShowTemplates(false);
                            };
                            return (
                              <li
                                key={tpl.id}
                                role="option"
                                aria-selected={false}
                                tabIndex={0}
                                onClick={applyTemplate}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter" || e.key === " ") {
                                    e.preventDefault();
                                    applyTemplate();
                                  }
                                }}
                                style={{
                                  display: "flex", alignItems: "center", padding: "8px",
                                  borderBottom: "1px solid var(--color-border)", cursor: "pointer",
                                  fontSize: "12px",
                                }}
                              >
                                <div style={{ flex: 1, overflow: "hidden" }}>
                                  <div style={{ fontWeight: 500 }}>{tpl.name}</div>
                                  <div style={{ color: "var(--color-text-secondary)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{tpl.subject}</div>
                                </div>
                                <button
                                  type="button"
                                  onClick={async (e) => {
                                    e.stopPropagation();
                                    const confirmed = await useConfirmStore.getState().confirm({
                                      title: t("compose.deleteTemplate", "Delete template"),
                                      message: t("compose.deleteTemplate", "Delete template") + ` "${tpl.name}"?`,
                                      destructive: true,
                                    });
                                    if (confirmed) {
                                      await deleteTemplate(tpl.id);
                                      void refreshTemplates();
                                    }
                                  }}
                                  aria-label={t("compose.deleteTemplate", "Delete template")}
                                  title={t("compose.deleteTemplate", "Delete template")}
                                  style={{ border: "none", background: "none", cursor: "pointer", color: "var(--color-text-secondary)", padding: "2px" }}
                                >
                                  <Trash2 size={12} />
                                </button>
                              </li>
                            );
                          })}
                        </ul>
                      )}
                    </div>
                  )}
                </div>
              </div>
              <div className="compose-toolbar-divider" />

              {/* Formatting toolbar */}
              <div className="compose-format-toolbar-slot">
                {editorMode === "rich" && editor && (
                  <EditorToolbar editor={editor} />
                )}
                {editorMode === "markdown" && (
                  <MarkdownToolbar textareaRef={textareaRef} onInsert={setRawSource} source={rawSource} />
                )}
                {editorMode === "html" && (
                  <button
                    type="button"
                    onClick={() => setHtmlPreview((v) => !v)}
                    title={htmlPreview ? t("compose.mode.hidePreview", "Hide preview") : t("compose.mode.showPreview", "Show preview")}
                    className={`compose-toolbar-text-button${htmlPreview ? " is-active" : ""}`}
                  >
                    {htmlPreview ? <EyeOff size={13} /> : <Eye size={13} />}
                    {htmlPreview ? t("compose.mode.hidePreview", "Hide preview") : t("compose.mode.showPreview", "Show preview")}
                  </button>
                )}
              </div>
            </div>

            {/* Mode tabs */}
            <div className="compose-mode-tabs" role="group" aria-label={t("compose.mode.label", "Editor mode")}>
              <ModeButton icon={Type} label={t("compose.mode.rich", "Rich Text")} active={editorMode === "rich"} onClick={() => switchMode("rich")} />
              <ModeButton icon={Hash} label={t("compose.mode.markdown", "Markdown")} active={editorMode === "markdown"} onClick={() => switchMode("markdown")} />
              <ModeButton icon={FileCode2} label={t("compose.mode.html", "HTML")} active={editorMode === "html"} onClick={() => switchMode("html")} />
            </div>
          </div>

          {/* Attachment list */}
          {attachments.length > 0 && (
            <div className="compose-inline-panel">
              {attachments.map((att, i) => (
                <div key={i} style={{
                  display: "flex", alignItems: "center", gap: "4px",
                  padding: "4px 8px", borderRadius: "4px",
                  backgroundColor: "var(--color-bg-hover)", fontSize: "12px",
                }}>
                  <FileText size={12} />
                  <span style={{ maxWidth: "150px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{att.name}</span>
                  <span style={{ color: "var(--color-text-secondary)", fontSize: "11px" }}>
                    {att.size < 1024 * 1024 ? `${(att.size / 1024).toFixed(0)} KB` : `${(att.size / (1024 * 1024)).toFixed(1)} MB`}
                  </span>
                  <button
                    type="button"
                    onClick={() => removeAttachment(att)}
                    aria-label={t("compose.removeAttachment", "Remove attachment {{name}}", { name: att.name })}
                    title={t("compose.removeAttachment", "Remove attachment {{name}}", { name: att.name })}
                    style={{ border: "none", background: "none", cursor: "pointer", padding: "0 2px", color: "var(--color-text-secondary)" }}
                  >
                    <X size={12} />
                  </button>
                </div>
              ))}
            </div>
          )}

          {/* Save template dialog */}
          {showSaveTemplate && (
            <div className="compose-inline-panel">
              <input
                type="text" value={templateName} onChange={(e) => setTemplateName(e.target.value)}
                placeholder={t("compose.templateName", "Template name")}
                autoFocus
                style={{
                  flex: 1, padding: "6px 8px", fontSize: "12px",
                  border: "1px solid var(--color-border)", borderRadius: "4px",
                  backgroundColor: "var(--color-bg)", color: "var(--color-text-primary)",
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && templateName.trim()) {
                    void handleSaveTemplate();
                  }
                  if (e.key === "Escape") setShowSaveTemplate(false);
                }}
              />
              <button
                type="button"
                onClick={() => {
                  if (!templateName.trim()) return;
                  void handleSaveTemplate();
                }}
                style={{
                  padding: "5px 12px", fontSize: "12px", border: "none",
                  borderRadius: "4px", backgroundColor: "var(--color-accent)",
                  color: "#fff", cursor: "pointer",
                }}
              >
                {t("common.save")}
              </button>
              <button
                type="button"
                onClick={() => setShowSaveTemplate(false)}
                style={{
                  padding: "5px 8px", fontSize: "12px", border: "1px solid var(--color-border)",
                  borderRadius: "4px", backgroundColor: "transparent",
                  color: "var(--color-text-secondary)", cursor: "pointer",
                }}
              >
                {t("common.cancel")}
              </button>
            </div>
          )}

          {/* Editor area */}
          <div
            className="compose-editor-area"
            onDragOver={(e) => { e.preventDefault(); setIsDragging(true); }}
            onDragLeave={() => setIsDragging(false)}
            onDrop={async (e) => {
              e.preventDefault();
              setIsDragging(false);
              const files = e.dataTransfer.files;
              if (!files.length) return;
              try {
                await stageAttachmentFiles(files);
              } catch (err) {
                console.warn("Failed to stage dropped attachment:", err);
                if (acceptsStagedAttachmentsRef.current) {
                  setSendError(t("compose.attachmentStageError", "Failed to attach file"));
                }
              }
            }}
            onPaste={async (e) => {
              const items = e.clipboardData?.items;
              if (!items) return;
              const imageFiles: File[] = [];
              for (const item of Array.from(items)) {
                if (item.type.startsWith("image/")) {
                  const file = item.getAsFile();
                  if (file) imageFiles.push(file);
                }
              }
              if (!imageFiles.length) return;
              e.preventDefault();
              try {
                await stageAttachmentFiles(imageFiles);
              } catch (err) {
                console.warn("Failed to stage pasted image:", err);
                if (acceptsStagedAttachmentsRef.current) {
                  setSendError(t("compose.attachmentStageError", "Failed to attach file"));
                }
              }
            }}
          >
            <div className="compose-editor-surface">
              {editorMode === "rich" ? (
                <EditorContent
                  editor={editor}
                  className="compose-editor-content"
                />
              ) : editorMode === "html" && htmlPreview ? (
                <div className="compose-preview-split">
                  <textarea
                    ref={textareaRef}
                    value={rawSource}
                    onChange={(e) => setRawSource(e.target.value)}
                    placeholder={t("compose.mode.htmlPlaceholder", "Write HTML source...")}
                    spellCheck={false}
                    className="compose-source-input compose-source-input--split is-code"
                  />
                  <iframe
                    sandbox="allow-same-origin"
                    srcDoc={rawSource}
                    title={t("compose.mode.preview", "Preview")}
                    className="compose-html-preview"
                  />
                </div>
              ) : (
                <textarea
                  ref={textareaRef}
                  value={rawSource}
                  onChange={(e) => setRawSource(e.target.value)}
                  placeholder={editorMode === "markdown"
                    ? t("compose.mode.markdownPlaceholder", "Write in Markdown...")
                    : t("compose.mode.htmlPlaceholder", "Write HTML source...")}
                  spellCheck={editorMode === "markdown"}
                  className={`compose-source-input${editorMode === "html" ? " is-code" : ""}`}
                />
              )}
            </div>
            {quotedReplyHtml && (
              <div className="compose-quoted-reply">
                <button
                  type="button"
                  className="compose-quoted-reply-toggle"
                  aria-expanded={showQuotedReply}
                  onClick={() => setShowQuotedReply((value) => !value)}
                >
                  {showQuotedReply ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  {showQuotedReply
                    ? t("compose.hideQuotedReply", "Hide quoted message")
                    : t("compose.showQuotedReply", "Show quoted message")}
                </button>
                {showQuotedReply && (
                  <div
                    className="scroll-region compose-quoted-reply-content"
                    dangerouslySetInnerHTML={{ __html: quotedReplyHtml }}
                  />
                )}
              </div>
            )}
            {isDragging && (
              <div className="compose-drop-overlay">
                <Paperclip size={20} />
                {t("compose.dropFiles", "Drop files to attach")}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Compose leave confirmation dialog */}
      {showComposeLeaveConfirm && (
        <ConfirmDialog
          title={t("compose.leaveTitle", "Discard draft?")}
          message={t("compose.leaveMessage", "You have unsaved changes. Are you sure you want to leave?")}
          cancelLabel={t("compose.leaveCancel", "Keep editing")}
          confirmLabel={t("compose.leaveConfirm", "Discard")}
          busyConfirmLabel={isSaving ? t("common.saving", "Saving...") : t("compose.discarding", "Discarding...")}
          destructive
          busy={isDiscarding || isSaving}
          extraActionLabel={t("compose.saveAndClose", "Save and close")}
          onExtraAction={() => { void handleSaveAndClose(); }}
          confirmDisabled={pendingAttachmentStagingCount > 0}
          onCancel={() => {
            if (!discardInProgressRef.current) cancelCloseCompose();
          }}
          onConfirm={() => { void handleDiscard(); }}
        />
      )}
    </div>
  );
}
