import { useState, useEffect, useRef } from "react";
import { EditorContent } from "@tiptap/react";
import {
  ArrowLeft, Send, X, AlertCircle,
  Type, FileCode2, Hash, Eye, EyeOff,
  Paperclip, FileText, Trash2, BookTemplate,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useMailStore } from "@/stores/mail.store";
import { useComposeStore } from "@/stores/compose.store";
import { useAccountsQuery } from "@/hooks/queries";
import { useSendEmailMutation } from "@/hooks/mutations";
import ContactAutocomplete from "@/components/ContactAutocomplete";
import { listTemplates, saveTemplate, deleteTemplate } from "@/lib/templates";
import type { EmailTemplate } from "@/lib/templates";
import { useComposeRecipients } from "@/hooks/useComposeRecipients";
import { useComposeDraft, loadDraftFromStorage, clearDraftStorage } from "@/hooks/useComposeDraft";
import { deleteDraft } from "@/lib/api";
import { useComposeEditor } from "@/hooks/useComposeEditor";
import { useConfirmStore } from "@/stores/confirm.store";
import { useToastStore } from "@/stores/toast.store";
import type { Account } from "@/lib/ipc-types";
import type { ComposeAttachment } from "./compose-draft";
import { ModeButton, EditorToolbar, MarkdownToolbar, composeStyles } from "./ComposeToolbar";

export default function ComposeView() {
  const composeMode = useComposeStore((s) => s.composeMode);
  const { data: accounts = [], isLoading } = useAccountsQuery();

  if (composeMode === "new" && isLoading) {
    return <div style={{ height: "100%" }} />;
  }

  return <ComposeViewInner accounts={accounts} />;
}

function ComposeViewInner({ accounts }: { accounts: Account[] }) {
  const { t } = useTranslation();
  const composeMode = useComposeStore((s) => s.composeMode);
  const composeReplyTo = useComposeStore((s) => s.composeReplyTo);
  const closeCompose = useComposeStore((s) => s.closeCompose);
  const showComposeLeaveConfirm = useComposeStore((s) => s.showComposeLeaveConfirm);
  const confirmCloseCompose = useComposeStore((s) => s.confirmCloseCompose);
  const cancelCloseCompose = useComposeStore((s) => s.cancelCloseCompose);
  const activeAccountId = useMailStore((s) => s.activeAccountId);

  const isReply = composeMode === "reply" || composeMode === "reply-all";
  const restoredDraft = useRef(
    composeMode === "new" ? loadDraftFromStorage(accounts.map((account) => account.id)) : null,
  );

  // ─── Recipients ──────────────────────────────────────────────────────────────
  const {
    fromAccountId, setFromAccountId,
    to, setTo, cc, setCc, bcc, setBcc,
    showCc, setShowCc, showBcc, setShowBcc,
  } = useComposeRecipients({
    composeMode, composeReplyTo, accounts, activeAccountId,
    restoredDraft: restoredDraft.current,
  });

  // ─── Subject ─────────────────────────────────────────────────────────────────
  const [subject, setSubject] = useState(() => {
    if (restoredDraft.current) return restoredDraft.current.subject;
    if (!composeReplyTo) return "";
    if (isReply) return `Re: ${composeReplyTo.subject.replace(/^(Re:\s*|Fwd:\s*)+/i, "")}`;
    if (composeMode === "forward") return `Fwd: ${composeReplyTo.subject.replace(/^(Re:\s*|Fwd:\s*)+/i, "")}`;
    return "";
  });
  const [sendError, setSendError] = useState<string | null>(null);
  const sendMutation = useSendEmailMutation();

  // ─── Editor ──────────────────────────────────────────────────────────────────
  const {
    editor, editorMode, rawSource, setRawSource,
    richTextHtml, htmlPreview, setHtmlPreview,
    switchMode, textareaRef,
  } = useComposeEditor({
    fromAccountId, composeMode, composeReplyTo, isReply, t,
    restoredDraft: restoredDraft.current,
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
  const [isDragging, setIsDragging] = useState(false);

  const [editorReady, setEditorReady] = useState(false);
  useEffect(() => {
    if (editor && !editorReady) setEditorReady(true);
  }, [editor, editorReady]);
  const { draftIdRef, draftIdsByAccountRef } = useComposeDraft({
    to, cc, bcc, subject, rawSource, richTextHtml, editorMode, composeMode, fromAccountId,
    attachments,
    editorReady,
  });

  // ─── Templates ───────────────────────────────────────────────────────────────
  const [showTemplates, setShowTemplates] = useState(false);
  const [templates, setTemplates] = useState<EmailTemplate[]>(() => listTemplates());
  const [showSaveTemplate, setShowSaveTemplate] = useState(false);
  const [templateName, setTemplateName] = useState("");

  useEffect(() => {
    if (!sendError) return;
    const timer = setTimeout(() => setSendError(null), 5000);
    return () => clearTimeout(timer);
  }, [sendError]);

  async function handleSend() {
    if (!fromAccountId || to.length === 0) return;

    if (!subject.trim()) {
      const confirmed = await useConfirmStore.getState().confirm({
        title: t("compose.noSubjectTitle", "No subject"),
        message: t("compose.noSubjectConfirm", "Send without a subject?"),
        confirmLabel: t("common.send", "Send"),
      });
      if (!confirmed) return;
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

    const inReplyTo =
      isReply && composeReplyTo?.message_id_header
        ? composeReplyTo.message_id_header
        : undefined;

    sendMutation.mutate(
      {
        accountId: fromAccountId,
        to: to.filter(Boolean),
        cc: cc.filter(Boolean),
        bcc: bcc.filter(Boolean),
        subject,
        bodyText,
        bodyHtml: bodyHtml || undefined,
        inReplyTo: inReplyTo || undefined,
        attachmentPaths: attachments.length > 0 ? attachments.map((a) => a.path) : undefined,
      },
      {
        onSuccess: () => {
          const draftIdsToDelete = { ...draftIdsByAccountRef.current };
          if (draftIdRef.current && fromAccountId) {
            draftIdsToDelete[fromAccountId] = draftIdRef.current;
          }
          for (const [draftAccountId, draftId] of Object.entries(draftIdsToDelete)) {
            deleteDraft(draftAccountId, draftId).catch((err) => {
              console.warn("Failed to delete draft after send:", err);
              useToastStore.getState().addToast({
                type: "error",
                message: t(
                  "compose.draftCleanupFailed",
                  "Sent, but failed to remove the saved draft. You can delete it from Drafts.",
                ),
              });
            });
          }
          draftIdsByAccountRef.current = {};
          draftIdRef.current = null;
          clearDraftStorage();
          useComposeStore.getState().setComposeDirty(false);
          closeCompose();
        },
        onError: (e) => {
          const msg = e instanceof Error ? e.message : String(e);
          setSendError(msg || t("compose.sendError", "Failed to send"));
        },
      },
    );
  }

  const title =
    composeMode === "reply"
      ? t("compose.reply", "Reply")
      : composeMode === "reply-all"
        ? t("compose.replyAll", "Reply All")
        : composeMode === "forward"
          ? t("compose.forward", "Forward")
          : t("compose.newMessage", "New Message");

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
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
          disabled={sendMutation.isPending || to.length === 0}
          style={{
            display: "flex", alignItems: "center", gap: "6px",
            padding: "7px 20px",
            backgroundColor: sendMutation.isPending ? "var(--color-text-secondary)" : "var(--color-accent, #2563eb)",
            color: "#fff", border: "none", borderRadius: "6px",
            cursor: sendMutation.isPending || to.length === 0 ? "default" : "pointer",
            opacity: to.length === 0 ? 0.5 : 1,
            fontSize: "13px", fontWeight: 500,
          }}
        >
          <Send size={14} />
          {sendMutation.isPending ? t("compose.sending", "Sending...") : t("compose.send", "Send")}
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
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "auto" }}>
        <div style={{ maxWidth: "768px", width: "100%", margin: "0 auto", display: "flex", flexDirection: "column", flex: 1 }}>
          {/* From */}
          {accounts.length > 1 && (
            <div style={composeStyles.fieldRow}>
              <span style={composeStyles.fieldLabel}>{t("compose.from", "From")}</span>
              <select
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
                    {acc.display_name ? `${acc.display_name} <${acc.email}>` : acc.email}
                  </option>
                ))}
              </select>
            </div>
          )}

          {/* To */}
          <div style={composeStyles.fieldRow}>
            <span style={composeStyles.fieldLabel}>{t("compose.to", "To")}</span>
            <ContactAutocomplete value={to} onChange={setTo} accountId={fromAccountId} placeholder="recipient@example.com" />
            <div style={{ display: "flex", gap: "4px", padding: "0 8px", flexShrink: 0 }}>
              {!showCc && <button onClick={() => setShowCc(true)} style={composeStyles.toggleBtn}>{t("compose.cc", "Cc")}</button>}
              {!showBcc && <button onClick={() => setShowBcc(true)} style={composeStyles.toggleBtn}>{t("compose.bcc", "Bcc")}</button>}
            </div>
          </div>

          {showCc && (
            <div style={composeStyles.fieldRow}>
              <span style={composeStyles.fieldLabel}>{t("compose.cc", "Cc")}</span>
              <ContactAutocomplete value={cc} onChange={setCc} accountId={fromAccountId} placeholder="cc@example.com" />
            </div>
          )}

          {showBcc && (
            <div style={composeStyles.fieldRow}>
              <span style={composeStyles.fieldLabel}>{t("compose.bcc", "Bcc")}</span>
              <ContactAutocomplete value={bcc} onChange={setBcc} accountId={fromAccountId} placeholder="bcc@example.com" />
            </div>
          )}

          {/* Subject */}
          <div style={composeStyles.fieldRow}>
            <span style={composeStyles.fieldLabel}>{t("compose.subject", "Subject")}</span>
              <input
                type="text" value={subject} onChange={(e) => setSubject(e.target.value)}
                placeholder={t("compose.subject", "Subject")}
                style={{ flex: 1, padding: "8px 0", border: "none", backgroundColor: "transparent", fontSize: "13px", color: "var(--color-text-primary)" }}
              />
          </div>

          {/* Mode switcher + Toolbar */}
          <div style={{
            display: "flex", alignItems: "center", gap: "0",
            borderBottom: "1px solid var(--color-border)",
          }}>
            {/* Attach + Template buttons */}
            <div style={{ display: "flex", alignItems: "center", gap: "2px", padding: "4px 8px" }}>
              <button
                type="button"
                onClick={() => attachInputRef.current?.click()}
                title={t("compose.attach", "Attach file")}
                aria-label={t("compose.attach", "Attach file")}
                style={{
                  display: "flex", alignItems: "center", gap: "4px",
                  padding: "4px 8px", borderRadius: "4px",
                  border: "none", cursor: "pointer", fontSize: "11px",
                  backgroundColor: "transparent", color: "var(--color-text-secondary)",
                }}
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
                  const newAttachments = Array.from(files).map((file) => ({
                    name: file.name,
                    path: (file as unknown as { path?: string }).path || file.name,
                    size: file.size,
                  }));
                  setAttachments((prev) => [...prev, ...newAttachments]);
                  e.target.value = "";
                }}
              />
              <div style={{ position: "relative" }}>
                <button
                  onClick={() => { setTemplates(listTemplates()); setShowTemplates((v) => !v); }}
                  aria-haspopup="listbox"
                  aria-expanded={showTemplates}
                  aria-label={t("compose.templates", "Templates")}
                  title={t("compose.templates", "Templates")}
                  style={{
                    display: "flex", alignItems: "center", gap: "4px",
                    padding: "4px 8px", borderRadius: "4px",
                    border: "none", cursor: "pointer", fontSize: "11px",
                    backgroundColor: showTemplates ? "var(--color-bg-secondary)" : "transparent",
                    color: showTemplates ? "var(--color-accent)" : "var(--color-text-secondary)",
                  }}
                >
                  <BookTemplate size={13} />
                </button>
                {showTemplates && (
                  <div style={{
                    position: "absolute", top: "100%", left: 0, zIndex: 100,
                    backgroundColor: "var(--color-bg)", border: "1px solid var(--color-border)",
                    borderRadius: "8px", boxShadow: "0 8px 24px rgba(0,0,0,0.12)",
                    minWidth: "220px", maxHeight: "300px", overflowY: "auto",
                  }}>
                    <div style={{ padding: "8px", borderBottom: "1px solid var(--color-border)", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                      <span id="compose-templates-label" style={{ fontSize: "12px", fontWeight: 600 }}>{t("compose.templates", "Templates")}</span>
                      <button
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
                                onClick={async (e) => {
                                  e.stopPropagation();
                                  const confirmed = await useConfirmStore.getState().confirm({
                                    title: t("compose.deleteTemplate", "Delete template"),
                                    message: t("compose.deleteTemplate", "Delete template") + ` "${tpl.name}"?`,
                                    destructive: true,
                                  });
                                  if (confirmed) { deleteTemplate(tpl.id); setTemplates(listTemplates()); }
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
          </div>
          <div style={{
            display: "flex", alignItems: "center", gap: "0",
            borderBottom: "1px solid var(--color-border)",
          }}>
            {/* Formatting toolbar */}
            {editorMode === "rich" && editor && (
              <div style={{ flex: 1 }}>
                <EditorToolbar editor={editor} />
              </div>
            )}
            {editorMode === "markdown" && (
              <div style={{ flex: 1 }}>
                <MarkdownToolbar textareaRef={textareaRef} onInsert={setRawSource} source={rawSource} />
              </div>
            )}
            {editorMode === "html" && (
              <div style={{ flex: 1, display: "flex", alignItems: "center", padding: "6px 8px" }}>
                <button
                  onClick={() => setHtmlPreview((v) => !v)}
                  title={htmlPreview ? t("compose.mode.hidePreview", "Hide preview") : t("compose.mode.showPreview", "Show preview")}
                  style={{
                    display: "flex", alignItems: "center", gap: "4px",
                    padding: "4px 8px", borderRadius: "4px",
                    border: "none", cursor: "pointer", fontSize: "11px",
                    backgroundColor: htmlPreview ? "var(--color-bg-secondary, rgba(0,0,0,0.08))" : "transparent",
                    color: htmlPreview ? "var(--color-accent, #2563eb)" : "var(--color-text-secondary)",
                  }}
                >
                  {htmlPreview ? <EyeOff size={13} /> : <Eye size={13} />}
                  {htmlPreview ? t("compose.mode.hidePreview", "Hide preview") : t("compose.mode.showPreview", "Show preview")}
                </button>
              </div>
            )}

            {/* Mode tabs */}
            <div style={{ display: "flex", gap: "2px", padding: "4px 8px", flexShrink: 0 }}>
              <ModeButton icon={Type} label={t("compose.mode.rich", "Rich Text")} active={editorMode === "rich"} onClick={() => switchMode("rich")} />
              <ModeButton icon={Hash} label={t("compose.mode.markdown", "Markdown")} active={editorMode === "markdown"} onClick={() => switchMode("markdown")} />
              <ModeButton icon={FileCode2} label={t("compose.mode.html", "HTML")} active={editorMode === "html"} onClick={() => switchMode("html")} />
            </div>
          </div>

          {/* Attachment list */}
          {attachments.length > 0 && (
            <div style={{ padding: "8px 60px", borderBottom: "1px solid var(--color-border)", display: "flex", flexWrap: "wrap", gap: "6px" }}>
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
                    onClick={() => setAttachments((prev) => prev.filter((_, j) => j !== i))}
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
            <div style={{
              padding: "8px 60px", borderBottom: "1px solid var(--color-border)",
              display: "flex", alignItems: "center", gap: "8px",
            }}>
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
                    const bodyContent = editorMode === "rich" && editor ? editor.getHTML() : rawSource;
                    saveTemplate({ name: templateName.trim(), subject, body: bodyContent });
                    setTemplateName("");
                    setShowSaveTemplate(false);
                    setTemplates(listTemplates());
                  }
                  if (e.key === "Escape") setShowSaveTemplate(false);
                }}
              />
              <button
                onClick={() => {
                  if (!templateName.trim()) return;
                  const bodyContent = editorMode === "rich" && editor ? editor.getHTML() : rawSource;
                  saveTemplate({ name: templateName.trim(), subject, body: bodyContent });
                  setTemplateName("");
                  setShowSaveTemplate(false);
                  setTemplates(listTemplates());
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
            style={{ flex: 1, minHeight: "300px", position: "relative" }}
            onDragOver={(e) => { e.preventDefault(); setIsDragging(true); }}
            onDragLeave={() => setIsDragging(false)}
            onDrop={async (e) => {
              e.preventDefault();
              setIsDragging(false);
              const files = e.dataTransfer.files;
              if (!files.length) return;
              const newAttachments: { name: string; path: string; size: number }[] = [];
              for (let i = 0; i < files.length; i++) {
                const file = files[i];
                const path = (file as unknown as { path?: string }).path || file.name;
                newAttachments.push({ name: file.name, path, size: file.size });
              }
              setAttachments((prev) => [...prev, ...newAttachments]);
            }}
          >
            {isDragging && (
              <div style={{
                position: "absolute", inset: 0, zIndex: 10,
                backgroundColor: "rgba(37, 99, 235, 0.08)",
                border: "2px dashed var(--color-accent)",
                borderRadius: "8px",
                display: "flex", alignItems: "center", justifyContent: "center",
                color: "var(--color-accent)", fontSize: "14px", fontWeight: 500,
              }}>
                <Paperclip size={20} style={{ marginRight: "8px" }} />
                {t("compose.dropFiles", "Drop files to attach")}
              </div>
            )}
            {editorMode === "rich" ? (
              <EditorContent
                editor={editor}
                style={{
                  padding: "16px 60px", fontSize: "14px",
                  color: "var(--color-text-primary)", minHeight: "300px", lineHeight: 1.7,
                }}
              />
            ) : editorMode === "html" && htmlPreview ? (
              <div style={{ display: "flex", height: "100%", minHeight: "300px" }}>
                <textarea
                  ref={textareaRef}
                  value={rawSource}
                  onChange={(e) => setRawSource(e.target.value)}
                  placeholder={t("compose.mode.htmlPlaceholder", "Write HTML source...")}
                  spellCheck={false}
                  style={{
                    width: "50%", height: "100%", minHeight: "300px",
                    padding: "16px 20px", border: "none", resize: "none",
                    backgroundColor: "transparent",
                    fontSize: "13px", lineHeight: 1.6,
                    color: "var(--color-text-primary)",
                    fontFamily: "monospace",
                    borderRight: "1px solid var(--color-border)",
                  }}
                />
                <iframe
                  sandbox="allow-same-origin"
                  srcDoc={rawSource}
                  title={t("compose.mode.preview", "Preview")}
                  style={{
                    width: "50%", height: "100%", minHeight: "300px",
                    border: "none",
                  }}
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
                style={{
                  width: "100%", height: "100%", minHeight: "300px",
                  padding: "16px 60px", border: "none", resize: "none",
                  backgroundColor: "transparent",
                  fontSize: "13px", lineHeight: 1.6,
                  color: "var(--color-text-primary)",
                  fontFamily: editorMode === "html" ? "monospace" : "inherit",
                }}
              />
            )}
          </div>
        </div>
      </div>

      {/* Compose leave confirmation dialog */}
      {showComposeLeaveConfirm && (
        <div
          style={{
            position: "fixed", inset: 0, zIndex: 9999,
            display: "flex", alignItems: "center", justifyContent: "center",
            backgroundColor: "rgba(0,0,0,0.4)",
          }}
          onClick={cancelCloseCompose}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="compose-leave-title"
            style={{
              backgroundColor: "var(--color-bg-primary, #fff)",
              borderRadius: "8px", padding: "24px", minWidth: "320px",
              boxShadow: "0 8px 32px rgba(0,0,0,0.2)",
            }}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => { if (e.key === "Escape") cancelCloseCompose(); }}
          >
            <h3 id="compose-leave-title" style={{ margin: "0 0 8px", fontSize: "15px", fontWeight: 600 }}>
              {t("compose.leaveTitle", "Discard draft?")}
            </h3>
            <p style={{ margin: "0 0 20px", fontSize: "13px", color: "var(--color-text-secondary)" }}>
              {t("compose.leaveMessage", "You have unsaved changes. Are you sure you want to leave?")}
            </p>
            <div style={{ display: "flex", justifyContent: "flex-end", gap: "8px" }}>
              <button
                onClick={cancelCloseCompose}
                style={{
                  padding: "6px 16px", borderRadius: "6px", fontSize: "13px",
                  border: "1px solid var(--color-border)", cursor: "pointer",
                  backgroundColor: "transparent", color: "var(--color-text-primary)",
                }}
              >
                {t("compose.leaveCancel", "Keep editing")}
              </button>
              <button
                onClick={confirmCloseCompose}
                style={{
                  padding: "6px 16px", borderRadius: "6px", fontSize: "13px",
                  border: "none", cursor: "pointer",
                  backgroundColor: "var(--color-danger, #ef4444)", color: "#fff",
                }}
              >
                {t("compose.leaveConfirm", "Discard")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
