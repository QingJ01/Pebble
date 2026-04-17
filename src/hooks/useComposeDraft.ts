import { useEffect, useRef, useCallback } from "react";
import { useComposeStore } from "@/stores/compose.store";
import { saveDraft } from "@/lib/api";

import type { EditorMode } from "./useComposeEditor";

const DRAFT_STORAGE_KEY = "pebble-compose-draft";

interface DraftData {
  accountId: string;
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

/**
 * Load the stored compose draft.
 *
 * When `validAccountIds` is provided, the draft is only returned if its
 * `accountId` is in that list — this prevents restoring a draft authored under
 * an account that has since been removed (or silently writing it to the
 * currently-active account, which was a real bug).
 */
export function loadDraftFromStorage(validAccountIds?: string[]): DraftData | null {
  try {
    const raw = localStorage.getItem(DRAFT_STORAGE_KEY);
    if (!raw) return null;
    const draft = JSON.parse(raw) as Partial<DraftData>;
    // Discard drafts older than 24 hours
    if (!draft.savedAt || Date.now() - draft.savedAt > 24 * 60 * 60 * 1000) {
      localStorage.removeItem(DRAFT_STORAGE_KEY);
      return null;
    }
    // Legacy drafts without accountId: drop them rather than guess.
    if (!draft.accountId || typeof draft.accountId !== "string") {
      localStorage.removeItem(DRAFT_STORAGE_KEY);
      return null;
    }
    if (validAccountIds && !validAccountIds.includes(draft.accountId)) {
      return null;
    }
    return draft as DraftData;
  } catch { return null; }
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
  /** True once the TipTap editor has mounted and populated richTextHtml with
   * its initial content (signature, quoted reply, etc.). Until this flips to
   * true, the snapshot would compare user edits against an empty string and
   * falsely report the compose as dirty. */
  editorReady: boolean;
}

export function useComposeDraft({
  to, cc, bcc, subject, rawSource, richTextHtml, editorMode, composeMode, fromAccountId, editorReady,
}: UseComposeDraftArgs) {
  // Snapshot the initial compose state so pre-populated reply/forward
  // fields don't immediately trigger the "unsaved draft" guard.
  // Deferred until the editor has rendered its initial content — taken once,
  // in an effect that runs after the first render post-editorReady.
  const initialSnapshot = useRef<{
    to: string[]; cc: string[]; bcc: string[]; subject: string;
    rawSource: string; richTextHtml: string;
  } | null>(null);
  useEffect(() => {
    if (!editorReady || initialSnapshot.current) return;
    initialSnapshot.current = {
      to: [...to], cc: [...cc], bcc: [...bcc], subject,
      rawSource, richTextHtml,
    };
    // Only depend on editorReady — we want this to run once after mount, not
    // each time the user edits.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorReady]);

  const arraysEqual = useCallback(
    (a: string[], b: string[]) => a.length === b.length && a.every((v, i) => v === b[i]),
    [],
  );

  // Ref to track the server-side draft ID across saves.
  // Scoped per account: when the user switches From, the prior draft_id
  // belongs to a different account and must not be reused.
  const draftIdRef = useRef<string | null>(null);
  const draftAccountRef = useRef<string | null>(null);
  if (draftAccountRef.current !== fromAccountId) {
    draftAccountRef.current = fromAccountId;
    draftIdRef.current = null;
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
      richTextHtml !== init.richTextHtml;
    useComposeStore.getState().setComposeDirty(userChanged);
  }, [arraysEqual, bcc, cc, rawSource, richTextHtml, subject, to, editorReady]);

  // Auto-save draft to localStorage and backend (debounced 3s)
  useEffect(() => {
    if (!composeMode) return;
    const timer = setTimeout(() => {
      const hasDraft = to.length > 0 || cc.length > 0 || bcc.length > 0 || subject.trim() || rawSource.trim() || richTextHtml.trim();
      if (hasDraft && fromAccountId) {
        saveDraftToStorage({ accountId: fromAccountId, to, cc, bcc, subject, rawSource, richTextHtml, editorMode });
        // Also save to backend under the current From account
        {
          // Pick body source based on current editor mode to avoid stale content.
          // For rich text, strip HTML tags to produce a plain-text fallback.
          const bodyText = editorMode === "rich"
            ? richTextHtml.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim()
            : rawSource;
          const bodyHtml = editorMode === "rich" ? richTextHtml : (editorMode === "html" ? rawSource : undefined);
          saveDraft({
            accountId: fromAccountId,
            to, cc, bcc, subject,
            bodyText,
            bodyHtml: bodyHtml || undefined,
            existingDraftId: draftIdRef.current || undefined,
          }).then((id) => {
            draftIdRef.current = id;
          }).catch((err) => {
            console.warn("Backend draft save failed:", err);
          });
        }
      }
    }, 3000);
    return () => clearTimeout(timer);
  }, [to, cc, bcc, subject, rawSource, richTextHtml, editorMode, composeMode, fromAccountId]);

  return { draftIdRef };
}
