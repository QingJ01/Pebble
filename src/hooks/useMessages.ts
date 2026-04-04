import { useEffect } from "react";
import { useMailStore } from "@/stores/mail.store";

export function useMessages() {
  const messages = useMailStore((s) => s.messages);
  const loading = useMailStore((s) => s.loadingMessages);
  const selectedMessageId = useMailStore((s) => s.selectedMessageId);
  const setSelectedMessage = useMailStore((s) => s.setSelectedMessage);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const fetchMessages = useMailStore((s) => s.fetchMessages);

  useEffect(() => {
    if (activeFolderId) {
      fetchMessages(activeFolderId);
    }
  }, [activeFolderId, fetchMessages]);

  return { messages, loading, selectedMessageId, setSelectedMessage };
}
