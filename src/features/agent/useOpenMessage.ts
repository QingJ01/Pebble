import { useMailStore } from "@/stores/mail.store";
import { useUIStore } from "@/stores/ui.store";

/** Open a message in the inbox (switching to its account). */
export function useOpenMessage() {
  const setActiveAccountId = useMailStore((s) => s.setActiveAccountId);
  const openMessageInInbox = useUIStore((s) => s.openMessageInInbox);
  return (accountId: string | null | undefined, messageId: string | null | undefined) => {
    if (!messageId) return;
    if (accountId) setActiveAccountId(accountId);
    openMessageInInbox(messageId);
  };
}
