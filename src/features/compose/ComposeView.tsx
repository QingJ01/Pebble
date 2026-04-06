import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { useEditor, useEditorState, EditorContent } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown as MarkdownExtension } from "tiptap-markdown";
import TurndownService from "turndown";
import {
  ArrowLeft, Send, Bold, Italic, Strikethrough, Heading1, Heading2,
  List, ListOrdered, Quote, Code, Minus, Undo2, Redo2, X, AlertCircle,
  Type, FileCode2, Hash, Link, Image, Eye, EyeOff,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import { useAccountsQuery } from "@/hooks/queries";
import { useSendEmailMutation } from "@/hooks/mutations";
import ContactAutocomplete from "@/components/ContactAutocomplete";
import { hasComposeDraft } from "./compose-draft";
import type { Editor } from "@tiptap/react";

type EditorMode = "rich" | "markdown" | "html";

const turndown = new TurndownService({ headingStyle: "atx", codeBlockStyle: "fenced" });

const DRAFT_STORAGE_KEY = "pebble-compose-draft";

interface DraftData {
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  rawSource: string;
  richTextHtml: string;
  editorMode: EditorMode;
  savedAt: number;
}

function saveDraftToStorage(draft: Omit<DraftData, "savedAt">) {
  try {
    localStorage.setItem(DRAFT_STORAGE_KEY, JSON.stringify({ ...draft, savedAt: Date.now() }));
  } catch { /* quota exceeded — silently skip */ }
}

function loadDraftFromStorage(): DraftData | null {
  try {
    const raw = localStorage.getItem(DRAFT_STORAGE_KEY);
    if (!raw) return null;
    const draft = JSON.parse(raw) as DraftData;
    // Discard drafts older than 24 hours
    if (Date.now() - draft.savedAt > 24 * 60 * 60 * 1000) {
      localStorage.removeItem(DRAFT_STORAGE_KEY);
      return null;
    }
    return draft;
  } catch { return null; }
}

function clearDraftStorage() {
  localStorage.removeItem(DRAFT_STORAGE_KEY);
}

export default function ComposeView() {
  const { t } = useTranslation();
  const composeMode = useUIStore((s) => s.composeMode);
  const composeReplyTo = useUIStore((s) => s.composeReplyTo);
  const closeCompose = useUIStore((s) => s.closeCompose);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const { data: accounts = [] } = useAccountsQuery();

  const [fromAccountId, setFromAccountId] = useState(activeAccountId || "");
  const currentAccount = accounts.find((a) => a.id === fromAccountId);
  const myEmail = currentAccount?.email || "";

  const isReply = composeMode === "reply" || composeMode === "reply-all";
  const restoredDraft = useRef<DraftData | null>(composeMode === "new" ? loadDraftFromStorage() : null);

  const [to, setTo] = useState<string[]>(() => {
    if (restoredDraft.current) return restoredDraft.current.to;
    if (!composeReplyTo) return [];
    if (composeMode === "reply") return [composeReplyTo.from_address];
    if (composeMode === "reply-all") {
      const all = [composeReplyTo.from_address, ...composeReplyTo.to_list.map((a) => a.address)];
      return [...new Set(all)].filter((addr) => addr !== myEmail);
    }
    return [];
  });

  const [cc, setCc] = useState<string[]>(() => {
    if (restoredDraft.current) return restoredDraft.current.cc;
    if (composeMode === "reply-all" && composeReplyTo) {
      return composeReplyTo.cc_list.map((a) => a.address).filter((addr) => addr !== myEmail);
    }
    return [];
  });

  const [bcc, setBcc] = useState<string[]>(restoredDraft.current?.bcc ?? []);
  const [showCc, setShowCc] = useState(() => cc.length > 0);
  const [showBcc, setShowBcc] = useState(false);

  // Re-calculate from/to/cc once accounts data loads (fixes reply-all with async data)
  useEffect(() => {
    if (accounts.length === 0) return;
    // Determine the correct account ID using local variable (not stale state)
    const newAccountId = (!fromAccountId || !accounts.find((a) => a.id === fromAccountId))
      ? (activeAccountId || accounts[0]?.id || "")
      : fromAccountId;
    if (newAccountId !== fromAccountId) {
      setFromAccountId(newAccountId);
    }
    // Re-filter to/cc to remove own email address using the resolved account
    const resolvedEmail = accounts.find((a) => a.id === newAccountId)?.email || "";
    if (composeMode === "reply-all" && composeReplyTo && resolvedEmail) {
      setTo((prev) => prev.filter((addr) => addr !== resolvedEmail));
      setCc((prev) => prev.filter((addr) => addr !== resolvedEmail));
    }
  }, [accounts]); // eslint-disable-line react-hooks/exhaustive-deps

  const [subject, setSubject] = useState(() => {
    if (restoredDraft.current) return restoredDraft.current.subject;
    if (!composeReplyTo) return "";
    if (isReply) return `Re: ${composeReplyTo.subject.replace(/^(Re:\s*|Fwd:\s*)+/i, "")}`;
    if (composeMode === "forward") return `Fwd: ${composeReplyTo.subject.replace(/^(Re:\s*|Fwd:\s*)+/i, "")}`;
    return "";
  });
  const [sendError, setSendError] = useState<string | null>(null);
  const sendMutation = useSendEmailMutation();

  // Editor mode: rich (WYSIWYG), markdown (raw text), html (source)
  const [editorMode, setEditorMode] = useState<EditorMode>(restoredDraft.current?.editorMode ?? "rich");
  const [rawSource, setRawSource] = useState(restoredDraft.current?.rawSource ?? "");
  const [richTextHtml, setRichTextHtml] = useState("");
  const [htmlPreview, setHtmlPreview] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Snapshot the initial compose state so pre-populated reply/forward
  // fields don't immediately trigger the "unsaved draft" guard.
  const initialSnapshot = useRef<{
    to: string[]; cc: string[]; bcc: string[]; subject: string;
  } | null>(null);
  if (!initialSnapshot.current) {
    initialSnapshot.current = { to: [...to], cc: [...cc], bcc: [...bcc], subject };
  }

  const arraysEqual = useCallback(
    (a: string[], b: string[]) => a.length === b.length && a.every((v, i) => v === b[i]),
    [],
  );

  // Track dirty state for leave-protection
  useEffect(() => {
    const init = initialSnapshot.current!;
    const userChanged =
      !arraysEqual(to, init.to) ||
      !arraysEqual(cc, init.cc) ||
      !arraysEqual(bcc, init.bcc) ||
      subject !== init.subject ||
      rawSource.trim().length > 0 ||
      hasComposeDraft({ to: [], cc: [], bcc: [], subject: "", rawSource, richTextHtml });
    useUIStore.getState().setComposeDirty(userChanged);
  }, [arraysEqual, bcc, cc, rawSource, richTextHtml, subject, to]);

  // Auto-save draft to localStorage (debounced 1s)
  useEffect(() => {
    if (!composeMode || composeMode !== "new") return;
    const timer = setTimeout(() => {
      const hasDraft = to.length > 0 || cc.length > 0 || bcc.length > 0 || subject.trim() || rawSource.trim() || richTextHtml.trim();
      if (hasDraft) {
        saveDraftToStorage({ to, cc, bcc, subject, rawSource, richTextHtml, editorMode });
      }
    }, 1000);
    return () => clearTimeout(timer);
  }, [to, cc, bcc, subject, rawSource, richTextHtml, editorMode, composeMode]);

  useEffect(() => {
    if (!sendError) return;
    const timer = setTimeout(() => setSendError(null), 5000);
    return () => clearTimeout(timer);
  }, [sendError]);

  const editorContent = useMemo(() => {
    try {
      const esc = (s: string) =>
        s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

      const extractBody = (html: string) => {
        try {
          const doc = new DOMParser().parseFromString(html, "text/html");
          return doc.body.innerHTML;
        } catch {
          return `<p>${esc(html)}</p>`;
        }
      };

      if (isReply && composeReplyTo) {
        const sender = esc(composeReplyTo.from_name || composeReplyTo.from_address || "");
        const dateStr = esc(new Date((composeReplyTo.date || 0) * 1000).toLocaleString());
        const body = composeReplyTo.body_html_raw
          ? extractBody(composeReplyTo.body_html_raw)
          : `<p>${esc(composeReplyTo.body_text || "")}</p>`;
        const attribution = t("compose.quoteAttribution", { date: dateStr, sender });
        return `<br/><br/><blockquote><p>${esc(attribution)}</p>${body}</blockquote>`;
      }
      if (composeMode === "forward" && composeReplyTo) {
        const sender = esc(composeReplyTo.from_name || composeReplyTo.from_address || "");
        const fwdSubject = esc(composeReplyTo.subject || "");
        const body = composeReplyTo.body_html_raw
          ? extractBody(composeReplyTo.body_html_raw)
          : `<p>${esc(composeReplyTo.body_text || "")}</p>`;
        return `<br/><br/><p>${esc(t("compose.forwardedHeader"))}</p><p>${esc(t("compose.forwardedFrom", { sender }))}</p><p>${esc(t("compose.forwardedSubject", { subject: fwdSubject }))}</p>${body}`;
      }
      return "";
    } catch (err) {
      console.error("[ComposeView] Failed to build editor content:", err);
      return "";
    }
  }, [composeMode, composeReplyTo, isReply, t]);

  const editor = useEditor({
    immediatelyRender: false,
    extensions: [
      StarterKit,
      Placeholder.configure({ placeholder: t("compose.editorPlaceholder", "Write your message...") }),
      MarkdownExtension.configure({ html: true, transformPastedText: true }),
    ],
    content: "",
  });

  // Set editor content after creation to avoid initialization crashes
  useEffect(() => {
    if (editor && editorContent) {
      editor.commands.setContent(editorContent);
    }
  }, [editor, editorContent]);

  useEffect(() => {
    if (!editor) {
      return;
    }

    const syncRichTextHtml = () => {
      setRichTextHtml(editor.getHTML());
    };

    syncRichTextHtml();
    editor.on("update", syncRichTextHtml);

    return () => {
      editor.off("update", syncRichTextHtml);
    };
  }, [editor]);

  // Switch between modes, syncing content
  function switchMode(newMode: EditorMode) {
    if (newMode === editorMode || !editor) return;

    if (editorMode === "rich") {
      // Leaving rich → capture content
      if (newMode === "markdown") {
        setRawSource(turndown.turndown(editor.getHTML()));
      } else {
        setRawSource(editor.getHTML());
      }
    } else if (editorMode === "markdown") {
      // Leaving markdown
      if (newMode === "rich") {
        // The Markdown extension handles conversion from markdown to HTML
        editor.commands.setContent(rawSource);
      } else {
        // markdown → html: convert via temp editor set
        editor.commands.setContent(rawSource);
        setRawSource(editor.getHTML());
      }
    } else {
      // Leaving html
      if (newMode === "rich") {
        editor.commands.setContent(rawSource);
      } else {
        setRawSource(turndown.turndown(rawSource));
      }
    }

    setEditorMode(newMode);
  }

  function handleSend() {
    if (!fromAccountId || to.length === 0) return;
    setSendError(null);

    let bodyHtml = "";
    let bodyText = "";

    if (editorMode === "rich" && editor) {
      bodyHtml = editor.getHTML();
      bodyText = editor.getText();
    } else if (editorMode === "html") {
      bodyHtml = rawSource;
      // Strip tags for plain text fallback
      const tmp = document.createElement("div");
      tmp.innerHTML = rawSource;
      bodyText = tmp.textContent || tmp.innerText || "";
    } else {
      // markdown mode — convert to HTML via editor
      if (editor) {
        editor.commands.setContent(rawSource);
        bodyHtml = editor.getHTML();
        bodyText = rawSource; // markdown is already readable plain text
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
      },
      {
        onSuccess: () => {
          clearDraftStorage();
          useUIStore.getState().setComposeDirty(false);
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
            style={backBtnStyle}
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
            <div style={fieldRowStyle}>
              <span style={fieldLabelStyle}>{t("compose.from", "From")}</span>
              <select
                value={fromAccountId}
                onChange={(e) => setFromAccountId(e.target.value)}
                style={{
                  flex: 1, padding: "6px 0", border: "none", outline: "none",
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
          <div style={fieldRowStyle}>
            <span style={fieldLabelStyle}>{t("compose.to", "To")}</span>
            <ContactAutocomplete value={to} onChange={setTo} accountId={fromAccountId} placeholder="recipient@example.com" />
            <div style={{ display: "flex", gap: "4px", padding: "0 8px", flexShrink: 0 }}>
              {!showCc && <button onClick={() => setShowCc(true)} style={toggleBtnStyle}>{t("compose.cc", "Cc")}</button>}
              {!showBcc && <button onClick={() => setShowBcc(true)} style={toggleBtnStyle}>{t("compose.bcc", "Bcc")}</button>}
            </div>
          </div>

          {showCc && (
            <div style={fieldRowStyle}>
              <span style={fieldLabelStyle}>{t("compose.cc", "Cc")}</span>
              <ContactAutocomplete value={cc} onChange={setCc} accountId={fromAccountId} placeholder="cc@example.com" />
            </div>
          )}

          {showBcc && (
            <div style={fieldRowStyle}>
              <span style={fieldLabelStyle}>{t("compose.bcc", "Bcc")}</span>
              <ContactAutocomplete value={bcc} onChange={setBcc} accountId={fromAccountId} placeholder="bcc@example.com" />
            </div>
          )}

          {/* Subject */}
          <div style={fieldRowStyle}>
            <span style={fieldLabelStyle}>{t("compose.subject", "Subject")}</span>
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

          {/* Editor area */}
          <div style={{ flex: 1, minHeight: "300px" }}>
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
    </div>
  );
}

// ─── Mode Button ───────────────────────────────────────────────────────────────

function ModeButton({ icon: Icon, label, active, onClick }: {
  icon: React.ElementType; label: string; active: boolean; onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      style={{
        display: "flex", alignItems: "center", gap: "4px",
        padding: "4px 8px", borderRadius: "4px",
        border: "none", cursor: "pointer",
        fontSize: "11px", fontWeight: active ? 600 : 400,
        backgroundColor: active ? "var(--color-bg-secondary, rgba(0,0,0,0.08))" : "transparent",
        color: active ? "var(--color-accent, #2563eb)" : "var(--color-text-secondary)",
        transition: "background-color 0.1s ease",
      }}
      onMouseEnter={(e) => { if (!active) e.currentTarget.style.backgroundColor = "var(--color-bg-hover, rgba(0,0,0,0.04))"; }}
      onMouseLeave={(e) => { if (!active) e.currentTarget.style.backgroundColor = "transparent"; }}
    >
      <Icon size={13} />
      {label}
    </button>
  );
}

// ─── Editor Toolbar ────────────────────────────────────────────────────────────

function EditorToolbar({ editor }: { editor: Editor }) {
  const { t } = useTranslation();

  const activeStates = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      strike: e.isActive("strike"),
      h1: e.isActive("heading", { level: 1 }),
      h2: e.isActive("heading", { level: 2 }),
      bulletList: e.isActive("bulletList"),
      orderedList: e.isActive("orderedList"),
      blockquote: e.isActive("blockquote"),
      codeBlock: e.isActive("codeBlock"),
    }),
  });

  function btn(icon: React.ElementType, label: string, action: () => void, active?: boolean) {
    return { icon, label, action, active };
  }

  const items = [
    btn(Bold, t("compose.toolbar.bold", "Bold"), () => editor.chain().focus().toggleBold().run(), activeStates.bold),
    btn(Italic, t("compose.toolbar.italic", "Italic"), () => editor.chain().focus().toggleItalic().run(), activeStates.italic),
    btn(Strikethrough, t("compose.toolbar.strike", "Strikethrough"), () => editor.chain().focus().toggleStrike().run(), activeStates.strike),
    btn(Heading1, t("compose.toolbar.heading1"), () => editor.chain().focus().toggleHeading({ level: 1 }).run(), activeStates.h1),
    btn(Heading2, t("compose.toolbar.heading2"), () => editor.chain().focus().toggleHeading({ level: 2 }).run(), activeStates.h2),
    btn(List, t("compose.toolbar.bulletList", "Bullet list"), () => editor.chain().focus().toggleBulletList().run(), activeStates.bulletList),
    btn(ListOrdered, t("compose.toolbar.orderedList", "Ordered list"), () => editor.chain().focus().toggleOrderedList().run(), activeStates.orderedList),
    btn(Quote, t("compose.toolbar.blockquote", "Quote"), () => editor.chain().focus().toggleBlockquote().run(), activeStates.blockquote),
    btn(Code, t("compose.toolbar.code", "Code"), () => editor.chain().focus().toggleCodeBlock().run(), activeStates.codeBlock),
    btn(Minus, t("compose.toolbar.hr", "Divider"), () => editor.chain().focus().setHorizontalRule().run()),
    btn(Undo2, t("compose.toolbar.undo", "Undo"), () => editor.chain().focus().undo().run()),
    btn(Redo2, t("compose.toolbar.redo", "Redo"), () => editor.chain().focus().redo().run()),
  ];

  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: "2px", padding: "6px 8px" }}>
      {items.map((item, i) => {
        const Icon = item.icon;
        return (
          <button
            key={i}
            onClick={item.action}
            title={item.label}
            aria-label={item.label}
            style={{
              display: "flex", alignItems: "center", justifyContent: "center",
              width: "28px", height: "28px", borderRadius: "4px",
              border: "none", cursor: "pointer",
              backgroundColor: item.active ? "var(--color-bg-secondary, rgba(0,0,0,0.08))" : "transparent",
              color: item.active ? "var(--color-accent, #2563eb)" : "var(--color-text-secondary)",
              transition: "background-color 0.1s ease, color 0.1s ease",
            }}
            onMouseEnter={(e) => { if (!item.active) e.currentTarget.style.backgroundColor = "var(--color-bg-hover, rgba(0,0,0,0.04))"; }}
            onMouseLeave={(e) => { if (!item.active) e.currentTarget.style.backgroundColor = "transparent"; }}
          >
            <Icon size={15} />
          </button>
        );
      })}
    </div>
  );
}

// ─── Markdown Toolbar ─────────────────────────────────────────────────────────

function MarkdownToolbar({ textareaRef, onInsert, source }: {
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  onInsert: (value: string) => void;
  source: string;
}) {
  const { t } = useTranslation();

  function insert(before: string, after = "", placeholder = "") {
    const ta = textareaRef.current;
    if (!ta) {
      onInsert(source + before + placeholder + after);
      return;
    }
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const selected = source.slice(start, end) || placeholder;
    const newText = source.slice(0, start) + before + selected + after + source.slice(end);
    onInsert(newText);
    // Restore cursor position after React re-render
    requestAnimationFrame(() => {
      ta.focus();
      const cursorPos = start + before.length + selected.length;
      ta.setSelectionRange(cursorPos, cursorPos);
    });
  }

  const items = [
    { icon: Bold, label: t("compose.toolbar.bold", "Bold"), action: () => insert("**", "**", "bold") },
    { icon: Italic, label: t("compose.toolbar.italic", "Italic"), action: () => insert("*", "*", "italic") },
    { icon: Strikethrough, label: t("compose.toolbar.strike", "Strikethrough"), action: () => insert("~~", "~~", "text") },
    { icon: Heading1, label: t("compose.toolbar.heading1"), action: () => insert("\n# ", "\n", "heading") },
    { icon: Heading2, label: t("compose.toolbar.heading2"), action: () => insert("\n## ", "\n", "heading") },
    { icon: List, label: t("compose.toolbar.bulletList", "Bullet list"), action: () => insert("\n- ", "") },
    { icon: ListOrdered, label: t("compose.toolbar.orderedList", "Ordered list"), action: () => insert("\n1. ", "") },
    { icon: Quote, label: t("compose.toolbar.blockquote", "Quote"), action: () => insert("\n> ", "") },
    { icon: Code, label: t("compose.toolbar.code", "Code"), action: () => insert("`", "`", "code") },
    { icon: Minus, label: t("compose.toolbar.hr", "Divider"), action: () => insert("\n---\n", "") },
    { icon: Link, label: t("compose.toolbar.link", "Link"), action: () => insert("[", "](url)", "text") },
    { icon: Image, label: t("compose.toolbar.image", "Image"), action: () => insert("![", "](url)", "alt") },
  ];

  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: "2px", padding: "6px 8px" }}>
      {items.map((item, i) => {
        const Icon = item.icon;
        return (
          <button
            key={i}
            onClick={item.action}
            title={item.label}
            aria-label={item.label}
            style={{
              display: "flex", alignItems: "center", justifyContent: "center",
              width: "28px", height: "28px", borderRadius: "4px",
              border: "none", cursor: "pointer",
              backgroundColor: "transparent",
              color: "var(--color-text-secondary)",
              transition: "background-color 0.1s ease",
            }}
            onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = "var(--color-bg-hover, rgba(0,0,0,0.04))"; }}
            onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = "transparent"; }}
          >
            <Icon size={15} />
          </button>
        );
      })}
    </div>
  );
}

// ─── Shared Styles ─────────────────────────────────────────────────────────────

const fieldLabelStyle: React.CSSProperties = {
  padding: "8px 0", fontSize: "13px", color: "var(--color-text-secondary)",
  width: "52px", flexShrink: 0, textAlign: "right", marginRight: "8px",
};

const fieldRowStyle: React.CSSProperties = {
  display: "flex", alignItems: "center",
  borderBottom: "1px solid var(--color-border)",
};

const toggleBtnStyle: React.CSSProperties = {
  padding: "4px 8px", border: "none", background: "none", cursor: "pointer",
  color: "var(--color-text-secondary)", fontSize: "12px", whiteSpace: "nowrap",
  borderRadius: "4px",
};

const backBtnStyle: React.CSSProperties = {
  display: "flex", alignItems: "center", gap: "4px",
  background: "none", border: "none", cursor: "pointer",
  color: "var(--color-text-secondary)", fontSize: "13px",
  padding: "4px 8px", borderRadius: "4px",
};
