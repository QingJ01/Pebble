import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import Button from "@/components/ui/Button";
import Dialog from "@/components/ui/Dialog";
import { toastError } from "@/stores/toast.store";
import GrantEditor from "./GrantEditor";
import { useMcpInvalidation, useMcpMutations, usePendingPairs, type McpLevel } from "./api";
import "./connections.css";

/** Asks the user about a new MCP client; shown wherever Pebble is. */
export default function PairDialog() {
  const { t } = useTranslation();
  useMcpInvalidation();
  const { data: pending = [] } = usePendingPairs();
  const { respond } = useMcpMutations();
  const request = pending[0];
  const [grants, setGrants] = useState<Record<string, McpLevel>>({});
  const titleId = useId();
  useEffect(() => setGrants({}), [request?.id]);
  if (!request) return null;
  const answer = (approve: boolean) =>
    respond.mutate(
      { id: request.id, approve, grants: approve ? grants : {} },
      { onError: (error) => toastError(error, "connections.pairFailed") },
    );
  return (
    <Dialog open onClose={() => answer(false)} labelledBy={titleId} dismissible={!respond.isPending} closeOnBackdrop={false}>
      <h2 id={titleId} className="dialog-title">
        {t("connections.pairTitle", "Let “{{client}}” use your mail?", { client: request.client })}
      </h2>
      <p className="connections-pair__lead">
        {t(
          "connections.pairLead",
          "An app on this computer asked to connect to Pebble over MCP. Approve only if you just set it up yourself: any app can use this name. Choose what it may see for each account; you can change this or disconnect it later in Settings → Connections.",
        )}
      </p>
      <GrantEditor grants={grants} onChange={setGrants} disabled={respond.isPending} />
      <p className="connections-pair__note">
        {t("connections.neverSend", "Connected apps can never send, delete or move mail.")}
      </p>
      <div className="dialog-actions">
        <Button onClick={() => answer(false)} disabled={respond.isPending}>{t("connections.deny", "Don't allow")}</Button>
        <Button
          variant="primary"
          onClick={() => answer(true)}
          loading={respond.isPending}
          disabled={Object.keys(grants).length === 0}
        >
          {t("connections.approve", "Allow")}
        </Button>
      </div>
    </Dialog>
  );
}
