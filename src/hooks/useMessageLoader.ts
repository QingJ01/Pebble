import { useEffect, useState } from "react";
import { getMessageWithHtml, getRenderedHtml } from "@/lib/api";
import { useUpdateFlagsMutation } from "@/hooks/mutations/useUpdateFlagsMutation";
import type { Message, RenderedHtml, PrivacyMode } from "@/lib/api";

export function useMessageLoader(messageId: string | null, privacyMode: PrivacyMode) {
  const flagsMutation = useUpdateFlagsMutation();
  const [message, setMessage] = useState<Message | null>(null);
  const [rendered, setRendered] = useState<RenderedHtml | null>(null);
  const [loading, setLoading] = useState(true);

  // Load message when messageId changes
  useEffect(() => {
    if (!messageId) {
      setMessage(null);
      setRendered(null);
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setMessage(null);
    setRendered(null);

    async function load() {
      try {
        const result = await getMessageWithHtml(messageId!, privacyMode);
        if (cancelled || !result) return;
        const [msg, html] = result;
        setMessage(msg);
        setRendered(html);

        if (!msg.is_read) {
          flagsMutation.mutate({ messageId: messageId!, isRead: true });
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    load();
    return () => { cancelled = true; };
  }, [messageId, privacyMode]); // eslint-disable-line react-hooks/exhaustive-deps

  // Re-render HTML when privacy mode changes (without reloading message)
  useEffect(() => {
    if (!message || !messageId) return;
    let cancelled = false;
    setRendered(null);

    getRenderedHtml(messageId, privacyMode).then((html) => {
      if (!cancelled) setRendered(html);
    });

    return () => { cancelled = true; };
  }, [privacyMode, messageId, message]); // eslint-disable-line react-hooks/exhaustive-deps

  return { message, setMessage, rendered, loading };
}
