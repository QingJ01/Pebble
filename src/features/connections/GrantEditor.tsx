import { useTranslation } from "react-i18next";
import Select from "@/components/ui/Select";
import { SettingsList, SettingsListRow, SettingsNotice } from "@/features/settings/SettingsLayout";
import { useAccountsQuery } from "@/hooks/queries";
import { MCP_LEVELS, type McpLevel } from "./api";
import { levelLabel } from "./levels";

/** Per-account access for an MCP client. Gmail can't be granted yet. */
export default function GrantEditor({
  grants,
  onChange,
  disabled,
}: {
  grants: Record<string, McpLevel>;
  onChange: (grants: Record<string, McpLevel>) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const { data: accounts = [] } = useAccountsQuery();
  const readsText = Object.values(grants).some((level) => level === "body" || level === "drafts");
  return (
    <>
      <SettingsList className="connections-grants">
        {accounts.map((account) => {
          const gmail = account.provider === "gmail";
          const level = grants[account.id] ?? "none";
          return (
            <SettingsListRow
              key={account.id}
              title={account.account_label || account.email}
              meta={gmail ? t("connections.gmailUnavailable", "Gmail accounts can't be shared yet") : account.account_label ? account.email : undefined}
              actions={
                <Select
                  size="sm"
                  aria-label={t("connections.levelFor", "Access to {{account}}", { account: account.email })}
                  value={gmail ? "none" : level}
                  disabled={disabled || gmail}
                  onChange={(event) => {
                    const next = { ...grants };
                    const value = event.target.value as McpLevel | "none";
                    if (value === "none") delete next[account.id];
                    else next[account.id] = value;
                    onChange(next);
                  }}
                >
                  <option value="none">{levelLabel(t, "none")}</option>
                  {MCP_LEVELS.map((option) => (
                    <option key={option} value={option}>{levelLabel(t, option)}</option>
                  ))}
                </Select>
              }
            />
          );
        })}
      </SettingsList>
      {readsText && (
        <SettingsNotice tone="warning" className="connections-warning">
          {t(
            "connections.bodyWarning",
            "With full text, mail content goes to this app and its AI model. Pebble can't control what happens to it there.",
          )}
        </SettingsNotice>
      )}
    </>
  );
}
