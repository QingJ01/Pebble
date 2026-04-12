import { useState, useEffect } from "react";
import type { Message } from "@/lib/ipc-types";
import type { Account } from "@/lib/ipc-types";

interface DraftRecipients {
  to?: string[];
  cc?: string[];
  bcc?: string[];
}

interface UseComposeRecipientsArgs {
  composeMode: string | null;
  composeReplyTo: Message | null;
  accounts: Account[];
  activeAccountId: string | null;
  restoredDraft: DraftRecipients | null;
}

export function useComposeRecipients({
  composeMode,
  composeReplyTo,
  accounts,
  activeAccountId,
  restoredDraft,
}: UseComposeRecipientsArgs) {
  const [fromAccountId, setFromAccountId] = useState(activeAccountId || "");
  const currentAccount = accounts.find((a) => a.id === fromAccountId);
  const myEmail = currentAccount?.email || "";

  const [to, setTo] = useState<string[]>(() => {
    if (restoredDraft) return restoredDraft.to ?? [];
    if (!composeReplyTo) return [];
    if (composeMode === "reply") return [composeReplyTo.from_address];
    if (composeMode === "reply-all") {
      const all = [composeReplyTo.from_address, ...composeReplyTo.to_list.map((a) => a.address)];
      return [...new Set(all)].filter((addr) => addr !== myEmail);
    }
    return [];
  });

  const [cc, setCc] = useState<string[]>(() => {
    if (restoredDraft) return restoredDraft.cc ?? [];
    if (composeMode === "reply-all" && composeReplyTo) {
      return composeReplyTo.cc_list.map((a) => a.address).filter((addr) => addr !== myEmail);
    }
    return [];
  });

  const [bcc, setBcc] = useState<string[]>(restoredDraft?.bcc ?? []);
  const [showCc, setShowCc] = useState(() => cc.length > 0);
  const [showBcc, setShowBcc] = useState(false);

  // Re-calculate from/to/cc once accounts data loads (fixes reply-all with async data)
  useEffect(() => {
    if (accounts.length === 0) return;
    const newAccountId = (!fromAccountId || !accounts.find((a) => a.id === fromAccountId))
      ? (activeAccountId || accounts[0]?.id || "")
      : fromAccountId;
    if (newAccountId !== fromAccountId) {
      setFromAccountId(newAccountId);
    }
    const resolvedEmail = accounts.find((a) => a.id === newAccountId)?.email || "";
    if (composeReplyTo && resolvedEmail) {
      setTo((prev) => prev.filter((addr) => addr !== resolvedEmail));
      setCc((prev) => prev.filter((addr) => addr !== resolvedEmail));
    }
  }, [accounts, activeAccountId, composeReplyTo]); // eslint-disable-line react-hooks/exhaustive-deps

  return {
    fromAccountId, setFromAccountId,
    to, setTo,
    cc, setCc,
    bcc, setBcc,
    showCc, setShowCc,
    showBcc, setShowBcc,
    myEmail,
  };
}
