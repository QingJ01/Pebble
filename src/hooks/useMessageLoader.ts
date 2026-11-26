import { useEffect, useRef, useState } from "react";
import { getMessageWithHtml, getRenderedHtml } from "@/lib/api";
import { useUpdateFlagsMutation } from "@/hooks/mutations/useUpdateFlagsMutation";
import { sanitizeHtml } from "@/lib/sanitizeHtml";
import type { Message, RenderedHtml, PrivacyMode } from "@/lib/api";
import { describeError, extractErrorMessage, type DescribedError } from "@/lib/extractErrorMessage";

export function useMessageLoader(messageId: string | null, privacyMode: PrivacyMode) {
  const flagsMutation = useUpdateFlagsMutation();
  const [message, setMessage] = useState<Message | null>(null);
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

  const renderKey = (id: string, mode: PrivacyMode) => `${id}:${JSON.stringify(mode)}`;

  // Load message when messageId changes
  useEffect(() => {
    if (!messageId) {
      setMessage(null);
      setRendered(null);
      setError(null);
      renderedKeyRef.current = null;
      setLoading(false);
      return;
    }

    let cancelled = false;
    const initialPrivacyMode = privacyModeRef.current;
    const initialRenderKey = renderKey(messageId, initialPrivacyMode);
    setLoading(true);
    setMessage(null);
    setRendered(null);
    setError(null);
    renderedKeyRef.current = null;

    async function load() {
      try {
        const result = await getMessageWithHtml(messageId!, initialPrivacyMode);
        if (cancelled || !result) return;
        const [msg, html] = result;
        setMessage(msg);
        renderedKeyRef.current = initialRenderKey;
        setRendered({ ...html, html: sanitizeHtml(html.html) });

        if (!cancelled && !msg.is_read) {
          flagsMutation.mutate({ messageId: messageId!, isRead: true });
        }
      } catch (err) {
        if (!cancelled) {
          setError(err);
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
    if (!message || !messageId || message.id !== messageId) return;
    const currentRenderKey = renderKey(messageId, privacyMode);
    if (renderedKeyRef.current === currentRenderKey) return;

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

  return { message, setMessage, rendered, loading, error, errorInfo };
}
