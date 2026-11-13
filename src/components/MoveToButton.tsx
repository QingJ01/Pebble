import { useState } from "react";
import { FolderInput } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listFolders } from "@/lib/api";
import { getMailMoveContext, moveMailMessages, type MailMoveResult } from "@/lib/folders-api";
import { clearMovedSelection, folderTree, invalidateFolderViews } from "@/lib/folderManagement";
import { folderIdsForSelection } from "@/lib/folderAggregation";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { useMailStore } from "@/stores/mail.store";
import FolderModal from "./FolderModal";

interface MoveMessage {id: string; account_id: string; is_draft: boolean; thread_id?: string | null}
interface Props {messages: MoveMessage[]; disabled?: boolean; onMoved?: (ids: string[]) => void; showLabel?: boolean}
export default function MoveToButton(props: Props) {
  const {t} = useTranslation();
  const [open,setOpen] = useState(false);
  const mixed = new Set(props.messages.map((m) => m.account_id)).size !== 1;
  const disabled = props.disabled || mixed || !props.messages.length || props.messages.some((m) => m.is_draft);
  const label = t("folderManagement.moveTo", "Move to folder");
  const reason = mixed ? t("folderManagement.sameAccount", "Select messages from one account") : props.messages.some((m) => m.is_draft) ? t("folderManagement.draftsProtected", "Drafts cannot be moved; finish editing first") : label;
  return <>
    <button className="folder-action-button" disabled={disabled} title={reason} aria-label={label} onClick={(e) => {e.stopPropagation();setOpen(true);}}><FolderInput size={16}/>{props.showLabel && label}</button>
    {open && <MovePicker {...props} onClose={() => setOpen(false)}/>}
  </>;
}

function MovePicker({messages,onMoved,onClose}: Props & {onClose: () => void}) {
  const {t} = useTranslation();
  const client = useQueryClient();
  // Snapshot selection on open; background refetches cannot retarget this dialog.
  const [selection] = useState(messages);
  const accountId = selection[0].account_id;
  const ids = selection.map((m) => m.id);
  const [target,setTarget] = useState("");
  const [source,setSource] = useState<string | null>(null);
  const [busy,setBusy] = useState(false);
  const [error,setError] = useState("");
  const [result,setResult] = useState<MailMoveResult | null>(null);
  const {data:folders = [],isLoading:foldersLoading,error:folderError} = useQuery({queryKey:["folders",accountId],queryFn:() => listFolders(accountId)});
  const {data:context,isLoading:contextLoading,error:contextError} = useQuery({queryKey:["mail-move-context",accountId,...ids],queryFn:() => getMailMoveContext(accountId,ids)});
  const memberships = context?.folder_ids_by_message ?? {};
  const commonSources = folders.filter((f) => ids.every((id) => memberships[id]?.includes(f.id)));
  const activeFolder = useMailStore.getState().activeFolderId;
  const current = folderIdsForSelection(activeFolder,folders).find((id) => commonSources.some((f) => f.id === id));
  const sourceId = source ?? current ?? (commonSources.length === 1 ? commonSources[0].id : "");
  const needsSource = ids.some((id) => (memberships[id]?.length ?? 0) > 1);
  const sourceFolder = folders.find((f) => f.id === sourceId);
  const gmailMutable = (folder: typeof folders[number]) => !folder.is_system || folder.remote_id.startsWith("__local_") || ["inbox","spam","trash"].includes(folder.role ?? "");
  const unsupportedSource = context?.provider === "gmail" && sourceFolder && !gmailMutable(sourceFolder);
  const loading = foldersLoading || contextLoading;
  const hasError = folderError || contextError;
  const destinations = folderTree(folders.filter((f) => f.role !== "drafts" && f.remote_id !== "__local_outbox__" && f.folder_type !== "category" && (context?.provider !== "gmail" || gmailMutable(f))));
  const targetDisabled = (folder: typeof folders[number]) => ids.some((id) => memberships[id]?.includes(folder.id)) || (context?.provider !== "pop3" && !!sourceFolder && sourceFolder.remote_id.startsWith("__local_") !== folder.remote_id.startsWith("__local_"));
  const targetFolder = folders.find((f) => f.id === target);
  return <FolderModal title={t("folderManagement.moveTo", "Move to folder")} busy={busy} onClose={onClose}>
    <form onSubmit={async (e) => {
      e.preventDefault();if (busy || unsupportedSource || !targetFolder || targetDisabled(targetFolder) || (needsSource && !sourceId)) return;
      setBusy(true);setError("");
      try {
        const response = await moveMailMessages(accountId,ids,sourceId || null,target);
        setResult(response);clearMovedSelection(response.moved_ids,selection);invalidateFolderViews(client);
        // Close selection/detail only for messages confirmed moved. Failed IDs
        // remain selected. Keep partial failures visible until dismissed.
        onMoved?.(response.moved_ids);
        if (!response.failures.length) onClose();
      } catch (e) { setError(`${t("folderManagement.moveFailed", "Move failed")}: ${extractErrorMessage(e)}`);invalidateFolderViews(client); }
      finally {setBusy(false);}
    }}>
      {context?.provider === "pop3" && <p>{t("folderManagement.pop3Local", "POP3 folders and moves are stored on this device only.")}</p>}
      {needsSource && <label>{t("folderManagement.source", "Source folder")}<select disabled={busy || !!result} value={sourceId} onChange={(e) => {setSource(e.target.value);setTarget("");}}>
        <option value="">{t("folderManagement.chooseSource", "Choose the label to remove")}</option>
        {folderTree(commonSources).map(({folder,path}) => <option key={folder.id} value={folder.id}>{path}</option>)}
      </select></label>}
      <label>{t("folderManagement.destination", "Destination")}<select disabled={busy || loading || !!hasError || !!result} value={target} onChange={(e) => setTarget(e.target.value)}>
        <option value="">{t("folderManagement.chooseFolder", "Choose a folder")}</option>
        {destinations.map(({folder,path,depth}) => <option key={folder.id} value={folder.id} disabled={targetDisabled(folder)}>{"　".repeat(depth)}{path}{folder.remote_id.startsWith("__local_") ? ` (${t("folderManagement.localOnly", "On this device")})` : ""}</option>)}
      </select></label>
      <p>{t("folderManagement.moveHint", "Only folders in this account are shown. Current folders and transfers between server and device-only folders are unavailable.")}</p>
      {context?.provider === "gmail" && <p>{t("folderManagement.gmailHint", "Move removes only the selected source label. Other labels are kept.")}</p>}
      {unsupportedSource && <p role="alert">{t("folderManagement.gmailDestination", "Gmail permits moving only between user labels, Inbox, Spam and Trash.")}</p>}
      {loading && <p>{t("folderManagement.loading", "Loading folders…")}</p>}
      {hasError && <p role="alert">{t("folderManagement.loadFailed", "Could not load folders. Check your account and try again.")}</p>}
      {error && <p role="alert">{error}</p>}
      {result?.failures.length ? <p role="alert">{t("folderManagement.partial", {defaultValue:"{{moved}} moved; {{failed}} failed. Failed messages remain selected.",moved:result.moved_ids.length,failed:result.failures.length})}{"\n"}{result.failures.map((f) => f.error).filter((e,i,a) => a.indexOf(e) === i).join("\n")}</p> : null}
      <div className="folder-modal-actions"><button type="button" disabled={busy} onClick={onClose}>{result ? t("folderManagement.close", "Close") : t("folderManagement.cancel", "Cancel")}</button><button type="submit" disabled={busy || loading || !!hasError || !!result || !!unsupportedSource || !targetFolder || targetDisabled(targetFolder) || (needsSource && !sourceId)}>{busy ? t("folderManagement.working", "Working…") : t("folderManagement.move", "Move")}</button></div>
    </form>
  </FolderModal>;
}
