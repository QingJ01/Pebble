use super::{codec::*, ArchiveJobs, ArchiveReport};
use crate::state::AppState;
use lettre::message::{
    header::{self, ContentDisposition, ContentId, ContentType, HeaderName, HeaderValue, Headers},
    Mailbox, Mailboxes, MultiPart, SinglePart,
};
use pebble_core::{EmailAddress, Message, Result};
use pebble_store::Store;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportSource {
    Archive {
        archive_id: String,
        folder_id: Option<String>,
        message_ids: Vec<String>,
    },
    Mailbox {
        account_id: String,
        folder_id: Option<String>,
        message_ids: Vec<String>,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Eml,
    Mbox,
}
#[derive(Deserialize)]
pub struct ArchiveExportRequest {
    pub job_id: String,
    pub source: ExportSource,
    pub format: ExportFormat,
}

#[tauri::command]
pub async fn mail_archive_export(
    state: State<'_, AppState>,
    jobs: State<'_, ArchiveJobs>,
    request: ArchiveExportRequest,
) -> Result<Option<ArchiveReport>> {
    let Some(directory) = rfd::AsyncFileDialog::new().pick_folder().await else {
        return Ok(None);
    };
    let directory = directory.path().canonicalize().map_err(io_error)?;
    if !directory.is_dir() {
        return Err(invalid("Choose an existing export directory"));
    }
    let cancel = jobs.register(&request.job_id)?;
    let job_id = request.job_id.clone();
    let store = state.store.clone();
    let attachments = state.attachments_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_export(
            &store,
            &attachments,
            &directory,
            &request.source,
            &request.format,
            &cancel,
        )
    })
    .await
    .map_err(|_| pebble_core::PebbleError::Internal("Archive export worker failed".into()));
    jobs.finish(&job_id);
    Ok(Some(result??))
}

#[tauri::command]
pub async fn mail_archive_save_attachment(
    state: State<'_, AppState>,
    archive_id: String,
    message_id: String,
    attachment_index: usize,
) -> Result<Option<String>> {
    let Some(directory) = rfd::AsyncFileDialog::new().pick_folder().await else {
        return Ok(None);
    };
    let directory = directory.path().canonicalize().map_err(io_error)?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let raw = store.get_archive_mime(&archive_id, &message_id)?;
        let (_, parsed) = parse(&raw)?;
        let attachment = parsed
            .attachments
            .get(attachment_index)
            .ok_or_else(|| invalid("Archive attachment not found"))?;
        let filename = safe_filename(&attachment.meta.filename);
        let path = Path::new(&filename);
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("bin");
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("attachment");
        let mut file = AtomicExport::new(&directory, stem, extension)?;
        file.write_all(&attachment.data).map_err(io_error)?;
        let filename = file
            .target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        file.publish()?;
        Ok(Some(filename))
    })
    .await
    .map_err(|_| pebble_core::PebbleError::Internal("Archive attachment worker failed".into()))?
}

fn ids(store: &Store, source: &ExportSource) -> Result<Vec<String>> {
    let (selected, folder) = match source {
        ExportSource::Archive {
            message_ids,
            folder_id,
            ..
        }
        | ExportSource::Mailbox {
            message_ids,
            folder_id,
            ..
        } => (message_ids, folder_id),
    };
    if selected.len() > MAX_MESSAGES {
        return Err(invalid("Export exceeds 10,000 messages"));
    }
    if !selected.is_empty() {
        if folder.is_some() {
            return Err(invalid("Choose selected messages OR a folder"));
        }
        let mut seen = std::collections::HashSet::new();
        return Ok(selected
            .iter()
            .filter(|id| seen.insert((*id).clone()))
            .cloned()
            .collect());
    }
    let folder = folder
        .as_deref()
        .ok_or_else(|| invalid("Choose messages or a folder to export"))?;
    let mut result = Vec::new();
    loop {
        let page: Vec<String> = match source {
            ExportSource::Archive { archive_id, .. } => store
                .list_archive_messages(archive_id, Some(folder), "", result.len() as u32)?
                .into_iter()
                .map(|m| m.id)
                .collect(),
            ExportSource::Mailbox { account_id, .. } => {
                if !store
                    .list_folders(account_id)?
                    .iter()
                    .any(|f| f.id == folder)
                {
                    return Err(invalid("Mailbox folder/account mismatch"));
                }
                store
                    .list_messages_by_folder(folder, 100, result.len() as u32)?
                    .into_iter()
                    .map(|m| m.id)
                    .collect()
            }
        };
        if page.is_empty() {
            break;
        }
        result.extend(page);
        if result.len() > MAX_MESSAGES {
            return Err(invalid("Export exceeds 10,000 messages"));
        }
    }
    Ok(result)
}

fn mime(store: &Store, root: &Path, source: &ExportSource, id: &str) -> Result<Vec<u8>> {
    match source {
        ExportSource::Archive { archive_id, .. } => store.get_archive_mime(archive_id, id),
        ExportSource::Mailbox { account_id, .. } => {
            let message = store
                .get_message(id)?
                .ok_or_else(|| invalid("Mailbox message no longer exists"))?;
            if &message.account_id != account_id {
                return Err(invalid("Mailbox message/account mismatch"));
            }
            serialize_cached_message(store, root, &message)
        }
    }
}

fn mailbox(address: &EmailAddress) -> Result<Mailbox> {
    if address
        .name
        .as_ref()
        .is_some_and(|s| s.contains(['\r', '\n', '\0']))
    {
        return Err(invalid("Unsafe recipient header"));
    }
    Ok(Mailbox::new(
        address.name.clone(),
        address
            .address
            .parse()
            .map_err(|_| invalid("Invalid cached email address"))?,
    ))
}

fn serialize_cached_message(store: &Store, root: &Path, message: &Message) -> Result<Vec<u8>> {
    if message.subject.contains(['\r', '\n', '\0'])
        || message.from_name.contains(['\r', '\n', '\0'])
    {
        return Err(invalid("Unsafe cached message header"));
    }
    let mut headers = Headers::new();
    headers.set(header::From::from(Mailboxes::from(mailbox(
        &EmailAddress {
            name: Some(message.from_name.clone()),
            address: message.from_address.clone(),
        },
    )?)));
    headers.set(header::Subject::from(message.subject.clone()));
    let date = chrono::DateTime::from_timestamp(message.date, 0)
        .filter(|date| (0..=9999).contains(&chrono::Datelike::year(date)))
        .ok_or_else(|| invalid("Invalid message date"))?;
    headers.insert_raw(HeaderValue::new(
        HeaderName::new_from_ascii_str("Date"),
        date.to_rfc2822(),
    ));
    let recipients = |addresses: &[EmailAddress]| -> Result<Mailboxes> {
        Ok(addresses
            .iter()
            .map(mailbox)
            .collect::<Result<Vec<_>>>()?
            .into())
    };
    if !message.to_list.is_empty() {
        headers.set(header::To::from(recipients(&message.to_list)?));
    }
    if !message.cc_list.is_empty() {
        headers.set(header::Cc::from(recipients(&message.cc_list)?));
    }
    if !message.bcc_list.is_empty() {
        headers.set(header::Bcc::from(recipients(&message.bcc_list)?));
    }
    for header in [
        &message.message_id_header,
        &message.in_reply_to,
        &message.references_header,
    ]
    .into_iter()
    .flatten()
    {
        if header.contains(['\r', '\n', '\0']) {
            return Err(invalid("Unsafe cached message ID header"));
        }
    }
    if let Some(id) = &message.message_id_header {
        headers.set(header::MessageId::from(id.clone()));
    }
    if let Some(id) = &message.in_reply_to {
        headers.set(header::InReplyTo::from(id.clone()));
    }
    if let Some(ids) = &message.references_header {
        headers.set(header::References::from(ids.clone()));
    }
    if message.body_text.len() > MAX_BODY_BYTES || message.body_html_raw.len() > MAX_BODY_BYTES {
        return Err(invalid("Cached body exceeds archive limit"));
    }
    let alternative = MultiPart::alternative()
        .singlepart(SinglePart::plain(message.body_text.clone()))
        .singlepart(SinglePart::html(message.body_html_raw.clone()));
    let mut mixed = MultiPart::mixed().multipart(alternative);
    let attachments = store.list_attachments_by_message(&message.id)?;
    if attachments.len() > 256 {
        return Err(invalid("Too many attachments"));
    }
    if message.has_attachments && attachments.is_empty() {
        return Err(invalid("Attachments are not fully cached; export refused"));
    }
    let mut total = 0usize;
    for attachment in attachments {
        let path = Path::new(
            attachment
                .local_path
                .as_deref()
                .ok_or_else(|| invalid("Attachment is not locally cached"))?,
        )
        .canonicalize()
        .map_err(io_error)?;
        let root = root.canonicalize().map_err(io_error)?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(invalid("Attachment is outside profile attachment storage"));
        }
        let mut content = Vec::new();
        File::open(path)
            .map_err(io_error)?
            .take(MAX_ATTACHMENT_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .map_err(io_error)?;
        total += content.len();
        if content.len() > MAX_ATTACHMENT_BYTES || total > MAX_ATTACHMENTS_BYTES {
            return Err(invalid("Attachments exceed archive limits"));
        }
        let filename = safe_filename(&attachment.filename);
        let disposition = if attachment.is_inline {
            ContentDisposition::inline_with_name(&filename)
        } else {
            ContentDisposition::attachment(&filename)
        };
        let mut part = SinglePart::builder()
            .header(
                attachment
                    .mime_type
                    .parse::<ContentType>()
                    .map_err(|_| invalid("Invalid attachment MIME type"))?,
            )
            .header(disposition);
        if let Some(cid) = attachment.content_id {
            if cid.contains(['\r', '\n', '\0']) {
                return Err(invalid("Unsafe content ID"));
            }
            part = part.header(ContentId::from(format!(
                "<{}>",
                cid.trim_matches(['<', '>'])
            )));
        }
        mixed = mixed.singlepart(part.body(content));
    }
    headers.insert_raw(HeaderValue::new(
        HeaderName::new_from_ascii_str("MIME-Version"),
        "1.0".into(),
    ));
    // Serialize MIME, not an SMTP envelope: archives may have no recipients.
    // Typed lettre headers still provide Unicode encoding and injection safety.
    let mut raw = headers.to_string().into_bytes();
    raw.extend_from_slice(&mixed.formatted());
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(invalid("Serialized message exceeds 32 MiB"));
    }
    Ok(raw)
}

fn safe_filename(value: &str) -> String {
    let mut bytes = 0usize;
    let name: String = value
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= 96
        })
        .collect();
    let name = name.trim().trim_matches('.');
    if name.is_empty() {
        "message".into()
    } else {
        name.into()
    }
}

struct AtomicExport {
    temporary: PathBuf,
    target: PathBuf,
    writer: BufWriter<File>,
    bytes: u64,
}
impl AtomicExport {
    fn new(directory: &Path, stem: &str, extension: &str) -> Result<Self> {
        let temporary = directory.join(format!(".pebble-{}.tmp", pebble_core::new_id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let writer = BufWriter::new(options.open(&temporary).map_err(io_error)?);
        let target = directory.join(format!(
            "{}-{}.{}",
            safe_filename(stem),
            pebble_core::new_id(),
            extension
        ));
        Ok(Self {
            temporary,
            target,
            writer,
            bytes: 0,
        })
    }
    fn publish(mut self) -> Result<()> {
        self.writer.flush().map_err(io_error)?;
        self.writer.get_ref().sync_all().map_err(io_error)?;
        // Linking is atomic and cannot overwrite an existing name (unlike rename
        // on Unix). Both paths are in the user-chosen directory/filesystem.
        std::fs::hard_link(&self.temporary, &self.target).map_err(io_error)?;
        Ok(())
    }
}
impl Drop for AtomicExport {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temporary);
    }
}
impl Write for AtomicExport {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self.bytes + buffer.len() as u64 > MAX_TOTAL_BYTES {
            return Err(std::io::Error::other("Archive output exceeds 512 MiB"));
        }
        let count = self.writer.write(buffer)?;
        self.bytes += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

fn run_export(
    store: &Store,
    attachments: &Path,
    directory: &Path,
    source: &ExportSource,
    format: &ExportFormat,
    cancel: &AtomicBool,
) -> Result<ArchiveReport> {
    let ids = ids(store, source)?;
    if ids.is_empty() {
        return Err(invalid("No messages to export"));
    }
    let mut report = ArchiveReport::default();
    let mut total = 0u64;
    let mut mbox = if matches!(format, ExportFormat::Mbox) {
        Some(AtomicExport::new(directory, "mail-archive", "mbox")?)
    } else {
        None
    };
    for id in &ids {
        if cancel.load(Ordering::Acquire) {
            report.cancelled = true;
            break;
        }
        report.processed += 1;
        let result = (|| -> Result<()> {
            let raw = mime(store, attachments, source, id)?;
            total += raw.len() as u64;
            if total > MAX_TOTAL_BYTES {
                return Err(invalid("Export exceeds 512 MiB"));
            }
            if let Some(file) = &mut mbox {
                write_mbox(file, &raw)?;
            } else {
                let mut file = AtomicExport::new(directory, "message", "eml")?;
                file.write_all(&raw).map_err(io_error)?;
                file.publish()?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => report.imported += 1,
            Err(error) => {
                let number = report.processed;
                super::record_failure(&mut report, format!("Message {number}: {error}"));
                if mbox.is_some() {
                    report.imported = 0;
                    return Ok(report);
                }
            }
        }
        if total > MAX_TOTAL_BYTES {
            break;
        }
    }
    if let Some(file) = mbox {
        if report.cancelled {
            report.imported = 0;
        } else if let Err(error) = file.publish() {
            report.imported = 0;
            super::record_failure(&mut report, error.to_string());
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::mail_archive::{codec, tests::Fixture};

    fn imported(fixture: &Fixture) -> String {
        let (metadata, _) = parse(codec::tests::MIME).unwrap();
        fixture
            .store
            .insert_archive_message(
                &fixture.archive.id,
                &fixture.folder.id,
                &fingerprint(codec::tests::MIME),
                codec::tests::MIME,
                &metadata,
            )
            .unwrap();
        fixture
            .store
            .list_archive_messages(&fixture.archive.id, None, "", 0)
            .unwrap()
            .remove(0)
            .id
    }

    #[test]
    fn original_eml_and_mbox_exports_preserve_mime_and_attachment_hashes() {
        let fixture = Fixture::new();
        let id = imported(&fixture);
        let output = fixture.root.join("exports");
        std::fs::create_dir(&output).unwrap();
        let source = ExportSource::Archive {
            archive_id: fixture.archive.id.clone(),
            folder_id: None,
            message_ids: vec![id.clone(), id],
        };
        for format in [ExportFormat::Eml, ExportFormat::Mbox] {
            let report = run_export(
                &fixture.store,
                &fixture.root,
                &output,
                &source,
                &format,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(report.imported, 1);
        }
        for entry in std::fs::read_dir(&output).unwrap() {
            let path = entry.unwrap().path();
            let raw = std::fs::read(&path).unwrap();
            let eml = if path.extension().unwrap() == "mbox" {
                let mut records = Vec::new();
                codec::read_mbox(std::io::Cursor::new(raw), |raw| {
                    records.push(raw);
                    Ok(true)
                })
                .unwrap();
                records.remove(0)
            } else {
                raw
            };
            assert_eq!(eml, codec::tests::MIME);
            let (meta, _) = parse(&eml).unwrap();
            assert_eq!(meta.attachments[1].sha256, fingerprint(&[0, 1, 2, 255]));
        }
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 2);
    }

    #[test]
    fn atomic_exports_never_overwrite_or_leave_partial_temporary_files() {
        let fixture = Fixture::new();
        let name = safe_filename(&"邮件😀".repeat(100));
        assert!(name.len() <= 96);
        let mut unicode = AtomicExport::new(&fixture.root, &name, "eml").unwrap();
        let path = unicode.target.clone();
        unicode.write_all(b"unicode name").unwrap();
        unicode.publish().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"unicode name");
        let target = fixture.root.join("existing.eml");
        std::fs::write(&target, b"original").unwrap();
        let mut file = AtomicExport::new(&fixture.root, "../../bad:name", "eml").unwrap();
        let temporary = file.temporary.clone();
        assert_eq!(file.target.parent(), Some(fixture.root.as_path()));
        file.write_all(b"new").unwrap();
        file.target = target.clone();
        assert!(file.publish().is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(!temporary.exists());
        {
            let mut file = AtomicExport::new(&fixture.root, "partial", "mbox").unwrap();
            file.write_all(b"partial").unwrap();
        }
        assert!(!std::fs::read_dir(&fixture.root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".pebble-")));
    }

    #[test]
    fn failures_do_not_publish_partial_mbox_but_eml_reports_successful_subset() {
        let fixture = Fixture::new();
        let id = imported(&fixture);
        let output = fixture.root.join("exports");
        std::fs::create_dir(&output).unwrap();
        let source = ExportSource::Archive {
            archive_id: fixture.archive.id.clone(),
            folder_id: None,
            message_ids: vec![id, "missing".into()],
        };
        let report = run_export(
            &fixture.store,
            &fixture.root,
            &output,
            &source,
            &ExportFormat::Mbox,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!((report.imported, report.failed), (0, 1));
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
        let report = run_export(
            &fixture.store,
            &fixture.root,
            &output,
            &source,
            &ExportFormat::Eml,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!((report.imported, report.failed), (1, 1));
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
        assert!(run_export(
            &fixture.store,
            &fixture.root,
            &fixture.root.join("nonexistent"),
            &source,
            &ExportFormat::Mbox,
            &AtomicBool::new(false)
        )
        .is_err());
    }

    #[test]
    fn cached_mime_round_trip_preserves_unicode_bcc_old_dates_html_cid_and_binary_data() {
        let fixture = Fixture::new();
        let (metadata, parsed) = parse(codec::tests::MIME).unwrap();
        let root = fixture.root.join("attachments");
        std::fs::create_dir(&root).unwrap();
        let mut message = Message {
            id: "cached".into(),
            account_id: "account".into(),
            remote_id: "provider-id".into(),
            message_id_header: parsed.message_id_header.clone(),
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: parsed.subject,
            from_address: parsed.from_address,
            from_name: parsed.from_name,
            to_list: parsed.to_list,
            cc_list: parsed.cc_list,
            bcc_list: parsed.bcc_list,
            date: parsed.date,
            body_text: parsed.body_text,
            body_html_raw: parsed.body_html,
            snippet: "".into(),
            has_attachments: true,
            is_read: false,
            is_starred: false,
            is_draft: false,
            is_deleted: false,
            deleted_at: None,
            remote_version: None,
            created_at: 0,
            updated_at: 0,
        };
        fixture.store.insert_message(&message, &[]).unwrap();
        for (index, attachment) in parsed.attachments.into_iter().enumerate() {
            let path = root.join(format!("{index}.bin"));
            std::fs::write(&path, attachment.data).unwrap();
            fixture
                .store
                .insert_attachment(&pebble_core::Attachment {
                    id: format!("attachment-{index}"),
                    message_id: message.id.clone(),
                    filename: attachment.meta.filename,
                    mime_type: attachment.meta.mime_type,
                    size: attachment.meta.size as i64,
                    local_path: Some(path.to_string_lossy().into()),
                    content_id: attachment.meta.content_id,
                    is_inline: attachment.meta.is_inline,
                })
                .unwrap();
        }
        let raw = serialize_cached_message(&fixture.store, &root, &message).unwrap();
        let (roundtrip, parsed) = parse(&raw).unwrap();
        assert_eq!(roundtrip.subject, metadata.subject);
        assert_eq!(roundtrip.date, metadata.date);
        assert_eq!(roundtrip.bcc_list[0].address, "hidden@example.com");
        assert_eq!(roundtrip.attachments.len(), 2);
        assert_eq!(
            roundtrip.attachments[0].content_id,
            metadata.attachments[0].content_id
        );
        assert_eq!(
            roundtrip.attachments[1].sha256,
            metadata.attachments[1].sha256
        );
        assert!(parsed.body_html.contains("cid:picture"));
        message.date = -1;
        message.to_list.clear();
        message.cc_list.clear();
        message.bcc_list.clear();
        let (_, old) =
            parse(&serialize_cached_message(&fixture.store, &root, &message).unwrap()).unwrap();
        assert_eq!(old.date, -1);
        message.date = i64::MAX;
        assert!(serialize_cached_message(&fixture.store, &root, &message).is_err());
        message.date = 0;
        message.subject = "bad\r\nBcc: injected@example.com".into();
        assert!(serialize_cached_message(&fixture.store, &root, &message).is_err());
        message.subject = "safe".into();
        std::fs::remove_file(root.join("0.bin")).unwrap();
        assert!(serialize_cached_message(&fixture.store, &root, &message).is_err());
    }
}
