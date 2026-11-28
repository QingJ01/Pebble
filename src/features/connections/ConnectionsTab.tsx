import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy } from "lucide-react";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";
import {
  SettingsEmpty,
  SettingsList,
  SettingsListRow,
  SettingsNotice,
  SettingsSection,
  StatusChip,
} from "@/features/settings/SettingsLayout";
import { formatDateTime } from "@/lib/formatDate";
import { toastError, useToastStore } from "@/stores/toast.store";
import GrantEditor from "./GrantEditor";
import {
  useMcpAudit,
  useMcpClients,
  useMcpInvalidation,
  useMcpMutations,
  useMcpStatus,
  type McpClient,
  type McpLevel,
} from "./api";
import { outcomeLabel, toolLabel } from "./levels";
import "./connections.css";

function CopyBlock({ label, text }: { label: string; text: string }) {
  const { t } = useTranslation();
  const addToast = useToastStore((s) => s.addToast);
  return (
    <div className="connections-copy">
      <div className="connections-copy__head">
        <span>{label}</span>
        <Button
          size="sm"
          variant="ghost"
          icon={<Copy size={13} />}
          onClick={() => {
            void navigator.clipboard?.writeText(text);
            addToast({ type: "success", message: t("connections.copied", "Copied") });
          }}
        >
          {t("connections.copy", "Copy")}
        </Button>
      </div>
      <pre className="connections-copy__text">{text}</pre>
    </div>
  );
}

function ClientRow({ client }: { client: McpClient }) {
  const { t, i18n } = useTranslation();
  const { setGrants, revoke } = useMcpMutations();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Record<string, McpLevel>>(client.grants);
  const revoked = client.revokedAt !== null;
  return (
    <div className={`connections-client${revoked ? " is-revoked" : ""}`}>
      <SettingsListRow
        title={client.name}
        meta={
          revoked
            ? t("connections.revokedOn", "Disconnected {{date}}", { date: formatDateTime(client.revokedAt!, undefined, i18n.language) })
            : client.lastUsedAt
              ? t("connections.lastUsed", "Last used {{date}}", { date: formatDateTime(client.lastUsedAt, undefined, i18n.language) })
              : t("connections.neverUsed", "Not used yet")
        }
        actions={
          revoked ? (
            <StatusChip>{t("connections.revoked", "Disconnected")}</StatusChip>
          ) : (
            <>
              <Button size="sm" variant="ghost" onClick={() => { setDraft(client.grants); setEditing(!editing); }}>
                {editing ? t("connections.close", "Close") : t("connections.edit", "Change access")}
              </Button>
              <Button
                size="sm"
                variant="danger-ghost"
                onClick={() => revoke.mutate(client.id, { onError: (e) => toastError(e, "connections.revokeFailed") })}
              >
                {t("connections.revoke", "Disconnect")}
              </Button>
            </>
          )
        }
      />
      {editing && !revoked && (
        <div className="connections-client__edit">
          <GrantEditor grants={draft} onChange={setDraft} />
          <div className="settings-actions">
            <Button
              variant="primary"
              size="sm"
              loading={setGrants.isPending}
              onClick={() =>
                setGrants.mutate(
                  { clientId: client.id, grants: draft },
                  { onSuccess: () => setEditing(false), onError: (e) => toastError(e, "connections.saveFailed") },
                )
              }
            >
              {t("connections.save", "Save")}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

export default function ConnectionsTab() {
  const { t, i18n } = useTranslation();
  useMcpInvalidation();
  const { data: status } = useMcpStatus();
  const { data: clients = [] } = useMcpClients();
  const { data: audit = [] } = useMcpAudit();
  const { setEnabled } = useMcpMutations();
  const names = new Map(clients.map((c) => [c.id, c.name]));
  const command = status ? [status.command, ...status.args].map((part) => (/\s/.test(part) ? `"${part}"` : part)).join(" ") : "";
  return (
    <div>
      <SettingsSection
        title={t("connections.mcpTitle", "MCP clients")}
        description={t(
          "connections.mcpDesc",
          "Let AI apps on this computer (Claude Desktop, Cursor and others that support MCP) search and read your mail through Pebble, with your approval for each app.",
        )}
      >
        <Checkbox
          checked={!!status?.enabled}
          disabled={!status || setEnabled.isPending}
          onChange={() =>
            setEnabled.mutate(!status?.enabled, { onError: (e) => toastError(e, "connections.toggleFailed") })
          }
          label={t("connections.enable", "Let MCP clients use Pebble")}
        />
        {status?.enabled && (
          <>
            {!status.running && (
              <SettingsNotice tone="warning">{t("connections.notRunning", "Pebble couldn't start listening. Turn it off and on again.")}</SettingsNotice>
            )}
            <p className="connections-help">
              {t(
                "connections.setupHelp",
                "Add Pebble as an MCP server in the app, using this command (or paste the JSON into Claude Desktop's configuration). Pebble needs to be running.",
              )}
            </p>
            <CopyBlock label={t("connections.command", "Command")} text={command} />
            <CopyBlock label={t("connections.config", "Claude Desktop configuration")} text={status.configJson} />
            <SettingsNotice tone="info">
              {t(
                "connections.trust",
                "Every new app must be approved here, and can only send, delete or move nothing. Any program running as you could ask to connect, so approve only apps you just set up.",
              )}
            </SettingsNotice>
          </>
        )}
      </SettingsSection>

      <SettingsSection title={t("connections.clientsTitle", "Connected apps")}>
        {clients.length === 0 ? (
          <SettingsEmpty>{t("connections.clientsEmpty", "No app has connected yet.")}</SettingsEmpty>
        ) : (
          <SettingsList>
            {clients.map((client) => <ClientRow key={client.id} client={client} />)}
          </SettingsList>
        )}
      </SettingsSection>

      <SettingsSection
        title={t("connections.auditTitle", "Recent activity")}
        description={t("connections.auditDesc", "What connected apps and Pebble's own assistant asked for. Only the request type and outcome are kept, never mail content.")}
      >
        {audit.length === 0 ? (
          <SettingsEmpty>{t("connections.auditEmpty", "Nothing yet.")}</SettingsEmpty>
        ) : (
          <SettingsList className="connections-audit">
            {audit.map((row) => (
              <SettingsListRow
                key={row.id}
                title={`${row.clientId === "assistant" ? t("connections.assistant", "Pebble assistant") : (names.get(row.clientId) ?? "?")} · ${toolLabel(t, row.tool)}`}
                meta={`${formatDateTime(row.createdAt, undefined, i18n.language)} · ${
                  row.outcome === "ok"
                    ? t("connections.results", { count: row.results, defaultValue: "{{count}} results" })
                    : outcomeLabel(t, row.outcome)
                }`}
              />
            ))}
          </SettingsList>
        )}
      </SettingsSection>
    </div>
  );
}
