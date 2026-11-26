import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import i18next from "i18next";
import { useAccountsQuery, useFoldersQuery } from "@/hooks/queries";
import { useMailStore } from "@/stores/mail.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { formatDateTime } from "@/lib/formatDate";
import * as api from "@/lib/mail-archive-api";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { useToastStore } from "@/stores/toast.store";
import Button from "@/components/ui/Button";
import { ensureArchiveProgressListener, useArchiveJobStore, type ArchiveReportKind } from "./archiveJob.store";
import "./mail-archive.css";

const ROOT = ["mail-archive"] as const;
function errorText(error: unknown): string {
  return extractErrorMessage(error);
}

/**
 * Run one archive action. State is kept in useArchiveJobStore (not component
 * state) so a running import/export survives leaving the page.
 */
async function performArchiveAction(action: () => Promise<void>) {
  const store = useArchiveJobStore.getState();
  if (store.busy) return;
  store.set({ busy: true, error: "" });
  try {
    await action();
  } catch (cause) {
    useArchiveJobStore.getState().set({ error: errorText(cause) });
    if (!useArchiveJobStore.getState().viewMounted) {
      useToastStore.getState().addToast({ type: "error", message: i18next.t("mailArchive.jobFailed", "Local mail library task failed"), detail: errorText(cause) });
    }
  } finally {
    useArchiveJobStore.getState().set({ busy: false, jobId: null });
  }
}

function reportFinished(kind: ArchiveReportKind, report: ArchiveReport) {
  const state = useArchiveJobStore.getState();
  state.set({ report, reportKind: kind });
  if (!state.viewMounted && kind !== "preview") {
    useToastStore.getState().addToast({
      type: report.failed > 0 ? "warning" : "success",
      message: i18next.t(kind === "export" ? "mailArchive.exportFinished" : "mailArchive.importFinished", {
        processed: report.processed, imported: report.imported, failed: report.failed,
      }),
    });
  }
}

type ArchiveReport = api.ArchiveReport;

export default function MailArchiveView() {
  const { t } = useTranslation();
  const client = useQueryClient();
  const { data: accounts = [] } = useAccountsQuery();
  const archives = useQuery({ queryKey: [...ROOT, "archives"], queryFn: api.listMailArchives });
  const [archiveChoice, setArchiveChoice] = useState("");
  const archiveId = archiveChoice || archives.data?.[0]?.id || "";
  const [accountChoice, setAccountChoice] = useState("");
  const accountId = accountChoice || accounts[0]?.id || "";
  const [archiveName, setArchiveName] = useState("");
  const [folderChoice, setFolderChoice] = useState("");
  const folders = useQuery({ queryKey: [...ROOT, archiveId, "folders"], queryFn: () => api.listArchiveFolders(archiveId), enabled: !!archiveId });
  const folderId = folderChoice || folders.data?.[0]?.id || "";
  const [folderName, setFolderName] = useState("");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState<string[]>([]);
  const [messageId, setMessageId] = useState("");
  const [selection, setSelection] = useState<api.ArchiveSelection | null>(null);
  const preview = useArchiveJobStore((s) => s.preview);
  const report = useArchiveJobStore((s) => s.report);
  const reportKind = useArchiveJobStore((s) => s.reportKind);
  const error = useArchiveJobStore((s) => s.error);
  const busy = useArchiveJobStore((s) => s.busy);
  const progress = useArchiveJobStore((s) => s.progress);
  const jobId = useArchiveJobStore((s) => s.jobId);
  const setJob = useArchiveJobStore((s) => s.set);
  const setPreview = (value: api.ArchiveReport | null) => setJob({ preview: value });
  const setReport = (value: api.ArchiveReport | null) => setJob({ report: value });
  const setError = (value: string) => setJob({ error: value });
  const [format, setFormat] = useState<api.ArchiveExportFormat>("eml");
  const messages = useQuery({ queryKey: [...ROOT, archiveId, folderId, search, offset], queryFn: () => api.listArchiveMessages(archiveId, folderId, search, offset), enabled: !!archiveId && !!folderId });
  const detail = useQuery({ queryKey: [...ROOT, archiveId, "message", messageId], queryFn: () => api.readArchiveMessage(archiveId, messageId), enabled: !!messageId && !!archiveId });

  const initialQueryEffect = useRef(true);
  useEffect(() => {
    if (initialQueryEffect.current) {
      initialQueryEffect.current = false;
      return;
    }
    const timer = setTimeout(() => { setSearch(query); setOffset(0); setSelected([]); setMessageId(""); }, 250);
    return () => clearTimeout(timer);
  }, [query]);
  // Jobs keep running when this page unmounts (switching tabs or views no
  // longer silently aborts an import); completion is reported by toast.
  useEffect(() => {
    ensureArchiveProgressListener();
    setJob({ viewMounted: true });
    return () => setJob({ viewMounted: false });
  }, [setJob]);

  const perform = performArchiveAction;
  function clearBrowser() { setSelected([]); setMessageId(""); setOffset(0); }
  async function refresh() { await client.invalidateQueries({ queryKey: ROOT }); }
  async function runImport(isPreview: boolean) {
    if (!selection || !archiveId || !folderId) return;
    await perform(async () => {
      if (!isPreview && !await useConfirmStore.getState().confirm({ title: t("mailArchive.importTitle"), message: t("mailArchive.confirmImport"), confirmLabel: t("mailArchive.importConfirmAction") })) return;
      const id = crypto.randomUUID(); setJob({ jobId: id, progress: null });
      const result = await api.importMailArchive(id, selection.id, archiveId, folderId, isPreview);
      reportFinished(isPreview ? "preview" : "import", result);
      if (isPreview) setPreview(result); else { setPreview(null); await refresh(); }
    });
  }
  async function runExport(source: api.ArchiveExportSource) {
    await perform(async () => {
      const id = crypto.randomUUID(); setJob({ jobId: id, progress: null });
      const result = await api.exportMailArchive(id, source, format);
      if (result) reportFinished("export", result);
    });
  }
  async function edit(folder: string, read: boolean, starred: boolean) {
    if (!detail.data) return;
    await perform(async () => { await api.updateArchiveMessage(archiveId, detail.data!.id, folder, read, starred); clearBrowser(); await refresh(); });
  }

  return <section className="mail-archive" aria-label={t("mailArchive.title")}>
    <header className="page-header page-header--flush">
      <div className="page-header__text">
        <h2 className="page-header__title">{t("mailArchive.title")}</h2>
        <p className="page-header__description">{t("mailArchive.localOnly")}</p>
        <p className="page-header__description">{t("mailArchive.searchScope")}</p>
      </div>
    </header>
    <fieldset disabled={busy}>
      <legend>{t("mailArchive.destination")}</legend>
      <div className="mail-archive-row">
        <label>{t("mailArchive.archive")}<select className="select" value={archiveId} onChange={(e) => { setArchiveChoice(e.target.value); setFolderChoice(""); setPreview(null); clearBrowser(); }}>
          <option value="" disabled>{t("mailArchive.chooseArchive")}</option>
          {archives.data?.map((archive) => <option key={archive.id} value={archive.id}>{archive.name} — {archive.account_label}</option>)}
        </select></label>
        <label>{t("mailArchive.folder")}<select className="select" value={folderId} onChange={(e) => { setFolderChoice(e.target.value); setPreview(null); clearBrowser(); }}>
          <option value="" disabled>{t("mailArchive.chooseFolder")}</option>
          {folders.data?.map((folder) => <option key={folder.id} value={folder.id}>{folder.name}</option>)}
        </select></label>
      </div>
      <form className="mail-archive-row" onSubmit={(e) => { e.preventDefault(); void perform(async () => { const archive = await api.createMailArchive(accountId, archiveName); setArchiveChoice(archive.id); setFolderChoice(""); setArchiveName(""); setPreview(null); clearBrowser(); await refresh(); }); }}>
        <label>{t("mailArchive.account")}<select className="select" value={accountId} onChange={(e) => setAccountChoice(e.target.value)}><option value="" disabled>{t("mailArchive.chooseAccount")}</option>{accounts.map((account) => <option key={account.id} value={account.id}>{account.email}</option>)}</select></label>
        <label>{t("mailArchive.newArchive")}<input className="input" value={archiveName} maxLength={200} onChange={(e) => setArchiveName(e.target.value)} /></label>
        <Button type="submit" disabled={!accountId || !archiveName.trim()}>{t("mailArchive.createArchive")}</Button>
      </form>
      {archiveId && <form className="mail-archive-row" onSubmit={(e) => { e.preventDefault(); void perform(async () => { const folder = await api.createArchiveFolder(archiveId, folderName); setFolderChoice(folder.id); setFolderName(""); setPreview(null); clearBrowser(); await refresh(); }); }}>
        <label>{t("mailArchive.newFolder")}<input className="input" value={folderName} maxLength={200} onChange={(e) => setFolderName(e.target.value)} /></label><Button type="submit" disabled={!folderName.trim()}>{t("mailArchive.createFolder")}</Button>
      </form>}
    </fieldset>
    <fieldset disabled={busy}>
      <legend>{t("mailArchive.importTitle")}</legend>
      <p className="mail-archive-note">{t("mailArchive.limits")}</p>
      <Button onClick={() => void perform(async () => { const result = await api.chooseArchiveFiles(); if (result) { setSelection(result); setPreview(null); setReport(null); } })}>{t("mailArchive.chooseFiles")}</Button>
      {selection && <ul>{selection.files.map((file, index) => <li key={index}>{file.name} ({(file.bytes / 1024).toFixed(1)} KiB)</li>)}</ul>}
      <div className="mail-archive-row"><Button disabled={!selection || !folderId} onClick={() => void runImport(true)}>{t("mailArchive.validate")}</Button>
        <Button variant="primary" disabled={!preview || !preview.imported || preview.cancelled} onClick={() => void runImport(false)}>{t("mailArchive.importAction")}</Button></div>
    </fieldset>
    {(error || archives.error || folders.error || messages.error || detail.error) && <p role="alert">{error || errorText(archives.error || folders.error || messages.error || detail.error)}</p>}
    {busy && <div role="status" className="mail-archive-progress">
      <p>{progress
        ? t("mailArchive.progressCounts", { processed: progress.processed, imported: progress.imported, duplicates: progress.duplicates, failed: progress.failed })
        : t("mailArchive.working")}</p>
      {jobId && <p className="mail-archive-note">{t("mailArchive.keepsRunning", "You can leave this page; the task keeps running and you'll be notified when it finishes.")}</p>}
      {jobId && <Button size="sm" onClick={() => void api.cancelArchiveJob(jobId).catch((cause) => setError(errorText(cause)))}>{t("mailArchive.cancel")}</Button>}
    </div>}
    {report && <section role="status" aria-label={t("mailArchive.summary")}>
      <h3>{t(`mailArchive.result.${reportKind}`)}</h3>
      <p>{t("mailArchive.resultCounts", { processed: report.processed, imported: report.imported, duplicates: report.duplicates, failed: report.failed })}</p>
      {report.cancelled && <p>{t("mailArchive.cancelled")}</p>}
      <ul>{report.errors.map((item, index) => <li key={index}>{item}</li>)}</ul>
    </section>}
    <fieldset disabled={busy || !archiveId}>
      <legend>{t("mailArchive.browse")}</legend>
      <label>{t("mailArchive.search")}<input className="input" type="search" maxLength={500} value={query} onChange={(e) => setQuery(e.target.value)} /></label>
      <div className="mail-archive-row">
        <label>{t("mailArchive.format")}<select className="select" value={format} onChange={(e) => setFormat(e.target.value as api.ArchiveExportFormat)}><option value="eml">EML</option><option value="mbox">MBOX (mboxrd)</option></select></label>
        <Button disabled={!selected.length} onClick={() => void runExport({ kind: "archive", archive_id: archiveId, message_ids: selected, folder_id: null })}>{t("mailArchive.exportSelected")}</Button>
        <Button disabled={!folderId} onClick={() => void runExport({ kind: "archive", archive_id: archiveId, message_ids: [], folder_id: folderId })}>{t("mailArchive.exportFolder")}</Button>
      </div>
      {messages.isLoading && <p role="status">{t("mailArchive.loading")}</p>}
      {!messages.isLoading && messages.data?.length === 0 && <p>{t("mailArchive.empty")}</p>}
      <ul className="mail-archive-messages">{messages.data?.map((message) => <li key={message.id}>
        <input className="checkbox" aria-label={t("mailArchive.selectMessage", { subject: message.subject })} type="checkbox" checked={selected.includes(message.id)} onChange={(e) => setSelected((ids) => e.target.checked ? [...ids, message.id] : ids.filter((id) => id !== message.id))} />
        <button type="button" aria-pressed={messageId === message.id} className={`mail-archive-message${message.is_read ? "" : " mail-archive-unread"}`} onClick={() => setMessageId(message.id)}>{message.is_starred && <span className="mail-archive-star" aria-hidden="true">★ </span>}{message.subject || t("mailArchive.noSubject")} — {message.sender}<time>{formatDateTime(message.date)}</time></button>
      </li>)}</ul>
      <div className="mail-archive-row"><Button disabled={!offset} onClick={() => { setOffset(Math.max(0, offset - 100)); setSelected([]); setMessageId(""); }}>{t("mailArchive.previous")}</Button><span>{offset + 1}–{offset + (messages.data?.length || 0)}</span><Button disabled={messages.data?.length !== 100} onClick={() => { setOffset(offset + 100); setSelected([]); setMessageId(""); }}>{t("mailArchive.next")}</Button></div>
      {detail.data && <article className="mail-archive-detail" aria-label={t("mailArchive.message")}>
        <h3>{detail.data.metadata.subject || t("mailArchive.noSubject")}</h3>
        <p>{detail.data.metadata.from_name} &lt;{detail.data.metadata.from_address}&gt;</p>
        <p>{t("mailArchive.toLine", { list: detail.data.metadata.to_list.map((a) => a.address).join(", ") })}</p>
        <p>{t("mailArchive.ccLine", { list: detail.data.metadata.cc_list.map((a) => a.address).join(", ") })}</p><p>{t("mailArchive.bccLine", { list: detail.data.metadata.bcc_list.map((a) => a.address).join(", ") })}</p>
        <p>{formatDateTime(detail.data.metadata.date)}</p>
        <div className="mail-archive-row"><Button onClick={() => void edit(detail.data!.folder_id, !detail.data!.is_read, detail.data!.is_starred)}>{t(detail.data.is_read ? "mailArchive.markUnread" : "mailArchive.markRead")}</Button><Button onClick={() => void edit(detail.data!.folder_id, detail.data!.is_read, !detail.data!.is_starred)}>{t(detail.data.is_starred ? "mailArchive.unstar" : "mailArchive.star")}</Button>
          <label>{t("mailArchive.move")}<select className="select" value={detail.data.folder_id} onChange={(e) => void edit(e.target.value, detail.data!.is_read, detail.data!.is_starred)}>{folders.data?.map((folder) => <option key={folder.id} value={folder.id}>{folder.name}</option>)}</select></label>
          <Button variant="danger-ghost" onClick={() => void perform(async () => { if (await useConfirmStore.getState().confirm({ title: t("mailArchive.delete"), message: t("mailArchive.deleteWarning"), confirmLabel: t("mailArchive.deletePermanently", "Delete permanently"), destructive: true })) { await api.deleteArchiveMessage(archiveId, detail.data!.id); clearBrowser(); await refresh(); } })}>{t("mailArchive.delete")}</Button>
        </div>
        <p className="mail-archive-note">{t("mailArchive.safePreview")}</p>
        <pre>{detail.data.metadata.body_text || t("mailArchive.noPlainBody")}</pre>
        <h4>{t("mailArchive.attachments")}</h4><ul className="mail-archive-attachments">{detail.data.metadata.attachments.map((attachment, index) => <li key={index}>
          {attachment.filename} ({attachment.size} B){attachment.is_inline && ` — ${t("mailArchive.inline")}`}{attachment.content_id && ` CID: ${attachment.content_id}`}<code>SHA256: {attachment.sha256}</code>
          <Button size="sm" onClick={() => void perform(async () => { await api.saveArchiveAttachment(archiveId, detail.data!.id, index); })}>{t("mailArchive.saveAttachment")}</Button>
        </li>)}</ul><code>MIME SHA256: {detail.data.fingerprint}</code>
      </article>}
    </fieldset>
    <MailboxExportPanel busy={busy} format={format} runExport={runExport} />
  </section>;
}

function MailboxExportPanel({ busy, format, runExport }: { busy: boolean; format: api.ArchiveExportFormat; runExport: (source: api.ArchiveExportSource) => Promise<void> }) {
  const { t } = useTranslation();
  const { data: accounts = [] } = useAccountsQuery();
  const activeAccount = useMailStore((s) => s.activeAccountId);
  const selectedIds = useMailStore((s) => s.selectedMessageIds);
  const single = useMailStore((s) => s.selectedMessageId);
  const [choice, setChoice] = useState("");
  const [folderId, setFolderId] = useState("");
  const accountId = choice || activeAccount || accounts[0]?.id || "";
  const { data: folders = [] } = useFoldersQuery(accountId || null);
  const selected = selectedIds.size ? [...selectedIds] : single ? [single] : [];
  return <fieldset disabled={busy}><legend>{t("mailArchive.mailboxExport")}</legend>
    <p className="mail-archive-note">{t("mailArchive.mailboxNote")}</p>
    <div className="mail-archive-row"><label>{t("mailArchive.exportAccount")}<select className="select" value={accountId} onChange={(e) => { setChoice(e.target.value); setFolderId(""); }}>{accounts.map((account) => <option key={account.id} value={account.id}>{account.email}</option>)}</select></label>
      <label>{t("mailArchive.exportMailboxFolder")}<select className="select" value={folderId} onChange={(e) => setFolderId(e.target.value)}><option value="">{t("mailArchive.chooseFolder")}</option>{folders.map((folder) => <option key={folder.id} value={folder.id}>{folder.name}</option>)}</select></label></div>
    <div className="mail-archive-row"><Button disabled={!folderId || !accountId} onClick={() => void runExport({ kind: "mailbox", account_id: accountId, folder_id: folderId, message_ids: [] })}>{t("mailArchive.exportMailbox", { format })}</Button>
      <Button disabled={!selected.length || accountId !== activeAccount} onClick={() => void runExport({ kind: "mailbox", account_id: accountId, folder_id: null, message_ids: selected })}>{t("mailArchive.exportMailboxSelected", { count: selected.length, format })}</Button></div>
  </fieldset>;
}
