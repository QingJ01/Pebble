import { useMailStore } from "@/stores/mail.store";
import { useUIStore } from "@/stores/ui.store";
import { useComposeStore } from "@/stores/compose.store";
import { getMessage } from "@/lib/api";

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

/** Open the reply composer for a message (loads it first). */
export function useReplyToMessage() {
  const setActiveAccountId = useMailStore((s) => s.setActiveAccountId);
  return async (accountId: string | null | undefined, messageId: string | null | undefined) => {
    if (!messageId) return false;
    const message = await getMessage(messageId).catch(() => null);
    if (!message) return false;
    if (accountId) setActiveAccountId(accountId);
    useComposeStore.getState().openCompose("reply", message);
    return true;
  };
}
