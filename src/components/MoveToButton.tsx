import { useState } from "react";
import { FolderInput } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listFolders } from "@/lib/api";
import { getMailMoveContext, type MailMoveResult } from "@/lib/folders-api";
import { clearMovedSelection, crossesLocalBoundary, folderTree, gmailMutableFolder, invalidateFolderViews, isMoveDestination } from "@/lib/folderManagement";
import { folderIdsForSelection } from "@/lib/folderAggregation";
import { formatError } from "@/lib/extractErrorMessage";
import { moveMessagesWithUndo } from "@/hooks/mutations/mailActions";
import Button from "./ui/Button";
import IconButton from "./ui/IconButton";
import { useMailStore } from "@/stores/mail.store";
import FolderModal from "./FolderModal";
import { usePresence } from "@/hooks/usePresence";

export interface MoveMessage {id: string; account_id: string; is_draft: boolean; thread_id?: string | null}
/**
 * Called with the IDs that were actually moved, the folder they left (null when
 * each message had a single folder and the backend picked it) and the target.
 */
export type OnMoved = (movedIds: string[], sourceFolderId: string | null, targetFolderId: string) => void;
interface Props {messages: MoveMessage[]; disabled?: boolean; onMoved?: OnMoved; showLabel?: boolean}

/** Why these messages cannot be moved, or null. */
export function moveDisabledReason(messages: MoveMessage[], t: (key: string, fallback: string) => string): string | null {
  if (new Set(messages.map((m) => m.account_id)).size > 1) return t("folderManagement.sameAccount", "Select messages from one account");
  if (messages.some((m) => m.is_draft)) return t("folderManagement.draftsProtected", "Drafts cannot be moved; finish editing first");
  return null;
}

export default function MoveToButton(props: Props) {
  const {t} = useTranslation();
  const [open,setOpen] = useState(false);
  const label = t("folderManagement.moveTo", "Move to folder");
  const blocked = moveDisabledReason(props.messages, t);
  const disabled = props.disabled || !!blocked || !props.messages.length;
  const onClick = (e: React.MouseEvent) => {e.stopPropagation();setOpen(true);};
  return <>
    {props.showLabel
      ? <Button variant="ghost" size="sm" icon={<FolderInput size={14}/>} disabled={disabled} title={blocked ?? label} onClick={onClick}>{label}</Button>
      : <IconButton label={label} tooltip={blocked ?? label} icon={<FolderInput size={14}/>} disabled={disabled} onClick={onClick}/>}
    <MoveToDialog messages={props.messages} open={open} onClose={() => setOpen(false)} onMoved={props.onMoved}/>
  </>;
}

/**
 * The move-to-folder picker on its own, for menus: keep it rendered and toggle
 * `open` (it plays its exit animation). The selection is snapshotted on open.
 */
export function MoveToDialog({messages, open, onClose, onMoved}: {messages: MoveMessage[]; open: boolean; onClose: () => void; onMoved?: OnMoved}) {
  const presence = usePresence(open);
  if (!presence.mounted || messages.length === 0) return null;
  return <MovePicker messages={messages} open={open} onClose={onClose} onMoved={onMoved}/>;
}

function MovePicker({messages,onMoved,onClose,open = true}: {messages: MoveMessage[]; onMoved?: OnMoved; onClose: () => void; open?: boolean}) {
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
  const unsupportedSource = context?.provider === "gmail" && sourceFolder && !gmailMutableFolder(sourceFolder);
  const loading = foldersLoading || contextLoading;
  const hasError = folderError || contextError;
  const destinations = folderTree(folders.filter((f) => isMoveDestination(f, context?.provider)));
  const targetDisabled = (folder: typeof folders[number]) => ids.some((id) => memberships[id]?.includes(folder.id)) || crossesLocalBoundary(sourceFolder, folder, context?.provider);
  const targetFolder = folders.find((f) => f.id === target);
  return <FolderModal open={open} title={t("folderManagement.moveTo", "Move to folder")} busy={busy} onClose={onClose}>
    <form onSubmit={async (e) => {
      e.preventDefault();if (busy || unsupportedSource || !targetFolder || targetDisabled(targetFolder) || (needsSource && !sourceId)) return;
      setBusy(true);setError("");
      try {
        const sourceByMessage = Object.fromEntries(ids.map((id) => [id, memberships[id]?.length === 1 ? memberships[id][0] : undefined]));
        const response = await moveMessagesWithUndo(client,{accountId,messageIds:ids,sourceFolderId:sourceId || null,targetFolderId:target,targetFolderName:targetFolder.name,sourceByMessage});
        setResult(response);clearMovedSelection(response.moved_ids,selection);invalidateFolderViews(client);
        // Close selection/detail only for messages confirmed moved. Failed IDs
        // remain selected. Keep partial failures visible until dismissed.
        onMoved?.(response.moved_ids,sourceId || null,target);
        if (!response.failures.length) onClose();
      } catch (e) { setError(formatError(e, { fallbackKey: "folderManagement.moveFailed", fallback: "Move failed" }));invalidateFolderViews(client); }
      finally {setBusy(false);}
    }}>
      {context?.provider === "pop3" && <p>{t("folderManagement.pop3Local", "POP3 folders and moves are stored on this device only.")}</p>}
      {needsSource && <label>{t("folderManagement.source", "Source folder")}<select className="select" disabled={busy || !!result} value={sourceId} onChange={(e) => {setSource(e.target.value);setTarget("");}}>
        <option value="">{t("folderManagement.chooseSource", "Choose the label to remove")}</option>
        {folderTree(commonSources).map(({folder,path}) => <option key={folder.id} value={folder.id}>{path}</option>)}
      </select></label>}
      <label>{t("folderManagement.destination", "Destination")}<select className="select" disabled={busy || loading || !!hasError || !!result} value={target} onChange={(e) => setTarget(e.target.value)}>
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
      <div className="folder-modal-actions"><Button variant={result ? "primary" : "secondary"} disabled={busy} onClick={onClose}>{result ? t("folderManagement.close", "Close") : t("folderManagement.cancel", "Cancel")}</Button>{!result && <Button type="submit" variant="primary" loading={busy} disabled={loading || !!hasError || !!unsupportedSource || !targetFolder || targetDisabled(targetFolder) || (needsSource && !sourceId)}>{busy ? t("folderManagement.working", "Working…") : t("folderManagement.move", "Move")}</Button>}</div>
    </form>
  </FolderModal>;
}
