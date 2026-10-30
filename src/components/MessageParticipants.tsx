import { useState } from "react";
import { useTranslation } from "react-i18next";
import ContactAddressAction from "./ContactAddressAction";
import { uniqueParticipants, type ContactParticipant } from "./contact-participants";

export interface ParticipantGroup {
  label: string;
  participants: readonly ContactParticipant[];
}

interface Props {
  accountId: string;
  groups: readonly ParticipantGroup[];
  collapsedLimit?: number;
}

const DEFAULT_COLLAPSED_LIMIT = 3;

function participantLabel(participant: ContactParticipant): string {
  return participant.name?.trim() || participant.address;
}

function participantTitle(participant: ContactParticipant): string {
  const name = participant.name?.trim();
  return name && name !== participant.address ? `${name} <${participant.address}>` : participant.address;
}

export default function MessageParticipants({ accountId, groups, collapsedLimit = DEFAULT_COLLAPSED_LIMIT }: Props) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  const nonEmptyGroups = groups
    .map((group) => ({ label: group.label, participants: uniqueParticipants(group.participants) }))
    .filter((group) => group.participants.length > 0);
  const totalCount = nonEmptyGroups.reduce((count, group) => count + group.participants.length, 0);
  const collapsible = totalCount > collapsedLimit;
  const showAll = expanded || !collapsible;

  let budget = collapsedLimit;
  const visibleCounts = nonEmptyGroups.map((group) => {
    const visibleCount = showAll
      ? group.participants.length
      : Math.min(group.participants.length, Math.max(0, budget));
    budget -= visibleCount;
    return visibleCount;
  });
  const hiddenCount = totalCount - visibleCounts.reduce((sum, count) => sum + count, 0);
  const lastVisibleGroupIndex = visibleCounts.reduce((last, count, index) => (count > 0 ? index : last), -1);

  const scrollable = expanded && collapsible;

  return (
    <div className="message-participants" aria-label={t("contacts.participantActions", "Contact actions")}>
      <div
        className={`message-participants-groups${scrollable ? " message-participants-groups--scroll" : ""}`}
      >
        {nonEmptyGroups.map((group, index) => {
          const visible = group.participants.slice(0, visibleCounts[index]);
          const hostsToggle = index === lastVisibleGroupIndex && collapsible && !expanded;
          return (
            <div key={group.label} className="participant-row">
              <span className="participant-label">{group.label}</span>
              {visible.map((participant) => (
                <span
                  key={participant.address.toLowerCase()}
                  className="participant-chip"
                  title={participantTitle(participant)}
                >
                  <span className="participant-name">{participantLabel(participant)}</span>
                  <ContactAddressAction
                    accountId={accountId}
                    name={participant.name}
                    address={participant.address}
                  />
                </span>
              ))}
              {hostsToggle && (
                <button
                  type="button"
                  className="participant-toggle"
                  aria-expanded={expanded}
                  onClick={() => setExpanded(true)}
                >
                  {t("participants.more", { count: hiddenCount, defaultValue: "+{{count}} more" })}
                </button>
              )}
            </div>
          );
        })}
      </div>
      {scrollable && (
        <div className="participant-row">
          <button
            type="button"
            className="participant-toggle"
            aria-expanded={expanded}
            onClick={() => setExpanded(false)}
          >
            {t("participants.showLess", "Show less")}
          </button>
        </div>
      )}
    </div>
  );
}
