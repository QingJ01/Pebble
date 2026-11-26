import { senderIdentityLabel } from "@/lib/accountIdentity";
import { useState, useEffect, useRef } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import Popover from "@/components/ui/Popover";
import { EditorContent } from "@tiptap/react";
import {
  ArrowLeft, Send, X, AlertCircle, Clock,
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
import { toastError, useToastStore } from "@/stores/toast.store";
import { formatError } from "@/lib/extractErrorMessage";
import type { Account } from "@/lib/ipc-types";
import type { ComposeAttachment } from "./compose-draft";
import { ModeButton, EditorToolbar, MarkdownToolbar } from "./ComposeToolbar";
import { subjectAfterAddingAttachments } from "./attachment-subject";
import { isValidEmailAddress, mergePendingRecipient } from "./recipient-utils";
import { queryClient } from "@/lib/query-client";
import { queueControlledSend, restoreControlledSend, deleteControlledSend, listControlledSends, type ControlledSend } from "@/lib/send-controls-api";
import { useControlledSends } from "./ScheduledSendsView";
import { countdownSeconds, scheduledInstant } from "./send-control-time";
import { loadUndoSeconds } from "./send-controls-settings";
import ScheduleTimeInput from "./ScheduleTimeInput";
import { useSendControlsStore } from "@/stores/send-controls.store";
import "./send-controls.css";
import { threadMessagesQueryKey } from "@/hooks/queries/useThreadMessagesQuery";

function modEnterLabel() {
  const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/i.test(navigator.platform || navigator.userAgent);
  return isMac ? "⌘↩" : "Ctrl+Enter";
}

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
  const [showSchedule, setShowSchedule] = useState(false);
  const [scheduleValue, setScheduleValue] = useState("");
  const [pendingSend, setPendingSend] = useState<ControlledSend | null>(null);
  const [secondsLeft, setSecondsLeft] = useState(0);
  const [undoBusy, setUndoBusy] = useState(false);
  const [reviewBeforeRetry, setReviewBeforeRetry] = useState(false);
  const [queueReplyUnknown, setQueueReplyUnknown] = useState(false);
  const sendRequestIdRef = useRef<string | null>(null);
  const controlledSends = useControlledSends(Boolean(pendingSend));

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
  const templatesButtonRef = useRef<HTMLButtonElement>(null);
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
  const [showQuotedReply, setShowQuotedReply] = useState(true);

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

  async function cleanupSavedDrafts(context: "send" | "schedule" | "discard") {
    const draftIdsToDelete = await cancelPendingDraftSaves();
    const deletionResults = await Promise.allSettled(
      Object.entries(draftIdsToDelete).map(([draftAccountId, draftId]) =>
        deleteDraft(draftAccountId, draftId),
      ),
    );
    const deletedIds = new Set<string>();
    Object.entries(draftIdsToDelete).forEach(([accountId, draftId], index) => {
      if (deletionResults[index].status === "fulfilled") {
        deletedIds.add(draftId);
        if (composeDraft?.id && composeDraft.accountId === accountId) {
          deletedIds.add(composeDraft.id);
        }
        if (draftIdsByAccountRef.current[accountId] === draftId) {
          delete draftIdsByAccountRef.current[accountId];
        }
      }
    });
    if (deletedIds.size > 0) {
      useMailStore.setState((state) => {
        const threadMessages = state.selectedThreadId
          ? queryClient.getQueryData<{ id: string }[]>(threadMessagesQueryKey(state.selectedThreadId))
          : undefined;
        const removedThread = deletedIds.has(state.selectedThreadId ?? "")
          || (threadMessages?.length ? threadMessages.every((message) => deletedIds.has(message.id)) : false);
        return {
          selectedMessageId: deletedIds.has(state.selectedMessageId ?? "") ? null : state.selectedMessageId,
          selectedThreadId: removedThread ? null : state.selectedThreadId,
          selectedMessageIds: new Set([...state.selectedMessageIds].filter((id) => !deletedIds.has(id))),
        };
      });
    }
    void queryClient.invalidateQueries({ queryKey: ["messages"] });
    void queryClient.invalidateQueries({ queryKey: ["threads"] });
    void queryClient.invalidateQueries({ queryKey: ["threadMessages"] });
    const failed = deletionResults.some((result) => result.status === "rejected");
    if (failed) {
      for (const result of deletionResults) {
        if (result.status === "rejected") {
          console.warn(`Failed to delete draft after ${context}:`, result.reason);
        }
      }
      useToastStore.getState().addToast({
        type: "error",
        message: context === "schedule" ? t("sendControls.draftCleanupFailed") : context === "send"
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
    useSendControlsStore.setState({ composeUndoId: pendingSend?.id ?? null });
    return () => useSendControlsStore.setState({ composeUndoId: null });
  }, [pendingSend?.id]);

  useEffect(() => {
    if (!pendingSend) return;
    const update = () => setSecondsLeft(countdownSeconds(pendingSend.dueAt));
    update();
    const timer = setInterval(update, 100);
    return () => clearInterval(timer);
  }, [pendingSend]);

  useEffect(() => {
    if (!pendingSend || undoBusy) return;
    const job = controlledSends.data?.find((item) => item.id === pendingSend.id);
    if (job?.state === "sent") {
      setPendingSend(null);
      void (async () => {
        // Native queue ownership cleans source drafts even after navigation/restart.
        await cleanupAllStagedAttachments();
        clearDraftStorage();
        useComposeStore.getState().setComposeDirty(false);
        closeCompose();
      })();
    } else if (job?.state === "review") {
      setPendingSend(null);
      setReviewBeforeRetry(true);
      setSendError(t("sendControls.reviewWarning"));
      sendInProgressRef.current = false;
      setSendInProgress(false);
      resumeDraftSaves();
    } else if (job?.state === "draft") {
      setPendingSend(null);
      sendRequestIdRef.current = null;
      sendInProgressRef.current = false;
      setSendInProgress(false);
      resumeDraftSaves();
    }
  }, [controlledSends.data, pendingSend, undoBusy]);

  async function handleUndo() {
    if (!pendingSend || undoBusy) return;
    setUndoBusy(true);
    try {
      const job = controlledSends.data?.find((item) => item.id === pendingSend.id) ?? pendingSend;
      const snapshot = await restoreControlledSend(job);
      await cleanupAllStagedAttachments();
      stagedAttachmentPathsRef.current = new Set(snapshot.attachments.map((a) => a.path));
      attachmentsRef.current = snapshot.attachments;
      setAttachments(snapshot.attachments);
      setPendingSend(null);
      sendInProgressRef.current = false;
      setSendInProgress(false);
      sendRequestIdRef.current = null;
      resumeDraftSaves();
      // Source drafts were never deleted; removing this cancelled backup is safe.
      void deleteControlledSend({ ...job, revision: job.revision + 1 }).catch(() => {});
    } catch {
      setSendError(t("sendControls.error"));
      void controlledSends.refetch();
    } finally { setUndoBusy(false); }
  }

  async function resolveQueueReply() {
    try {
      const jobs = await listControlledSends();
      const job = jobs.find((item) => item.id === sendRequestIdRef.current);
      setQueueReplyUnknown(false);
      if (job) {
        setPendingSend(job);
        // Use the reply itself when the query cache has not caught up yet.
        await queryClient.invalidateQueries({ queryKey: ["send-controls"] });
      } else {
        sendInProgressRef.current = false;
        setSendInProgress(false);
        resumeDraftSaves();
        setSendError(t("sendControls.error"));
      }
    } catch {
      // A lost IPC reply is not evidence that queueing failed. Keep the copy inert.
      setQueueReplyUnknown(true);
    }
  }

  async function handleSend(scheduleAt?: number) {
    if (sendInProgressRef.current || discardInProgressRef.current) return;
    if (reviewBeforeRetry) {
      if (!(await useConfirmStore.getState().confirm({ title: t("sendControls.review"), message: t("sendControls.restoreWarning") }))) return;
      try {
        const jobs = await listControlledSends();
        const job = jobs.find((item) => item.id === sendRequestIdRef.current);
        if (job?.state === "review") {
          const restored = await restoreControlledSend(job);
          await Promise.allSettled(restored.attachments.map((attachment) => cleanupStagedComposeAttachment(attachment.path)));
        } else if (job && job.state !== "draft") {
          setSendError(t("sendControls.error"));
          return;
        }
      } catch { setSendError(t("sendControls.error")); return; }
      setReviewBeforeRetry(false);
      sendRequestIdRef.current = null;
    }
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
      // Parse in an inert document: assigning innerHTML on a live-document
      // element would load images and fire inline handlers in the source.
      const parsed = new DOMParser().parseFromString(rawSource, "text/html");
      bodyText = parsed.body.textContent || "";
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

    const undoSeconds = loadUndoSeconds();
    if (scheduleAt !== undefined || undoSeconds > 0) {
      try {
        // Complete autosave first, but do not delete the source draft during Undo.
        await flushDraftSave({ to: finalTo, cc: finalCc, bcc: finalBcc });
        await cancelPendingDraftSaves();
        const requestId = sendRequestIdRef.current ??= crypto.randomUUID();
        const job = await queueControlledSend({
          sourceDrafts: Object.entries(draftIdsByAccountRef.current).map(([accountId, id]) => ({ accountId, id })),
          accountId: fromAccountId, to: finalTo, cc: finalCc, bcc: finalBcc,
          subject: finalSubject, bodyText, bodyHtml: outgoingBodyHtml || undefined,
          inReplyTo, editorMode, composeMode: composeMode ?? "new",
          rawSource, richTextHtml: editorMode === "rich" ? bodyHtml : richTextHtml,
          quotedReplyHtml, attachments: attachmentsRef.current,
        }, scheduleAt !== undefined ? { dueAt: scheduleAt } : { undoSeconds }, requestId);
        if (scheduleAt !== undefined) {
          // The independently encrypted scheduled copy now owns its attachment lifetime.
          // Source drafts stay protected by durable queue ownership until delivery/cancellation.
          await cleanupAllStagedAttachments();
          clearDraftStorage();
          useComposeStore.getState().setComposeDirty(false);
          useToastStore.getState().addToast({ message: t("sendControls.queued"), type: "success" });
          closeCompose();
        } else { setPendingSend(job); }
      } catch {
        setQueueReplyUnknown(true);
        await resolveQueueReply();
      }
      return;
    }

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
          setSendError(formatError(e, { fallbackKey: "compose.sendError", fallback: "Failed to send" }));
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
      toastError(error, "errors.saveDraft");
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
  const isSending = sendInProgress || sendMutation.isPending;
  const sendDisabledReason = isSending
    ? null
    : !fromAccountId
      ? t("compose.sendDisabledNoAccount", "Add an account to send mail")
      : pendingAttachmentStagingCount > 0
        ? t("compose.sendDisabledUploading", "Attachments are still uploading")
        : !hasToRecipient
          ? t("compose.sendDisabledNoRecipient", "Add a recipient")
          : null;
  const sendDisabled = isSending || sendDisabledReason !== null;

  // ─── Initial focus: "To" for a new message, the body for reply/forward ──────
  const initialFocusDoneRef = useRef(false);
  useEffect(() => {
    if (initialFocusDoneRef.current || !editorReady) return;
    initialFocusDoneRef.current = true;
    const frame = requestAnimationFrame(() => {
      // Respect focus the user already placed inside the form, and any open dialog.
      const active = document.activeElement;
      if (active && active !== document.body && active.closest("[data-compose-root], [role=dialog], [role=alertdialog]")) return;
      const wantsRecipient = !composeMode || composeMode === "new";
      if (wantsRecipient && to.filter(Boolean).length === 0) {
        document.getElementById("compose-to-input")?.focus();
        return;
      }
      if (wantsRecipient && !subjectRef.current.trim()) {
        document.getElementById("compose-subject")?.focus();
        return;
      }
      if (editorMode === "rich" && editor) {
        editor.commands.focus?.("start");
      } else {
        textareaRef.current?.focus();
        textareaRef.current?.setSelectionRange(0, 0);
      }
    });
    return () => cancelAnimationFrame(frame);
    // Runs once, as soon as the editor is ready.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorReady]);

  function handleComposeKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Enter" || !(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
    if (event.nativeEvent.isComposing) return;
    // Mod-Enter sends from anywhere in the form, including inside the editor
    // (capture phase so ProseMirror's own Mod-Enter binding never sees it).
    event.preventDefault();
    event.stopPropagation();
    if (showComposeLeaveConfirm) return;
    if (sendDisabled) {
      if (sendDisabledReason) setSendError(sendDisabledReason);
      return;
    }
    void handleSend();
  }

  return (
    <div
      data-compose-root=""
      aria-busy={sendInProgress}
      onKeyDownCapture={handleComposeKeyDown}
      className="compose-view"
    >
      {pendingSend && <div className="send-controls-banner" role="status">
        <span>{secondsLeft > 0 ? t("sendControls.countdown", { count: secondsLeft }) : t("sendControls.undoWaiting")}</span>
        <Button size="sm" variant="primary" loading={undoBusy} onClick={() => void handleUndo()}>{t("sendControls.undo")}</Button>
        <small>{t("sendControls.undoHint")}</small>
        {sendError && <p role="alert">{sendError}</p>}
      </div>}
      {queueReplyUnknown && <div role="alert" className="send-controls-banner">
        {t("sendControls.queueReplyUnknown")}
        <Button size="sm" onClick={() => void resolveQueueReply()}>{t("sendControls.refresh")}</Button>
      </div>}
      <div inert={sendInProgress} className="compose-view__body">
      {/* Header */}
      <div className="compose-header">
        <div className="compose-header__lead">
          <Button
            variant="ghost"
            size="sm"
            onClick={closeCompose}
            icon={<ArrowLeft size={14} />}
          >
            {t("compose.back", "Back")}
          </Button>
          <h2 className="compose-header__title">
            {title}
          </h2>
        </div>
        <div className="compose-header__actions">
          {sendDisabledReason && (
            <span id="compose-send-disabled-reason" className="compose-send-hint">
              {sendDisabledReason}
            </span>
          )}
          <Button
            variant="secondary"
            aria-expanded={showSchedule}
            onClick={() => setShowSchedule((value) => !value)}
            disabled={sendDisabled}
            aria-describedby={sendDisabledReason ? "compose-send-disabled-reason" : undefined}
            title={sendDisabledReason ?? undefined}
            icon={<Clock size={14} />}
          >
            {t("sendControls.schedule")}
          </Button>
          <Button
            variant="primary"
            onClick={() => void handleSend()}
            disabled={sendDisabled && !isSending}
            loading={isSending}
            aria-describedby={sendDisabledReason ? "compose-send-disabled-reason" : undefined}
            aria-keyshortcuts="Meta+Enter Control+Enter"
            title={sendDisabledReason ?? t("compose.sendShortcutHint", "Send ({{keys}})", { keys: modEnterLabel() })}
            icon={<Send size={14} />}
          >
            {isSending ? t("compose.sending", "Sending...") : t("compose.send", "Send")}
          </Button>
        </div>
      </div>

      {showSchedule && (
        <div className="send-controls-options">
          <ScheduleTimeInput value={scheduleValue} onChange={setScheduleValue} />
          <Button disabled={sendDisabled || scheduledInstant(scheduleValue) === null} onClick={() => {
            const instant = scheduledInstant(scheduleValue);
            if (instant === null) { setSendError(t("sendControls.invalidTime")); return; }
            void handleSend(instant);
          }}>{t("sendControls.schedule")}</Button>
          <small>{t("sendControls.policy")}</small>
        </div>
      )}

      {/* Error banner */}
      {sendError && (
        <div role="alert" aria-live="assertive" className="compose-error-banner">
          <AlertCircle size={14} aria-hidden="true" />
          <span className="compose-error-banner__text">{sendError}</span>
          <IconButton
            label={t("common.dismiss", "Dismiss")}
            icon={<X size={14} />}
            onClick={() => setSendError(null)}
            style={{ color: "inherit" }}
          />
        </div>
      )}

      {/* Fields + Editor */}
      <div className="scroll-region compose-scroll">
        <div className="compose-form-shell">
          {/* From */}
          {accounts.length > 0 && (
            <div className="compose-field-row">
              <label htmlFor="compose-from-account" className="compose-field-label">
                {t("compose.from", "From")}
              </label>
              <select
                id="compose-from-account"
                name="from"
                value={fromAccountId}
                onChange={(e) => setFromAccountId(e.target.value)}
                className="compose-field-control compose-field-control--select"
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
            <p className="compose-field-note">
              {t("accountSetup.providerManagedName", "The sender name is managed by your email provider.")}
            </p>
          )}

          {/* To */}
          <div className="compose-field-row">
            <label id="compose-to-label" htmlFor="compose-to-input" className="compose-field-label">
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
              placeholder={t("compose.toPlaceholder", "Name or email address")}
            />
            <div className="compose-field-row__actions">
              {!showCc && <Button variant="ghost" size="sm" onClick={() => setShowCc(true)}>{t("compose.cc", "Cc")}</Button>}
              {!showBcc && <Button variant="ghost" size="sm" onClick={() => setShowBcc(true)}>{t("compose.bcc", "Bcc")}</Button>}
            </div>
          </div>

          {showCc && (
            <div className="compose-field-row">
              <label id="compose-cc-label" htmlFor="compose-cc-input" className="compose-field-label">
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
            <div className="compose-field-row">
              <label id="compose-bcc-label" htmlFor="compose-bcc-input" className="compose-field-label">
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
          <div className="compose-field-row">
            <label htmlFor="compose-subject" className="compose-field-label">
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
                className="compose-field-control"
              />
          </div>

          {/* Toolbar */}
          <div className="compose-editor-toolbar-row">
            <div className="compose-toolbar-tools">
              <div className="compose-toolbar-actions">
                <IconButton
                  onClick={() => attachInputRef.current?.click()}
                  label={t("compose.attach", "Attach file")}
                  icon={<Paperclip size={14} />}
                />
                <input
                  ref={attachInputRef}
                  type="file"
                  multiple
                  className="sr-only"
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
                <IconButton
                  ref={templatesButtonRef}
                  onClick={() => { void refreshTemplates(); setShowTemplates((v) => !v); }}
                  aria-haspopup="listbox"
                  aria-expanded={showTemplates}
                  label={t("compose.templates", "Templates")}
                  icon={<BookTemplate size={14} />}
                />
                <Popover
                  open={showTemplates}
                  onClose={() => setShowTemplates(false)}
                  anchorRef={templatesButtonRef}
                  placement="bottom-start"
                  aria-labelledby="compose-templates-label"
                  className="menu compose-template-menu"
                >
                  <div className="compose-template-menu__header">
                    <span id="compose-templates-label" className="menu-label compose-template-menu__title">{t("compose.templates", "Templates")}</span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => { setShowSaveTemplate(true); setShowTemplates(false); }}
                    >
                      {t("compose.saveAsTemplate", "Save current")}
                    </Button>
                  </div>
                  <div className="menu-separator" />
                  {templates.length === 0 ? (
                    <div className="compose-template-menu__empty">
                      {t("compose.noTemplates", "No templates saved")}
                    </div>
                  ) : (
                    <ul
                      role="listbox"
                      aria-labelledby="compose-templates-label"
                      className="compose-template-menu__list"
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
                                className="menu-item compose-template-item"
                              >
                                <div className="compose-template-item__text">
                                  <div className="compose-template-item__name">{tpl.name}</div>
                                  <div className="compose-template-item__subject">{tpl.subject}</div>
                                </div>
                                <IconButton
                                  variant="danger"
                                  onKeyDown={(e) => e.stopPropagation()}
                                  onClick={async (e) => {
                                    e.stopPropagation();
                                    const confirmed = await useConfirmStore.getState().confirm({
                                      title: t("compose.deleteTemplate", "Delete template"),
                                      message: t("compose.deleteTemplateConfirm", "Delete template \"{{name}}\"?", { name: tpl.name }),
                                      confirmLabel: t("common.delete", "Delete"),
                                      destructive: true,
                                    });
                                    if (confirmed) {
                                      try {
                                        await deleteTemplate(tpl.id);
                                      } catch (err) {
                                        toastError(err);
                                      }
                                      void refreshTemplates();
                                    }
                                  }}
                                  label={t("compose.deleteTemplateNamed", "Delete template {{name}}", { name: tpl.name })}
                                  icon={<Trash2 size={14} />}
                                />
                              </li>
                            );
                      })}
                    </ul>
                  )}
                </Popover>
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
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setHtmlPreview((v) => !v)}
                    pressed={htmlPreview}
                    icon={htmlPreview ? <EyeOff size={14} /> : <Eye size={14} />}
                  >
                    {htmlPreview ? t("compose.mode.hidePreview", "Hide preview") : t("compose.mode.showPreview", "Show preview")}
                  </Button>
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
                <div key={i} className="chip chip--md compose-attachment-chip">
                  <FileText size={12} aria-hidden="true" />
                  <span className="compose-attachment-chip__name">{att.name}</span>
                  <span className="compose-attachment-chip__size">
                    {att.size < 1024 * 1024 ? `${(att.size / 1024).toFixed(0)} KB` : `${(att.size / (1024 * 1024)).toFixed(1)} MB`}
                  </span>
                  <button
                    type="button"
                    className="chip__remove"
                    onClick={() => removeAttachment(att)}
                    aria-label={t("compose.removeAttachment", "Remove attachment {{name}}", { name: att.name })}
                    title={t("compose.removeAttachment", "Remove attachment {{name}}", { name: att.name })}
                  >
                    <X size={11} aria-hidden="true" />
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
                className="input input--sm compose-template-name"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && templateName.trim()) {
                    void handleSaveTemplate();
                  }
                  if (e.key === "Escape") setShowSaveTemplate(false);
                }}
              />
              <Button
                variant="primary"
                size="sm"
                disabled={!templateName.trim()}
                onClick={() => {
                  if (!templateName.trim()) return;
                  void handleSaveTemplate();
                }}
              >
                {t("common.save")}
              </Button>
              <Button size="sm" onClick={() => setShowSaveTemplate(false)}>
                {t("common.cancel")}
              </Button>
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
            <div className={`compose-editor-surface${quotedReplyHtml ? " compose-editor-surface--with-quote" : ""}`}>
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
                    sandbox=""
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
              {quotedReplyHtml && (
                <div className="compose-quoted-reply">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="compose-quoted-reply-toggle"
                    aria-expanded={showQuotedReply}
                    onClick={() => setShowQuotedReply((value) => !value)}
                    icon={showQuotedReply ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  >
                    {showQuotedReply
                      ? t("compose.hideQuotedReply", "Hide quoted message")
                      : t("compose.showQuotedReply", "Show quoted message")}
                  </Button>
                  {showQuotedReply && (
                    <div
                      className="compose-quoted-reply-content"
                      dangerouslySetInnerHTML={{ __html: quotedReplyHtml }}
                    />
                  )}
                </div>
              )}
            </div>
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
    </div>
  );
}
