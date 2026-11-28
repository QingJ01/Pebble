import { useTranslation } from "react-i18next";
import AgentSide from "./AgentSide";
import AgentThread from "./AgentThread";
import "./agent.css";

/**
 * The Pebble assistant (#107), reached from the sidebar. The middle is a
 * conversation: Pebble posts briefings, nudges and follow-up news on its
 * own, and (with a model set up) answers questions. The right column shows
 * what it is keeping track of.
 */
export default function AgentView() {
  const { t } = useTranslation();
  return (
    <div className="agent-view">
      <h2 className="sr-only">{t("agent.title", "Assistant")}</h2>
      <AgentThread />
      <AgentSide />
    </div>
  );
}
