import { useEffect, useState } from "react";
import { ArrowLeft, Clock, Languages, Reply, ReplyAll, Forward, Star, Archive, Trash2, LayoutGrid, RotateCcw } from "lucide-react";
import { getMessageWithHtml, getRenderedHtml, translateText, trustSender, archiveMessage, deleteMessage, restoreMessage } from "@/lib/api";
import { useQueryClient } from "@tanstack/react-query";
import { useUIStore } from "@/stores/ui.store";
import { useKanbanStore } from "@/stores/kanban.store";
import { useToastStore } from "@/stores/toast.store";
import { useTranslation } from "react-i18next";
import { useUpdateFlagsMutation } from "@/hooks/mutations/useUpdateFlagsMutation";
import { sanitizeHtml } from "@/lib/sanitizeHtml";
import type { Message, MessageSummary, RenderedHtml, PrivacyMode, TranslateResult } from "@/lib/api";
import { MessageDetailSkeleton } from "./Skeleton";
import PrivacyBanner from "./PrivacyBanner";
import AttachmentList from "./AttachmentList";
import SnoozePopover from "../features/inbox/SnoozePopover";
import ConfirmDialog from "./ConfirmDialog";
import { ShadowDomEmail } from "./ShadowDomEmail";

import TranslatePopover from "../features/translate/TranslatePopover";

// Translation cache: avoids re-translating on toggle or revisit (capped at 20 entries)
const translationCache = new Map<string, TranslateResult & { _isHtml?: boolean }>();
const TRANSLATION_CACHE_MAX = 20;
const CHUNK_SIZE = 30; // Max text nodes per translation request

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
  const openCompose = useUIStore((s) => s.openCompose);
  const queryClient = useQueryClient();
  const flagsMutation = useUpdateFlagsMutation();
  const [message, setMessage] = useState<Message | null>(null);
  const [rendered, setRendered] = useState<RenderedHtml | null>(null);
  const [loading, setLoading] = useState(true);
  const [privacyMode, setPrivacyMode] = useState<PrivacyMode>(() => {
    const saved = localStorage.getItem("pebble-privacy-mode");
    if (saved === "off") return "Off";
    if (saved === "strict") return "Strict";
    if (saved === "relaxed") return "LoadOnce";
    return "LoadOnce";
  });
  const [showSnooze, setShowSnooze] = useState(false);
  const [showTranslate, setShowTranslate] = useState<{ text: string; position: { x: number; y: number } } | null>(null);
  const [bilingualMode, setBilingualMode] = useState(false);
  const [bilingualResult, setBilingualResult] = useState<TranslateResult | null>(null);
  const [bilingualLoading, setBilingualLoading] = useState(false);
  const [showKanbanPicker, setShowKanbanPicker] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const inKanban = useKanbanStore((s) => s.cardIdSet.has(messageId));

  // Load message when messageId changes
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setMessage(null);
    setRendered(null);
    setBilingualMode(false);
    setBilingualResult(null);

    async function load() {
      try {
        // Single IPC call: get message + rendered HTML together
        const result = await getMessageWithHtml(messageId, privacyMode);
        if (cancelled || !result) return;
        const [msg, html] = result;
        setMessage(msg);
        setRendered(html);

        if (!msg.is_read) {
          flagsMutation.mutate({ messageId, isRead: true });
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    load();
    return () => { cancelled = true; };
  }, [messageId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Re-render HTML when privacy mode changes (without reloading message)
  useEffect(() => {
    // Skip initial render (handled by message loading effect above)
    if (!message) return;
    let cancelled = false;
    setRendered(null);

    getRenderedHtml(messageId, privacyMode).then((html) => {
      if (!cancelled) setRendered(html);
    });

    return () => { cancelled = true; };
  }, [privacyMode]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!showSnooze) return;
    function handleClick() {
      setShowSnooze(false);
    }
    document.addEventListener("click", handleClick);
    return () => document.removeEventListener("click", handleClick);
  }, [showSnooze]);

  useEffect(() => {
    if (!showTranslate) return;
    function handleClick() {
      setShowTranslate(null);
    }
    document.addEventListener("click", handleClick);
    return () => document.removeEventListener("click", handleClick);
  }, [showTranslate]);

  useEffect(() => {
    if (!showKanbanPicker) return;
    function handleClick() { setShowKanbanPicker(false); }
    // Delay to avoid catching the same click that opened the picker
    const timer = setTimeout(() => {
      document.addEventListener("click", handleClick);
    }, 0);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("click", handleClick);
    };
  }, [showKanbanPicker]);

  async function handleAddToKanban(column: "todo" | "waiting" | "done") {
    if (!message) return;
    setShowKanbanPicker(false);
    try {
      await useKanbanStore.getState().addCard(message.id, column);
      useToastStore.getState().addToast({
        message: t("messageActions.kanbanSuccess", "Added to kanban board"),
        type: "success",
      });
    } catch {
      useToastStore.getState().addToast({
        message: t("messageActions.kanbanFailed", "Failed to add to kanban"),
        type: "error",
      });
    }
  }

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

  async function handleBilingualToggle() {
    if (bilingualMode) {
      setBilingualMode(false);
      return;
    }
    if (!message) return;

    const uiLang = localStorage.getItem("pebble-language") || "zh";
    const cacheKey = `${messageId}:${uiLang}`;

    // Check cache first
    const cached = translationCache.get(cacheKey);
    if (cached) {
      setBilingualResult(cached);
      setBilingualMode(true);
      return;
    }

    setBilingualMode(true);
    setBilingualLoading(true);
    try {
      const hasHtml = !!(rendered && rendered.html);

      if (hasHtml) {
        // HTML email: translate in chunks while preserving layout
        const doc = new DOMParser().parseFromString(sanitizeHtml(rendered!.html), "text/html");
        const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
        const textNodes: Text[] = [];
        let n: Text | null;
        while ((n = walker.nextNode() as Text | null)) {
          if (n.textContent?.trim()) textNodes.push(n);
        }

        // Translate in chunks to avoid timeouts on long emails.
        // Uses a unique separator so we can reliably split the response,
        // with numbered-index fallback for services that preserve them.
        const SEP = "\n⸻\n";
        for (let start = 0; start < textNodes.length; start += CHUNK_SIZE) {
          const chunk = textNodes.slice(start, start + CHUNK_SIZE);
          const batch = chunk.map((nd) => nd.textContent!.trim()).join(SEP);
          const result = await translateText(batch, "auto", uiLang);

          // Split on separator; if the service preserved it, we get exact mapping
          const parts = result.translated.split("⸻").map((s) => s.trim()).filter(Boolean);
          if (parts.length === chunk.length) {
            // Exact 1:1 mapping
            for (let i = 0; i < chunk.length; i++) {
              chunk[i].textContent = parts[i];
            }
          } else {
            // Fallback: replace the entire chunk's text with the translated result
            // Split by newlines and try positional matching
            const lines = result.translated.split("\n").map((s) => s.trim()).filter(Boolean);
            for (let i = 0; i < Math.min(chunk.length, lines.length); i++) {
              chunk[i].textContent = lines[i];
            }
          }
          // Show progressive results after each chunk
          const partial = { translated: sanitizeHtml(doc.body.innerHTML), segments: [], _isHtml: true } as TranslateResult & { _isHtml?: boolean };
          setBilingualResult(partial);
        }

        const final_ = { translated: sanitizeHtml(doc.body.innerHTML), segments: [], _isHtml: true } as TranslateResult & { _isHtml?: boolean };
        setBilingualResult(final_);
        if (translationCache.size >= TRANSLATION_CACHE_MAX) {
          translationCache.delete(translationCache.keys().next().value!);
        }
        translationCache.set(cacheKey, final_);
      } else {
        // Plain text email
        const textToTranslate = message.body_text
          || new DOMParser().parseFromString(message.body_html_raw || "", "text/html").body.textContent
          || "";
        const result = { ...await translateText(textToTranslate, "auto", uiLang), _isHtml: false } as TranslateResult & { _isHtml?: boolean };
        setBilingualResult(result);
        if (translationCache.size >= TRANSLATION_CACHE_MAX) {
          translationCache.delete(translationCache.keys().next().value!);
        }
        translationCache.set(cacheKey, result);
      }
    } catch (err) {
      console.error("Translation failed:", err);
    } finally {
      setBilingualLoading(false);
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
          <div style={{ position: "relative", marginLeft: "auto", flexShrink: 0 }}>
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
        <div style={{ display: "flex", gap: "2px", padding: "4px 16px 4px 48px", position: "relative" }}>
          {([
            { icon: Reply, label: t("messageActions.reply"), action: () => openCompose("reply", message) },
            { icon: ReplyAll, label: t("messageActions.replyAll"), action: () => openCompose("reply-all", message) },
            { icon: Forward, label: t("messageActions.forward"), action: () => openCompose("forward", message) },
            {
              icon: Star,
              label: message.is_starred ? t("messageActions.unstar") : t("messageActions.star"),
              action: () => {
                flagsMutation.mutate(
                  { messageId: message.id, isStarred: !message.is_starred },
                  { onSuccess: () => setMessage({ ...message, is_starred: !message.is_starred }) },
                );
              },
              active: message.is_starred,
            },
            {
              icon: folderRole === "archive" ? RotateCcw : Archive,
              label: folderRole === "archive"
                ? t("messageActions.unarchive", "Unarchive")
                : t("messageActions.archive"),
              action: async () => {
                queryClient.setQueriesData<MessageSummary[]>({ queryKey: ["messages"] }, (old) => old?.filter((m) => m.id !== message.id));
                onBack();
                try {
                  const result = await archiveMessage(message.id);
                  if (result === "skipped") return;
                  queryClient.invalidateQueries({ queryKey: ["messages"] });
                  queryClient.invalidateQueries({ queryKey: ["threads"] });
                  if (result === "unarchived") {
                    useToastStore.getState().addToast({
                      message: t("messageActions.unarchiveSuccess", "Message moved to inbox"),
                      type: "success",
                    });
                  } else {
                    useToastStore.getState().addToast({
                      message: t("messageActions.archiveSuccess", "Message archived"),
                      type: "success",
                    });
                  }
                } catch {
                  queryClient.invalidateQueries({ queryKey: ["messages"] });
                  useToastStore.getState().addToast({
                    message: folderRole === "archive"
                      ? t("messageActions.unarchiveFailed", "Failed to unarchive")
                      : t("messageActions.archiveFailed", "Failed to archive message"),
                    type: "error",
                  });
                }
              },
            },
            ...(folderRole === "trash" ? [{
              icon: RotateCcw,
              label: t("messageActions.restore", "Restore"),
              action: async () => {
                queryClient.setQueriesData<MessageSummary[]>({ queryKey: ["messages"] }, (old) => old?.filter((m) => m.id !== message.id));
                onBack();
                try {
                  await restoreMessage(message.id);
                  queryClient.invalidateQueries({ queryKey: ["messages"] });
                  useToastStore.getState().addToast({
                    message: t("messageActions.restoreSuccess", "Message restored to inbox"),
                    type: "success",
                  });
                } catch {
                  queryClient.invalidateQueries({ queryKey: ["messages"] });
                  useToastStore.getState().addToast({
                    message: t("messageActions.restoreFailed", "Failed to restore message"),
                    type: "error",
                  });
                }
              },
            }] : []),
            {
              icon: Trash2,
              label: t("messageActions.delete"),
              action: () => setShowDeleteConfirm(true),
            },
          ] as Array<{ icon: React.ComponentType<{ size?: number }>; label: string; action: () => void; active?: boolean; disabled?: boolean }>).map(({ icon: Icon, label, action, active, disabled }, i) => (
            <button
              key={i}
              onClick={disabled ? undefined : action}
              title={disabled ? label + " (coming soon)" : label}
              aria-label={label}
              style={{
                background: "none",
                border: "none",
                cursor: disabled ? "default" : "pointer",
                padding: "6px 8px",
                borderRadius: "4px",
                color: active ? "var(--color-accent)" : "var(--color-text-secondary)",
                display: "flex",
                alignItems: "center",
                transition: "background-color 0.12s ease, color 0.12s ease",
                opacity: disabled ? 0.35 : 1,
              }}
              onMouseEnter={disabled ? undefined : (e) => { (e.currentTarget as HTMLButtonElement).style.backgroundColor = "var(--color-bg-hover)"; }}
              onMouseLeave={disabled ? undefined : (e) => { (e.currentTarget as HTMLButtonElement).style.backgroundColor = "transparent"; }}
            >
              <Icon size={16} />
            </button>
          ))}
          {/* Kanban button + picker */}
          <div style={{ position: "relative" }}>
            <button
              onClick={() => setShowKanbanPicker(true)}
              title={t("messageActions.addToKanban")}
              aria-label={t("messageActions.addToKanban")}
              style={{
                background: "none",
                border: "none",
                cursor: "pointer",
                padding: "6px 8px",
                borderRadius: "4px",
                color: inKanban ? "var(--color-accent)" : "var(--color-text-secondary)",
                display: "flex",
                alignItems: "center",
                transition: "background-color 0.12s ease, color 0.12s ease",
              }}
              onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.backgroundColor = "var(--color-bg-hover)"; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.backgroundColor = "transparent"; }}
            >
              <LayoutGrid size={16} />
            </button>
            {showKanbanPicker && (
              <div
                onClick={(e) => e.stopPropagation()}
                style={{
                  position: "absolute",
                  right: "0px",
                  top: "100%",
                  marginTop: "4px",
                  backgroundColor: "var(--color-bg)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "8px",
                  boxShadow: "0 4px 12px rgba(0,0,0,0.12)",
                  padding: "4px",
                  zIndex: 10,
                  minWidth: "140px",
                }}
              >
                {([
                  { col: "todo" as const, label: t("kanban.todo", "To Do") },
                  { col: "waiting" as const, label: t("kanban.waiting", "Waiting") },
                  { col: "done" as const, label: t("kanban.done", "Done") },
                ]).map(({ col, label }) => (
                  <button
                    key={col}
                    onClick={() => handleAddToKanban(col)}
                    style={{
                      display: "block",
                      width: "100%",
                      padding: "8px 12px",
                      border: "none",
                      background: "transparent",
                      cursor: "pointer",
                      fontSize: "13px",
                      color: "var(--color-text-primary)",
                      borderRadius: "4px",
                      textAlign: "left",
                    }}
                    onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = "var(--color-bg-hover)"; }}
                    onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = "transparent"; }}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
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
        <TranslatePopover
          text={showTranslate.text}
          position={showTranslate.position}
          onClose={() => setShowTranslate(null)}
        />
      )}

      {showDeleteConfirm && (
        <ConfirmDialog
          title={t("messageActions.delete", "Delete")}
          message={t("messageActions.deleteConfirm", "Move this message to trash?")}
          confirmLabel={t("common.delete", "Delete")}
          destructive
          onCancel={() => setShowDeleteConfirm(false)}
          onConfirm={async () => {
            setShowDeleteConfirm(false);
            queryClient.setQueriesData<MessageSummary[]>({ queryKey: ["messages"] }, (old) => old?.filter((m) => m.id !== message.id));
            onBack();
            try {
              await deleteMessage(message.id);
              queryClient.invalidateQueries({ queryKey: ["messages"] });
              queryClient.invalidateQueries({ queryKey: ["threads"] });
              useToastStore.getState().addToast({
                message: t("messageActions.deleteSuccess", "Message deleted"),
                type: "success",
              });
            } catch {
              queryClient.invalidateQueries({ queryKey: ["messages"] });
              useToastStore.getState().addToast({
                message: t("messageActions.deleteFailed", "Failed to delete message"),
                type: "error",
              });
            }
          }}
        />
      )}
    </div>
  );
}
