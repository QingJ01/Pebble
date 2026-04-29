import { useEffect, useRef, useCallback } from "react";
import { useComposeStore } from "@/stores/compose.store";
import { saveDraft } from "@/lib/api";
import { hasComposeDraft, type ComposeAttachment } from "@/features/compose/compose-draft";

import type { EditorMode } from "./useComposeEditor";

const DRAFT_STORAGE_KEY = "pebble-compose-draft";

export interface DraftData {
  accountId: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  rawSource: string;
  richTextHtml: string;
  editorMode: EditorMode;
  attachments: ComposeAttachment[];
  savedAt: number;
}

/**
 * LocalStorage drafts are no longer restored because they contain plaintext
 * message bodies and attachment paths. Backend drafts remain the persistence
 * mechanism; this only clears legacy plaintext data left by older builds.
 */
export function loadDraftFromStorage(validAccountIds?: string[]): DraftData | null {
  void validAccountIds;
  try {
    localStorage.removeItem(DRAFT_STORAGE_KEY);
  } catch { /* ignore legacy cleanup failure */ }
  return null;
}

export function clearDraftStorage() {
  localStorage.removeItem(DRAFT_STORAGE_KEY);
}

interface UseComposeDraftArgs {
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  rawSource: string;
  richTextHtml: string;
  editorMode: EditorMode;
  composeMode: string | null;
  fromAccountId: string | null;
  attachments: ComposeAttachment[];
  /** True once the TipTap editor has mounted and populated richTextHtml with
   * its initial content (signature, quoted reply, etc.). Until this flips to
   * true, the snapshot would compare user edits against an empty string and
   * falsely report the compose as dirty. */
  editorReady: boolean;
}

export function useComposeDraft({
  to, cc, bcc, subject, rawSource, richTextHtml, editorMode, composeMode, fromAccountId, attachments, editorReady,
}: UseComposeDraftArgs) {
  // Snapshot the initial compose state so pre-populated reply/forward
  // fields don't immediately trigger the "unsaved draft" guard.
  // Deferred until the editor has rendered its initial content - taken once,
  // in an effect that runs after the first render post-editorReady.
  const initialSnapshot = useRef<{
    to: string[]; cc: string[]; bcc: string[]; subject: string;
    rawSource: string; richTextHtml: string; attachments: ComposeAttachment[];
  } | null>(null);
  useEffect(() => {
    if (!editorReady || initialSnapshot.current) return;
    initialSnapshot.current = {
      to: [...to], cc: [...cc], bcc: [...bcc], subject,
      rawSource, richTextHtml, attachments: attachments.map((a) => ({ ...a })),
    };
    // Only depend on editorReady - we want this to run once after mount, not
    // each time the user edits.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorReady]);

  const arraysEqual = useCallback(
    (a: string[], b: string[]) => a.length === b.length && a.every((v, i) => v === b[i]),
    [],
  );
  const attachmentsEqual = useCallback(
    (a: ComposeAttachment[], b: ComposeAttachment[]) =>
      a.length === b.length &&
      a.every((v, i) => v.name === b[i]?.name && v.path === b[i]?.path && v.size === b[i]?.size),
    [],
  );

  // Ref to track the server-side draft ID across saves.
  // Scoped per account: when the user switches From, the prior draft_id
  // belongs to a different account and must not be reused.
  const draftIdRef = useRef<string | null>(null);
  const draftAccountRef = useRef<string | null>(null);
  const draftIdsByAccountRef = useRef<Record<string, string>>({});
  const saveGenerationByAccountRef = useRef<Record<string, number>>({});
  if (draftAccountRef.current !== fromAccountId) {
    draftAccountRef.current = fromAccountId;
    draftIdRef.current = fromAccountId ? draftIdsByAccountRef.current[fromAccountId] ?? null : null;
  }

  // Track dirty state for leave-protection.
  // Skip until the initial snapshot is captured (i.e. editor ready).
  useEffect(() => {
    const init = initialSnapshot.current;
    if (!init) return;
    const userChanged =
      !arraysEqual(to, init.to) ||
      !arraysEqual(cc, init.cc) ||
      !arraysEqual(bcc, init.bcc) ||
      subject !== init.subject ||
      rawSource !== init.rawSource ||
      richTextHtml !== init.richTextHtml ||
      !attachmentsEqual(attachments, init.attachments);
    useComposeStore.getState().setComposeDirty(userChanged);
  }, [arraysEqual, attachments, attachmentsEqual, bcc, cc, rawSource, richTextHtml, subject, to, editorReady]);

  // Auto-save draft to backend (debounced 3s). Do not persist plaintext draft
  // data in WebView storage.
  useEffect(() => {
    if (!composeMode || !editorReady || !initialSnapshot.current) return;
    const timer = setTimeout(() => {
      const draftAttachments = attachments.filter((attachment) =>
        attachment.path.trim().length > 0 || attachment.name.trim().length > 0,
      );
      const hasDraft = hasComposeDraft({
        to, cc, bcc, subject, rawSource, richTextHtml, attachments: draftAttachments,
      });
      if (hasDraft && fromAccountId) {
        const accountIdAtSave = fromAccountId;
        const generation = (saveGenerationByAccountRef.current[accountIdAtSave] ?? 0) + 1;
        saveGenerationByAccountRef.current[accountIdAtSave] = generation;
        // Save to backend under the current From account.
        {
          // Pick body source based on current editor mode to avoid stale content.
          // For rich text, strip HTML tags to produce a plain-text fallback.
          const bodyText = editorMode === "rich"
            ? richTextHtml.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim()
            : rawSource;
          const bodyHtml = editorMode === "rich" ? richTextHtml : (editorMode === "html" ? rawSource : undefined);
          saveDraft({
            accountId: accountIdAtSave,
            to, cc, bcc, subject,
            bodyText,
            bodyHtml: bodyHtml || undefined,
            attachmentPaths: draftAttachments.map((attachment) => attachment.path).filter(Boolean),
            existingDraftId: draftIdsByAccountRef.current[accountIdAtSave] || undefined,
          }).then((id) => {
            if (saveGenerationByAccountRef.current[accountIdAtSave] === generation) {
              draftIdsByAccountRef.current[accountIdAtSave] = id;
              if (draftAccountRef.current === accountIdAtSave) {
                draftIdRef.current = id;
              }
            }
          }).catch((err) => {
            console.warn("Backend draft save failed:", err);
          });
        }
      }
    }, 3000);
    return () => clearTimeout(timer);
  }, [attachments, to, cc, bcc, subject, rawSource, richTextHtml, editorMode, composeMode, fromAccountId, editorReady]);

  return { draftIdRef, draftIdsByAccountRef };
}
