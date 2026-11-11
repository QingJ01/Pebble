import { useState } from "react";
import { useTranslation } from "react-i18next";
import { getDraft } from "@/lib/api";
import { useComposeStore } from "@/stores/compose.store";
import { useToastStore } from "@/stores/toast.store";

export default function ResumeDraftButton({ messageId }: { messageId: string }) {
  const { t } = useTranslation();
  const [resuming, setResuming] = useState(false);
  async function resume() {
    if (resuming) return;
    setResuming(true);
    try {
      const { message, attachments, editor_mode } = await getDraft(messageId);
      useComposeStore.getState().openDraft({
        id: message.id, accountId: message.account_id,
        to: message.to_list.map((address) => address.address),
        cc: message.cc_list.map((address) => address.address),
        bcc: message.bcc_list.map((address) => address.address),
        subject: message.subject,
        rawSource: editor_mode === "html" ? message.body_html_raw : message.body_text,
        richTextHtml: message.body_html_raw, editorMode: editor_mode,
        inReplyTo: message.in_reply_to ?? undefined,
        attachments: attachments.filter((attachment) => !attachment.is_inline).map((attachment) => ({
          name: attachment.filename, path: attachment.local_path!, size: attachment.size,
        })), savedAt: Date.now(),
      });
    } catch (error) {
      useToastStore.getState().addToast({ type: "error", message: String(error) });
    } finally {
      setResuming(false);
    }
  }
  return <button disabled={resuming} onClick={() => { void resume(); }}>{t("compose.resumeDraft", "Resume draft")}</button>;
}
