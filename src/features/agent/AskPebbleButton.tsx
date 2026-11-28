import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import IconButton from "@/components/ui/IconButton";
import { useUIStore } from "@/stores/ui.store";
import { useAskContext } from "./askContext";

/** Reader toolbar: ask the assistant about the message being read. */
export default function AskPebbleButton({
  accountId,
  messageId,
  subject,
  from,
}: {
  accountId: string;
  messageId: string;
  subject: string;
  from: string;
}) {
  const { t } = useTranslation();
  const setActiveView = useUIStore((s) => s.setActiveView);
  const setContext = useAskContext((s) => s.setContext);
  return (
    <IconButton
      size="md"
      label={t("agent.askAboutThis", "Ask Pebble about this")}
      icon={<Sparkles size={16} />}
      onClick={() => {
        setContext({ accountId, messageId, subject, from });
        setActiveView("agent");
      }}
    />
  );
}
