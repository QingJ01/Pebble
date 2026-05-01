import { useState, useEffect, useRef, useMemo } from "react";
import { useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown as MarkdownExtension } from "tiptap-markdown";
import TurndownService from "turndown";
import { getSignature } from "@/lib/signatures";
import { sanitizeHtml } from "@/lib/sanitizeHtml";
import type { Message } from "@/lib/ipc-types";
import type { TFunction } from "i18next";

export type EditorMode = "rich" | "markdown" | "html";

const turndown = new TurndownService({ headingStyle: "atx", codeBlockStyle: "fenced" });

interface UseComposeEditorArgs {
  fromAccountId: string;
  composeMode: string | null;
  composeReplyTo: Message | null;
  isReply: boolean;
  t: TFunction;
  restoredDraft: {
    editorMode?: EditorMode;
    rawSource?: string;
    richTextHtml?: string;
  } | null;
}

interface BuildComposeEditorContentArgs {
  composeMode: string | null;
  composeReplyTo: Message | null;
  isReply: boolean;
  signatureHtml: string;
  t: TFunction;
}

interface BuildReplyQuoteHtmlArgs {
  composeReplyTo: Message | null;
  t: TFunction;
}

interface ShouldApplyInitialEditorContentArgs {
  editorExists: boolean;
  initialized: boolean;
  signatureReady: boolean;
  hasRestoredDraft: boolean;
}

function escapeHtml(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function parseBodyHtml(html: string) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  return doc.body.innerHTML;
}

function decodeHtmlEntities(value: string) {
  const doc = new DOMParser().parseFromString(value, "text/html");
  return doc.body.textContent ?? "";
}

function startsWithEncodedHtmlTag(value: string) {
  return /^\s*(?:&lt;|&#0*60;|&#x0*3c;)\s*\/?[a-z][\w:-]*(?:\s|\/?(?:&gt;|&#0*62;|&#x0*3e;))/i.test(value);
}

function looksLikeHtml(value: string) {
  return /<\s*\/?[a-z][\w:-]*(?:\s|\/?>)/i.test(value);
}

export function extractComposeBodyHtml(html: string) {
  try {
    const source = html.trim();
    let body = parseBodyHtml(source);
    const decoded = decodeHtmlEntities(body).trim();

    if (startsWithEncodedHtmlTag(source) && decoded !== body.trim() && looksLikeHtml(decoded)) {
      body = parseBodyHtml(decoded);
    }

    return sanitizeHtml(body);
  } catch {
    return `<p>${escapeHtml(html)}</p>`;
  }
}

export function buildComposeEditorContent({
  composeMode,
  composeReplyTo,
  isReply,
  signatureHtml,
  t,
}: BuildComposeEditorContentArgs) {
  try {
    if (isReply && composeReplyTo) {
      return `<p><br></p>${signatureHtml}`;
    }
    if (composeMode === "forward" && composeReplyTo) {
      const sender = escapeHtml(composeReplyTo.from_name || composeReplyTo.from_address || "");
      const fwdSubject = escapeHtml(composeReplyTo.subject || "");
      const body = composeReplyTo.body_html_raw
        ? extractComposeBodyHtml(composeReplyTo.body_html_raw)
        : `<p>${escapeHtml(composeReplyTo.body_text || "")}</p>`;
      return `${signatureHtml}<br/><br/><p>${escapeHtml(t("compose.forwardedHeader"))}</p><p>${escapeHtml(t("compose.forwardedFrom", { sender }))}</p><p>${escapeHtml(t("compose.forwardedSubject", { subject: fwdSubject }))}</p>${body}`;
    }
    return signatureHtml;
  } catch (err) {
    console.error("[ComposeView] Failed to build editor content:", err);
    return "";
  }
}

export function buildReplyQuoteHtml({ composeReplyTo, t }: BuildReplyQuoteHtmlArgs) {
  if (!composeReplyTo) return "";

  const sender = escapeHtml(composeReplyTo.from_name || composeReplyTo.from_address || "");
  const dateStr = escapeHtml(new Date((composeReplyTo.date || 0) * 1000).toLocaleString());
  const body = composeReplyTo.body_html_raw
    ? extractComposeBodyHtml(composeReplyTo.body_html_raw)
    : `<p>${escapeHtml(composeReplyTo.body_text || "")}</p>`;
  const attribution = t("compose.quoteAttribution", { date: dateStr, sender });

  return `<blockquote class="compose-original-quote"><p>${escapeHtml(attribution)}</p>${body}</blockquote>`;
}

export function appendReplyQuoteHtml(bodyHtml: string, quotedReplyHtml: string) {
  const quote = quotedReplyHtml.trim();
  if (!quote) return bodyHtml;
  const body = bodyHtml.trim() || "<p><br></p>";
  return `${body}<br/><br/>${quote}`;
}

export function shouldApplyInitialEditorContent({
  editorExists,
  initialized,
  signatureReady,
  hasRestoredDraft,
}: ShouldApplyInitialEditorContentArgs) {
  if (!editorExists || initialized) return false;
  return hasRestoredDraft || signatureReady;
}

export function useComposeEditor({
  fromAccountId,
  composeMode,
  composeReplyTo,
  isReply,
  t,
  restoredDraft,
}: UseComposeEditorArgs) {
  const [editorMode, setEditorMode] = useState<EditorMode>(restoredDraft?.editorMode ?? "rich");
  const [rawSource, setRawSource] = useState(restoredDraft?.rawSource ?? "");
  const [richTextHtml, setRichTextHtml] = useState("");
  const [htmlPreview, setHtmlPreview] = useState(false);
  const [signature, setSignature] = useState("");
  const [signatureReady, setSignatureReady] = useState(Boolean(restoredDraft));
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const contentInitializedRef = useRef(false);

  useEffect(() => {
    if (restoredDraft) {
      setSignatureReady(true);
      return;
    }
    let cancelled = false;
    setSignatureReady(false);
    getSignature(fromAccountId)
      .then((loaded) => {
        if (!cancelled) setSignature(loaded);
      })
      .catch((err) => {
        console.warn("Failed to load signature:", err);
        if (!cancelled) setSignature("");
      })
      .finally(() => {
        if (!cancelled) setSignatureReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, [fromAccountId, restoredDraft]);

  // Build signature HTML block
  const signatureHtml = useMemo(() => {
    const sig = signature;
    if (!sig) return "";
    return `<br/><br/><div style="color:var(--color-text-secondary);font-size:13px">--<br/>${escapeHtml(sig).replace(/\n/g, "<br/>")}</div>`;
  }, [signature]);

  const editorContent = useMemo(() => {
    return buildComposeEditorContent({ composeMode, composeReplyTo, isReply, signatureHtml, t });
  }, [composeMode, composeReplyTo, isReply, t, signatureHtml]);
  const quotedReplyHtml = useMemo(() => {
    return isReply ? buildReplyQuoteHtml({ composeReplyTo, t }) : "";
  }, [composeReplyTo, isReply, t]);

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
    if (!shouldApplyInitialEditorContent({
      editorExists: Boolean(editor),
      initialized: contentInitializedRef.current,
      signatureReady,
      hasRestoredDraft: Boolean(restoredDraft),
    }) || !editor) return;
    // If restoring a draft, prefer its richTextHtml over generated editorContent
    if (restoredDraft?.richTextHtml) {
      editor.commands.setContent(restoredDraft.richTextHtml);
    } else if (editorContent) {
      editor.commands.setContent(editorContent);
    }
    if (isReply && !restoredDraft) {
      requestAnimationFrame(() => editor.commands.focus("start"));
    }
    contentInitializedRef.current = true;
  }, [editor, editorContent, isReply, restoredDraft, signatureReady]);

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
      if (newMode === "markdown") {
        setRawSource(turndown.turndown(editor.getHTML()));
      } else {
        setRawSource(editor.getHTML());
      }
    } else if (editorMode === "markdown") {
      if (newMode === "rich") {
        editor.commands.setContent(rawSource);
      } else {
        editor.commands.setContent(rawSource);
        setRawSource(editor.getHTML());
      }
    } else {
      if (newMode === "rich") {
        editor.commands.setContent(rawSource);
      } else {
        setRawSource(turndown.turndown(rawSource));
      }
    }

    setEditorMode(newMode);
  }

  return {
    editor,
    editorMode,
    rawSource,
    setRawSource,
    richTextHtml,
    htmlPreview,
    setHtmlPreview,
    switchMode,
    textareaRef,
    quotedReplyHtml,
  };
}
