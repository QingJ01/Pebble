mod codec;
pub mod export;

use crate::state::AppState;
use codec::{fingerprint, invalid, io_error, MAX_FILES, MAX_MESSAGES, MAX_TOTAL_BYTES};
use pebble_core::{PebbleError, Result};
use pebble_store::{mail_archive::*, Store};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, State};

#[derive(Default)]
pub struct ArchiveJobs {
    selection: Mutex<Option<(String, Vec<PathBuf>)>>,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Clone, Serialize)]
pub struct ArchiveFileInfo {
    pub name: String,
    pub bytes: u64,
}
#[derive(Serialize)]
pub struct ArchiveSelection {
    pub id: String,
    pub files: Vec<ArchiveFileInfo>,
}
#[derive(Clone, Default, Serialize)]
pub struct ArchiveReport {
    pub processed: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub failed: usize,
    pub cancelled: bool,
    pub errors: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct ArchiveProgress {
    pub job_id: String,
    pub processed: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub failed: usize,
}

#[derive(Deserialize)]
pub struct ArchiveImportRequest {
    pub job_id: String,
    pub selection_id: String,
    pub archive_id: String,
    pub folder_id: String,
    pub preview: bool,
}

fn locked() -> PebbleError {
    PebbleError::Internal("Archive state unavailable".into())
}

impl ArchiveJobs {
    fn register(&self, id: &str) -> Result<Arc<AtomicBool>> {
        let mut jobs = self.jobs.lock().map_err(|_| locked())?;
        if id.len() > 100 || id.is_empty() || jobs.len() >= 2 || jobs.contains_key(id) {
            return Err(invalid("Archive job already running or invalid"));
        }
        let flag = Arc::new(AtomicBool::new(false));
        jobs.insert(id.into(), flag.clone());
        Ok(flag)
    }
    fn finish(&self, id: &str) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(id);
        }
    }
}

fn source_file(path: &Path) -> Result<File> {
    if !path.is_absolute()
        || !std::fs::symlink_metadata(path)
            .map_err(io_error)?
            .file_type()
            .is_file()
    {
        return Err(invalid(
            "Archive source must be a regular, non-symlink file",
        ));
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension != "eml" && extension != "mbox" {
        return Err(invalid("Choose .eml or .mbox files"));
    }
    let file = File::open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid("Archive source is not a regular file"));
    }
    Ok(file)
}

#[tauri::command]
pub async fn mail_archive_choose_files(
    jobs: State<'_, ArchiveJobs>,
) -> Result<Option<ArchiveSelection>> {
    let Some(files) = rfd::AsyncFileDialog::new()
        .add_filter("Email archive", &["eml", "mbox"])
        .pick_files()
        .await
    else {
        return Ok(None);
    };
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(invalid("Choose at most 64 archive files"));
    }
    let mut paths = Vec::new();
    let mut infos = Vec::new();
    let mut total = 0u64;
    for selected in files {
        let path = selected.path().to_path_buf();
        let file = source_file(&path)?;
        let len = file.metadata().map_err(io_error)?.len();
        total = total
            .checked_add(len)
            .ok_or_else(|| invalid("Archive files exceed 512 MiB"))?;
        if total > MAX_TOTAL_BYTES {
            return Err(invalid("Archive files exceed 512 MiB"));
        }
        infos.push(ArchiveFileInfo {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            bytes: len,
        });
        paths.push(path);
    }
    let id = pebble_core::new_id();
    *jobs.selection.lock().map_err(|_| locked())? = Some((id.clone(), paths));
    Ok(Some(ArchiveSelection { id, files: infos }))
}

#[tauri::command]
pub async fn mail_archive_list(state: State<'_, AppState>) -> Result<Vec<MailArchive>> {
    state.store.list_mail_archives()
}
#[tauri::command]
pub async fn mail_archive_create(
    state: State<'_, AppState>,
    account_id: String,
    name: String,
) -> Result<MailArchive> {
    state.store.create_mail_archive(&account_id, &name)
}
#[tauri::command]
pub async fn mail_archive_folders(
    state: State<'_, AppState>,
    archive_id: String,
) -> Result<Vec<ArchiveFolder>> {
    state.store.list_archive_folders(&archive_id)
}
#[tauri::command]
pub async fn mail_archive_create_folder(
    state: State<'_, AppState>,
    archive_id: String,
    name: String,
) -> Result<ArchiveFolder> {
    state.store.create_archive_folder(&archive_id, &name)
}
#[tauri::command]
pub async fn mail_archive_messages(
    state: State<'_, AppState>,
    archive_id: String,
    folder_id: Option<String>,
    query: String,
    offset: u32,
) -> Result<Vec<ArchiveMessageSummary>> {
    state
        .store
        .list_archive_messages(&archive_id, folder_id.as_deref(), &query, offset)
}
#[tauri::command]
pub async fn mail_archive_read(
    state: State<'_, AppState>,
    archive_id: String,
    message_id: String,
) -> Result<ArchiveMessageDetail> {
    state.store.get_archive_message(&archive_id, &message_id)
}
#[tauri::command]
pub async fn mail_archive_update(
    state: State<'_, AppState>,
    archive_id: String,
    message_id: String,
    folder_id: String,
    is_read: bool,
    is_starred: bool,
) -> Result<()> {
    state
        .store
        .update_archive_message(&archive_id, &message_id, &folder_id, is_read, is_starred)
}
#[tauri::command]
pub async fn mail_archive_delete(
    state: State<'_, AppState>,
    archive_id: String,
    message_id: String,
) -> Result<()> {
    state.store.delete_archive_message(&archive_id, &message_id)
}
#[tauri::command]
pub async fn mail_archive_cancel(jobs: State<'_, ArchiveJobs>, job_id: String) -> Result<()> {
    if let Some(flag) = jobs.jobs.lock().map_err(|_| locked())?.get(&job_id) {
        flag.store(true, Ordering::Release);
    }
    Ok(())
}

#[tauri::command]
pub async fn mail_archive_import(
    state: State<'_, AppState>,
    jobs: State<'_, ArchiveJobs>,
    app: tauri::AppHandle,
    request: ArchiveImportRequest,
) -> Result<ArchiveReport> {
    let paths = {
        let selected = jobs.selection.lock().map_err(|_| locked())?;
        let (id, paths) = selected
            .as_ref()
            .ok_or_else(|| invalid("Choose archive files first"))?;
        if id != &request.selection_id {
            return Err(invalid("Archive selection expired; choose files again"));
        }
        paths.clone()
    };
    let cancel = jobs.register(&request.job_id)?;
    let job_id = request.job_id.clone();
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_import(
            &store,
            &paths,
            &request.archive_id,
            &request.folder_id,
            request.preview,
            &cancel,
            |report| {
                let _ = app.emit(
                    "mail-archive:progress",
                    ArchiveProgress {
                        job_id: request.job_id.clone(),
                        processed: report.processed,
                        imported: report.imported,
                        duplicates: report.duplicates,
                        failed: report.failed,
                    },
                );
            },
        )
    })
    .await
    .map_err(|_| PebbleError::Internal("Archive worker failed".into()));
    jobs.finish(&job_id);
    result?
}

struct BoundedFile {
    file: File,
    remaining: u64,
}
impl Read for BoundedFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0u8];
            return if self.file.read(&mut probe)? == 0 {
                Ok(0)
            } else {
                Err(std::io::Error::other("Archive source grew while reading"))
            };
        }
        let len = buffer.len().min(self.remaining as usize);
        let count = self.file.read(&mut buffer[..len])?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Archive source shrank while reading",
            ));
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

fn run_import(
    store: &Store,
    paths: &[PathBuf],
    archive_id: &str,
    folder_id: &str,
    preview: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&ArchiveReport),
) -> Result<ArchiveReport> {
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err(invalid("Choose 1–64 archive files"));
    }
    if !store
        .list_archive_folders(archive_id)?
        .iter()
        .any(|f| f.id == folder_id)
    {
        return Err(invalid("Choose a local archive destination"));
    }
    let mut report = ArchiveReport::default();
    let mut total = 0u64;
    let mut seen = HashSet::new();
    for (file_index, path) in paths.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            report.cancelled = true;
            break;
        }
        let mut process_file = || -> Result<()> {
            let file = source_file(path)?;
            let len = file.metadata().map_err(io_error)?.len();
            total = total
                .checked_add(len)
                .ok_or_else(|| invalid("Archive import exceeds 512 MiB"))?;
            if total > MAX_TOTAL_BYTES {
                return Err(invalid("Archive import exceeds 512 MiB"));
            }
            let reader = BufReader::new(BoundedFile {
                file,
                remaining: len,
            });
            let mut accept = |raw: Vec<u8>| -> Result<bool> {
                if cancel.load(Ordering::Acquire) {
                    report.cancelled = true;
                    return Ok(false);
                }
                if report.processed >= MAX_MESSAGES {
                    return Err(invalid("Archive import exceeds 10,000 messages"));
                }
                report.processed += 1;
                let result = (|| -> Result<bool> {
                    let (metadata, _) = codec::parse(&raw)?;
                    let hash = fingerprint(&raw);
                    let exists = if preview {
                        !seen.insert(hash.clone())
                            || store.archive_has_content(archive_id, &hash)?
                    } else {
                        !store
                            .insert_archive_message(archive_id, folder_id, &hash, &raw, &metadata)?
                    };
                    Ok(!exists)
                })();
                match result {
                    Ok(true) => report.imported += 1,
                    Ok(false) => report.duplicates += 1,
                    Err(error) => {
                        let number = report.processed;
                        record_failure(
                            &mut report,
                            format!("File {}, message {number}: {error}", file_index + 1),
                        );
                    }
                }
                progress(&report);
                Ok(true)
            };
            if path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .eq_ignore_ascii_case("eml")
            {
                accept(codec::read_eml(reader)?)?;
            } else {
                codec::read_mbox(reader, accept)?;
            }
            Ok(())
        };
        if let Err(error) = process_file() {
            record_failure(&mut report, format!("File {}: {error}", file_index + 1));
            progress(&report);
        }
        if report.processed >= MAX_MESSAGES || total > MAX_TOTAL_BYTES {
            if file_index + 1 < paths.len() {
                record_failure(
                    &mut report,
                    "Remaining files were not processed because an archive limit was reached"
                        .into(),
                );
                progress(&report);
            }
            break;
        }
    }
    report.cancelled |= cancel.load(Ordering::Acquire);
    Ok(report)
}

fn record_failure(report: &mut ArchiveReport, error: String) {
    report.failed += 1;
    if report.errors.len() < 100 {
        report.errors.push(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub struct Fixture {
        pub root: PathBuf,
        pub store: Store,
        pub archive: MailArchive,
        pub folder: ArchiveFolder,
    }
    impl Fixture {
        pub fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "pebble-mail-archive-test-{}",
                pebble_core::new_id()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let db = root.join("mail.db");
            let store = Store::open(&db).unwrap();
            let conn = rusqlite::Connection::open(&db).unwrap();
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='mail_archives')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            if !exists {
                conn.execute_batch(include_str!(
                    "../../../../crates/pebble-store/src/mail_archive_schema.sql"
                ))
                .unwrap();
            }
            store
                .insert_account(&pebble_core::Account {
                    id: "account".into(),
                    email: "test@example.invalid".into(),
                    display_name: "Test".into(),
                    account_label: None,
                    provider_display_name: None,
                    color: None,
                    provider: pebble_core::ProviderType::Gmail,
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
            let archive = store
                .create_mail_archive("account", "Local archive")
                .unwrap();
            let folder = store.list_archive_folders(&archive.id).unwrap().remove(0);
            Self {
                root,
                store,
                archive,
                folder,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn imports_original_mime_and_index_without_authentication_providers_or_remote_ids() {
        let fixture = Fixture::new();
        let path = fixture.root.join("message.eml");
        std::fs::write(&path, codec::tests::MIME).unwrap();
        let run = |preview| {
            run_import(
                &fixture.store,
                std::slice::from_ref(&path),
                &fixture.archive.id,
                &fixture.folder.id,
                preview,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap()
        };
        assert_eq!(run(true).imported, 1);
        assert!(fixture
            .store
            .list_archive_messages(&fixture.archive.id, None, "", 0)
            .unwrap()
            .is_empty());
        assert_eq!(run(false).imported, 1);
        assert_eq!(run(false).duplicates, 1);
        let rows = fixture
            .store
            .list_archive_messages(&fixture.archive.id, None, "Unicode", 0)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            fixture
                .store
                .get_archive_mime(&fixture.archive.id, &rows[0].id)
                .unwrap(),
            codec::tests::MIME
        );
        assert_eq!(
            fixture
                .store
                .get_archive_message(&fixture.archive.id, &rows[0].id)
                .unwrap()
                .metadata
                .attachments[1]
                .sha256,
            codec::fingerprint(&[0, 1, 2, 255])
        );
        // A Gmail account with no credentials cannot connect. Import needs only
        // Store and never constructs any provider or queues a provider operation.
        let conn = rusqlite::Connection::open(fixture.root.join("mail.db")).unwrap();
        for table in ["messages", "pending_mail_ops"] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
    }

    #[test]
    fn mixed_mbox_failures_are_reported_and_cancel_keeps_only_completed_messages() {
        let fixture = Fixture::new();
        let path = fixture.root.join("mixed.mbox");
        let mut mbox = Vec::new();
        codec::write_mbox(&mut mbox, codec::tests::MIME).unwrap();
        codec::write_mbox(&mut mbox, b"not MIME\n").unwrap();
        codec::write_mbox(&mut mbox, b"Subject: distinct\n\nFrom body\n").unwrap();
        std::fs::write(&path, &mbox).unwrap();
        let result = run_import(
            &fixture.store,
            std::slice::from_ref(&path),
            &fixture.archive.id,
            &fixture.folder.id,
            false,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(
            (result.processed, result.imported, result.failed),
            (3, 2, 1)
        );
        assert_eq!(result.errors.len(), 1);
        // Cancellation must retain a newly committed message, independently of
        // the account-wide duplicate receipts created by the first import.
        let mut fresh = Vec::new();
        codec::write_mbox(&mut fresh, b"Subject: fresh first\n\nnew body\n").unwrap();
        codec::write_mbox(&mut fresh, b"Subject: fresh second\n\nnew body\n").unwrap();
        std::fs::write(&path, fresh).unwrap();
        let archive = fixture
            .store
            .create_mail_archive("account", "Cancel")
            .unwrap();
        let folder = fixture
            .store
            .list_archive_folders(&archive.id)
            .unwrap()
            .remove(0);
        let cancel = AtomicBool::new(false);
        let result = run_import(
            &fixture.store,
            &[path],
            &archive.id,
            &folder.id,
            false,
            &cancel,
            |report| {
                if report.processed == 1 {
                    cancel.store(true, Ordering::Release);
                }
            },
        )
        .unwrap();
        assert!(result.cancelled);
        assert_eq!(result.imported, 1);
        assert_eq!(
            fixture
                .store
                .list_archive_messages(&archive.id, None, "", 0)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn reads_bound_changed_files_and_reject_symlinks_or_missing_files() {
        let fixture = Fixture::new();
        let path = fixture.root.join("source.eml");
        std::fs::write(&path, b"12345").unwrap();
        let reader = BoundedFile {
            file: File::open(&path).unwrap(),
            remaining: 3,
        };
        assert!(codec::read_eml(reader).is_err());
        assert!(source_file(&fixture.root).is_err());
        assert!(source_file(&fixture.root.join("missing.eml")).is_err());
        #[cfg(unix)]
        {
            let link = fixture.root.join("link.eml");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(source_file(&link).is_err());
        }
        let result = run_import(
            &fixture.store,
            &[path, fixture.root.join("missing.eml")],
            &fixture.archive.id,
            &fixture.folder.id,
            false,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(result.failed, 2);
    }

    #[test]
    fn repeated_content_with_missing_ids_is_skipped_inside_preview() {
        let fixture = Fixture::new();
        let one = fixture.root.join("one.eml");
        let two = fixture.root.join("two.eml");
        std::fs::write(&one, b"Subject: no ID\n\nbody\n").unwrap();
        std::fs::copy(&one, &two).unwrap();
        let result = run_import(
            &fixture.store,
            &[one, two],
            &fixture.archive.id,
            &fixture.folder.id,
            true,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!((result.imported, result.duplicates), (1, 1));
        let jobs = ArchiveJobs::default();
        assert!(jobs.register("same").is_ok());
        assert!(jobs.register("same").is_err());
        jobs.finish("same");
        assert!(jobs.register("same").is_ok());
    }

    #[test]
    fn truncated_source_cannot_be_imported_as_an_incomplete_message() {
        let fixture = Fixture::new();
        let path = fixture.root.join("truncated.eml");
        std::fs::write(&path, b"Subject: mail\n\nbody").unwrap();
        let reader = BoundedFile {
            file: source_file(&path).unwrap(),
            remaining: 100,
        };
        assert!(codec::read_eml(reader).is_err());
    }
}
