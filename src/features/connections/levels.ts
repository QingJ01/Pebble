import type { TFunction } from "i18next";
import type { McpLevel } from "./api";

export function levelLabel(t: TFunction, level: McpLevel | "none"): string {
  switch (level) {
    case "none":
      return t("connections.levelNone", "Not allowed");
    case "metadata":
      return t("connections.levelMetadata", "Search (sender, subject, date)");
    case "summary":
      return t("connections.levelSummary", "Search with snippets");
    case "body":
      return t("connections.levelBody", "Full text");
    case "drafts":
      return t("connections.levelDrafts", "Full text and draft proposals");
  }
}

export function toolLabel(t: TFunction, tool: string): string {
  switch (tool) {
    case "mail_search":
      return t("connections.toolSearch", "Search");
    case "mail_get_thread":
      return t("connections.toolThread", "Read thread");
    case "mail_propose_draft":
    case "draft_email":
      return t("connections.toolDraft", "Propose draft");
    case "mail_list_accounts":
      return t("connections.toolAccounts", "List accounts");
    case "mail_list_recent":
    case "list_recent_mail":
      return t("connections.toolRecent", "Recent mail");
    case "mail_get_message":
      return t("connections.toolMessage", "Read message");
    case "mail_list_open_items":
    case "list_open_items":
      return t("connections.toolOpenItems", "Open items");
    case "mail_follow_up":
    case "follow_up":
      return t("connections.toolFollowUp", "Follow up");
    case "mail_proposal_status":
      return t("connections.toolProposalStatus", "Draft status");
    case "search_mail":
      return t("connections.toolSearch", "Search");
    case "read_thread":
      return t("connections.toolThread", "Read thread");
    default:
      return tool;
  }
}

export function outcomeLabel(t: TFunction, outcome: string): string {
  switch (outcome) {
    case "ok":
      return t("connections.outcomeOk", "OK");
    case "forbidden":
      return t("connections.outcomeForbidden", "Not allowed");
    case "rate_limited":
      return t("connections.outcomeRateLimited", "Too many calls");
    case "not_found":
      return t("connections.outcomeNotFound", "Not found");
    case "invalid":
      return t("connections.outcomeInvalid", "Bad request");
    default:
      return t("connections.outcomeError", "Error");
  }
}
