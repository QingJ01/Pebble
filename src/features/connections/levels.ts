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
      return t("connections.toolDraft", "Propose draft");
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
