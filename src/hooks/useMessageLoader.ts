import { useCallback, useEffect, useRef, useState } from "react";
import { getMessageWithHtml, getRenderedHtml } from "@/lib/api";
import { useUpdateFlagsMutation } from "@/hooks/mutations/useUpdateFlagsMutation";
import { sanitizeHtml } from "@/lib/sanitizeHtml";
import type { Message, RenderedHtml, PrivacyMode } from "@/lib/api";
import { describeError, extractErrorMessage, type DescribedError } from "@/lib/extractErrorMessage";

/* ─── Small in-memory cache (stale-while-revalidate) ───────────────────
   Re-opening a recently viewed message (or one prefetched by the list)
   renders instantly; the loader still refetches in the background so flags
   and content stay fresh. */

interface CacheEntry {
  message: Message;
  rendered: RenderedHtml;
  renderKey: string;
}

const CACHE_LIMIT = 30;
const cache = new Map<string, CacheEntry>();
const inflight = new Map<string, Promise<CacheEntry | null>>();

const renderKey = (id: string, mode: PrivacyMode) => `${id}:${JSON.stringify(mode)}`;

function remember(id: string, entry: CacheEntry) {
  cache.delete(id);
  cache.set(id, entry);
  while (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

function fetchEntry(id: string, mode: PrivacyMode): Promise<CacheEntry | null> {
  const key = renderKey(id, mode);
  const pending = inflight.get(key);
  if (pending) return pending;
  const promise = getMessageWithHtml(id, mode)
    .then((result) => {
      if (!result) return null;
      const [message, html] = result;
      const entry: CacheEntry = { message, rendered: { ...html, html: sanitizeHtml(html.html) }, renderKey: key };
      remember(id, entry);
      return entry;
    })
    .finally(() => inflight.delete(key));
  inflight.set(key, promise);
  return promise;
}

/** Warm the cache for a message the user is likely to open next (e.g. list neighbours). */
export function prefetchMessage(id: string, mode: PrivacyMode): Promise<void> {
  const cached = cache.get(id);
  if (cached && cached.renderKey === renderKey(id, mode)) return Promise.resolve();
  return fetchEntry(id, mode).then(() => undefined, () => undefined);
}

const revalidateListeners = new Set<(id: string) => void>();

/**
 * Ask any open reader showing `id` to reload it now (e.g. it vanished from the
 * list after a sync or an outside delete). A reader whose message no longer
 * exists switches to "not found" instead of showing stale content.
 */
export function revalidateMessage(id: string) {
  for (const listener of revalidateListeners) listener(id);
}

/** Drop cached messages (tests; or after bulk changes such as account removal). */
export function clearMessageCache(id?: string) {
  if (id) cache.delete(id);
  else cache.clear();
}

/**
 * Loads a message and its rendered HTML.
 *
 * While the next message loads, the previous `message`/`rendered` are kept
 * (`isStale` is true) so the view can dim them instead of flashing a skeleton.
 */
export function useMessageLoader(messageId: string | null, privacyMode: PrivacyMode) {
  const flagsMutation = useUpdateFlagsMutation();
  const [message, setMessageState] = useState<Message | null>(null);
  const [rendered, setRendered] = useState<RenderedHtml | null>(null);
  /** Raw error message (never "[object Object]"). */
  const [error, setErrorMessage] = useState<string | null>(null);
  /** Localized description of the same error for display. */
  const [errorInfo, setErrorInfo] = useState<DescribedError | null>(null);
  const setError = (err: unknown) => {
    if (err === null) {
      setErrorMessage(null);
      setErrorInfo(null);
      return;
    }
    setErrorMessage(extractErrorMessage(err));
    setErrorInfo(describeError(err, { fallbackKey: "errors.loadMessage" }));
  };
  const [loading, setLoading] = useState(true);
  const privacyModeRef = useRef(privacyMode);
  const renderedKeyRef = useRef<string | null>(null);

  useEffect(() => {
    privacyModeRef.current = privacyMode;
  }, [privacyMode]);

  /** Update the displayed message (e.g. after a flag change) and the cache. */
  const setMessage = useCallback((next: Message | null | ((prev: Message | null) => Message | null)) => {
    setMessageState((prev) => {
      const value = typeof next === "function" ? next(prev) : next;
      if (value) {
        const entry = cache.get(value.id);
        if (entry) cache.set(value.id, { ...entry, message: value });
      }
      return value;
    });
  }, []);

  // Reload on request (revalidateMessage) while this message is open.
  useEffect(() => {
    if (!messageId) return;
    let cancelled = false;
    const listener = (id: string) => {
      if (id !== messageId) return;
      const mode = privacyModeRef.current;
      fetchEntry(id, mode).then((entry) => {
        if (cancelled) return;
        if (!entry) {
          cache.delete(id);
          setMessageState(null);
          setRendered(null);
          renderedKeyRef.current = null;
          return;
        }
        setMessageState(entry.message);
        if (renderedKeyRef.current === entry.renderKey) setRendered(entry.rendered);
      }, () => { /* keep what is shown; the next action reports errors */ });
    };
    revalidateListeners.add(listener);
    return () => {
      cancelled = true;
      revalidateListeners.delete(listener);
    };
  }, [messageId]);

  // Load message when messageId changes
  useEffect(() => {
    if (!messageId) {
      setMessageState(null);
      setRendered(null);
      setError(null);
      renderedKeyRef.current = null;
      setLoading(false);
      return;
    }

    let cancelled = false;
    const initialPrivacyMode = privacyModeRef.current;
    const initialRenderKey = renderKey(messageId, initialPrivacyMode);
    const cached = cache.get(messageId);
    setError(null);

    // Only reuse cached HTML rendered under the current privacy mode: HTML
    // rendered under a looser mode could load remote images / trackers.
    const cachedRenderUsable = !!cached && cached.renderKey === initialRenderKey;

    if (cached && cachedRenderUsable) {
      // Show the cached copy immediately, then revalidate in the background.
      setMessageState(cached.message);
      setRendered(cached.rendered);
      renderedKeyRef.current = cached.renderKey;
      setLoading(false);
    } else if (cached) {
      // Headers are safe to show; the body waits for a render in this mode.
      setMessageState(cached.message);
      setRendered(null);
      renderedKeyRef.current = null;
      setLoading(true);
    } else {
      // Keep the previous message on screen (dimmed by the view) meanwhile.
      setLoading(true);
      renderedKeyRef.current = null;
    }

    async function load() {
      try {
        const entry = await fetchEntry(messageId!, initialPrivacyMode);
        if (cancelled) return;
        if (!entry) {
          // The message is gone (deleted, moved by sync): show "not found"
          // instead of leaving the previous message on screen as stale.
          cache.delete(messageId!);
          setMessageState(null);
          setRendered(null);
          renderedKeyRef.current = null;
          return;
        }
        setMessageState(entry.message);
        // A privacy re-render may already have replaced the cached HTML.
        if (!cachedRenderUsable || renderedKeyRef.current === initialRenderKey) {
          renderedKeyRef.current = initialRenderKey;
          setRendered(entry.rendered);
        }

        if (!cancelled && !entry.message.is_read) {
          flagsMutation.mutate({ messageId: messageId!, isRead: true });
          remember(messageId!, { ...entry, message: { ...entry.message, is_read: true } });
        }
      } catch (err) {
        if (cancelled) return;
        if (cachedRenderUsable) {
          console.warn("Background message refresh failed:", err);
          return;
        }
        setMessageState(null);
        setRendered(null);
        setError(err);
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    load();
    return () => { cancelled = true; };
  }, [messageId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Re-render HTML when privacy mode changes (without reloading message)
  useEffect(() => {
    if (!message || !messageId || message.id !== messageId) return;
    const currentRenderKey = renderKey(messageId, privacyMode);
    if (renderedKeyRef.current === currentRenderKey) return;
    // The initial load for this id is still in flight with this mode.
    if (renderedKeyRef.current === null) return;

    let cancelled = false;
    setRendered(null);
    setError(null);

    getRenderedHtml(messageId, privacyMode)
      .then((html) => {
        if (!cancelled) {
          renderedKeyRef.current = currentRenderKey;
          setRendered({ ...html, html: sanitizeHtml(html.html) });
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err);
        }
      });

    return () => { cancelled = true; };
  }, [privacyMode, messageId, message]); // eslint-disable-line react-hooks/exhaustive-deps

  const isStale = message !== null && message.id !== messageId;
  return { message, setMessage, rendered, loading, isStale, error, errorInfo };
}
