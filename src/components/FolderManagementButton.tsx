import { useState } from "react";
import { FolderCog } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listFolders, type Account, type Folder } from "@/lib/api";
import { createMailFolder, renameMailFolder, renameMailFolderNamespace, getMailFolderParents } from "@/lib/folders-api";
import { folderTree, invalidateFolderViews, renameDisabledReason } from "@/lib/folderManagement";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { accountOptionLabel } from "@/lib/accountIdentity";
import FolderModal from "./FolderModal";

interface Props { accounts: Account[]; folders: Folder[]; accountId: string | null; collapsed?: boolean }
export default function FolderManagementButton(props: Props) {
  const [open,setOpen] = useState(false);
  const { t } = useTranslation();
  if (!props.accounts.length) return null;
  return <>
    <button className="folder-action-button" title={t("folderManagement.manage", "Manage folders")} aria-label={t("folderManagement.manage", "Manage folders")} onClick={() => setOpen(true)}>
      <FolderCog size={16}/>{!props.collapsed && t("folderManagement.manage", "Manage folders")}
    </button>
    {open && <FolderManager {...props} onClose={() => setOpen(false)}/>}
  </>;
}

function FolderManager({accounts,folders,accountId,onClose}: Props & {onClose: () => void}) {
  const {t} = useTranslation();
  const client = useQueryClient();
  const [account,setAccount] = useState(accountId ?? accounts[0].id);
  const [mode,setMode] = useState("create");
  const [folderId,setFolderId] = useState("");
  const [parentId,setParentId] = useState("");
  const [name,setName] = useState("");
  const [busy,setBusy] = useState(false);
  const [error,setError] = useState("");
  const {data:loadedFolders,error:foldersError}=useQuery({queryKey:["folders",account],queryFn:()=>listFolders(account),initialData:()=>folders.some((f)=>f.account_id===account)?folders.filter((f)=>f.account_id===account):undefined});
  const accountFolders = (loadedFolders ?? folders).filter((f) => f.account_id === account);
  const provider = accounts.find((a) => a.id === account)?.provider ?? "";
  const selected = accountFolders.find((f) => f.id === folderId);
  const {data:serverParents=[],error:parentError}=useQuery({queryKey:["managed-folder-parents",account],queryFn:()=>getMailFolderParents(account),enabled:provider==="imap"});
  const namespace=serverParents.find((p)=>!p.selectable&&folderId===`namespace:${p.remote_id}`);
  const disabledReason=mode==="rename" ? namespace ? namespace.is_system ? "folderManagement.systemProtected" : null : renameDisabledReason(selected,accountFolders,provider) : null;
  const parentFolders = accountFolders.filter((f) => f.folder_type!=="category" && (provider !== "gmail" || !f.is_system) && (provider === "pop3" || !f.remote_id.startsWith("__local_")));
  const entries = folderTree(accountFolders);
  return <FolderModal title={t("folderManagement.manage", "Manage folders")} busy={busy} onClose={onClose}>
    <form onSubmit={async (e) => {
      e.preventDefault(); if (busy || disabledReason || !name.trim()) return;
      setBusy(true);setError("");
      try {
        if (mode === "create") {
          if(provider==="imap") await createMailFolder(account,name,null,parentId||null);
          else await createMailFolder(account,name,parentId || null);
        }
        else if(namespace) await renameMailFolderNamespace(account,namespace.remote_id,name);
        else await renameMailFolder(account,folderId,name);
        invalidateFolderViews(client); onClose();
      } catch (e) { setError(`${t("folderManagement.failed", "Folder operation failed")}: ${extractErrorMessage(e)}`); }
      finally {setBusy(false);}
    }}>
      <label>{t("folderManagement.account", "Account")}<select disabled={busy} value={account} onChange={(e) => {setAccount(e.target.value);setFolderId("");setParentId("");setName("");setError("");}}>
        {accounts.map((a) => <option key={a.id} value={a.id}>{accountOptionLabel(a)}</option>)}
      </select></label>
      {provider === "pop3" && <p>{t("folderManagement.pop3Local", "POP3 folders and moves are stored on this device only.")}</p>}
      <label>{t("folderManagement.action", "Action")}<select disabled={busy} value={mode} onChange={(e) => {setMode(e.target.value);setName("");setError("");}}>
        <option value="create">{t("folderManagement.create", "New folder")}</option>
        <option value="rename">{t("folderManagement.rename", "Rename folder")}</option>
      </select></label>
      {mode === "create" ? <label>{t("folderManagement.parent", "Parent folder")}<select disabled={busy} value={parentId} onChange={(e) => setParentId(e.target.value)}>
        <option value="">{t("folderManagement.root", "Account root")}</option>
        {provider==="imap" ? serverParents.filter((p)=>p.delimiter).map((p)=><option key={p.remote_id} value={p.remote_id}>{p.name}</option>) : folderTree(parentFolders).map(({folder,path,depth}) => <option key={folder.id} value={folder.id}>{"　".repeat(depth)}{path}</option>)}
      </select></label> : <label>{t("folderManagement.folder", "Folder")}<select disabled={busy} value={folderId} onChange={(e) => {setFolderId(e.target.value);const f=accountFolders.find((f) => f.id===e.target.value);const p=serverParents.find((p)=>p.remote_id===f?.remote_id||`namespace:${p.remote_id}`===e.target.value);setName(p ? p.name.split(p.delimiter||"/").pop()??"" : provider==="gmail" ? f?.name.split('/').pop()??"" : f?.name??"");setError("");}}>
        <option value="">{t("folderManagement.chooseFolder", "Choose a folder")}</option>
        {entries.map(({folder,path}) => <option key={folder.id} value={folder.id}>{path}</option>)}
        {serverParents.filter((p)=>!p.selectable).map((p)=><option key={p.remote_id} value={`namespace:${p.remote_id}`}>{p.name}</option>)}
      </select></label>}
      <label>{t("folderManagement.name", "Name")}<input disabled={busy || !!disabledReason} value={name} onChange={(e) => setName(e.target.value)} maxLength={255} autoComplete="off"/></label>
      {disabledReason && <p>{t(disabledReason)}</p>}
      {(parentError || foldersError) && <p role="alert">{t("folderManagement.loadFailed", "Could not load folders. Check your account and try again.")}</p>}
      {error && <p role="alert">{error}</p>}
      <div className="folder-modal-actions"><button type="button" disabled={busy} onClick={onClose}>{t("folderManagement.cancel", "Cancel")}</button><button type="submit" disabled={busy || !!disabledReason || !name.trim()}>{busy ? t("folderManagement.working", "Working…") : t("folderManagement.save", "Save")}</button></div>
    </form>
  </FolderModal>;
}
