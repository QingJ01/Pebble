mod commands;
mod events;
mod snooze_watcher;
mod state;

use state::AppState;
use std::path::PathBuf;
use std::time::Instant;
use tauri::{Emitter, Manager};

#[derive(Debug, PartialEq, Eq)]
struct StartupPhaseTiming {
    label: &'static str,
    phase_ms: u128,
    total_ms: u128,
}

fn startup_phase_timing(
    label: &'static str,
    start: Instant,
    phase_start: Instant,
    now: Instant,
) -> StartupPhaseTiming {
    StartupPhaseTiming {
        label,
        phase_ms: now.duration_since(phase_start).as_millis(),
        total_ms: now.duration_since(start).as_millis(),
    }
}

fn log_startup_phase(start: Instant, phase_start: &mut Instant, label: &'static str) {
    let now = Instant::now();
    let timing = startup_phase_timing(label, start, *phase_start, now);
    tracing::info!(
        "[startup] {}: {}ms phase, {}ms total",
        timing.label,
        timing.phase_ms,
        timing.total_ms
    );
    *phase_start = now;
}

fn get_db_path(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let app_data = app
        .path()
        .app_data_dir()?;
    std::fs::create_dir_all(&app_data)?;
    let db_dir = app_data.join("db");
    std::fs::create_dir_all(&db_dir)?;
    Ok(db_dir.join("pebble.db"))
}

fn get_index_path(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let app_data = app
        .path()
        .app_data_dir()?;
    let index_dir = app_data.join("search_index");
    std::fs::create_dir_all(&index_dir)?;
    Ok(index_dir)
}

#[cfg(test)]
mod startup_timing_tests {
    use super::startup_phase_timing;
    use std::time::{Duration, Instant};

    #[test]
    fn startup_phase_timing_reports_phase_and_total_elapsed_ms() {
        let start = Instant::now();
        let phase_start = start + Duration::from_millis(75);
        let now = start + Duration::from_millis(250);

        let timing = startup_phase_timing("search index opened", start, phase_start, now);

        assert_eq!(timing.label, "search index opened");
        assert_eq!(timing.phase_ms, 175);
        assert_eq!(timing.total_ms, 250);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "pebble=info,pebble_store=info,pebble_mail=info,pebble_search=info,pebble_translate=info,pebble_crypto=info,pebble_oauth=info".into()
            }),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let startup_start = Instant::now();
            let mut startup_phase = startup_start;
            tracing::info!("[startup] tauri setup started");

            let db_path = get_db_path(app)?;
            tracing::info!("Database path: {}", db_path.display());
            log_startup_phase(startup_start, &mut startup_phase, "app data paths resolved");

            let store = pebble_store::Store::open(&db_path)?;
            tracing::info!("Database initialized successfully");
            log_startup_phase(
                startup_start,
                &mut startup_phase,
                "database opened and migrations complete",
            );

            match store.quick_check() {
                Ok(result) if result == "ok" => tracing::info!("Database integrity check passed"),
                Ok(result) => tracing::warn!("Database integrity check warning: {}", result),
                Err(e) => tracing::warn!("Database integrity check failed: {}", e),
            }
            log_startup_phase(startup_start, &mut startup_phase, "database quick check complete");

            let index_path = get_index_path(app)?;
            tracing::info!("Search index path: {}", index_path.display());
            let search = pebble_search::TantivySearch::open(&index_path)?;
            let search_needs_reindex = search.needs_reindex();
            tracing::info!("Search index initialized successfully");
            log_startup_phase(startup_start, &mut startup_phase, "search index opened");

            // The full `SELECT COUNT(*) FROM messages` consistency check used
            // to run here and block the main window from appearing. It now
            // runs inside the background reindex task below, so startup can
            // proceed without waiting on a full-table scan.

            let crypto = pebble_crypto::CryptoService::init()?;
            tracing::info!("Crypto service initialized successfully");
            log_startup_phase(startup_start, &mut startup_phase, "crypto service initialized");

            let app_data = app
                .path()
                .app_data_dir()?;
            let attachments_dir = app_data.join("attachments");
            std::fs::create_dir_all(&attachments_dir)?;
            tracing::info!("Attachments directory: {}", attachments_dir.display());
            log_startup_phase(startup_start, &mut startup_phase, "attachments directory ready");

            let (snooze_stop_tx, snooze_stop_rx) = std::sync::mpsc::channel::<()>();
            app.manage(AppState::new(store, search, crypto, snooze_stop_tx, attachments_dir));
            log_startup_phase(startup_start, &mut startup_phase, "app state registered");

            // Start snooze watcher on the Tauri async runtime
            let state: tauri::State<AppState> = app.state();
            let store_clone = state.store.clone();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(snooze_watcher::run_snooze_watcher(
                store_clone,
                app_handle.clone(),
                snooze_stop_rx,
            ));

            // Decide whether to rebuild the search index, and do it in the
            // background so startup never waits on the DB count query. The
            // task itself performs the consistency check (comparing the
            // index doc count to the live DB row count) — the main thread
            // only needs the cheap schema-version flag.
            let store_for_reindex = state.store.clone();
            let search_for_reindex = state.search.clone();
            let app_for_reindex = app_handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                // 1. Process any pending search ops left over from a previous crash.
                let pending = store_for_reindex.list_search_pending().unwrap_or_default();
                if !pending.is_empty() {
                    tracing::info!("Recovering {} pending search operations from previous session", pending.len());
                    let mut ids_to_clear = Vec::with_capacity(pending.len());
                    for (msg_id, op) in &pending {
                        match op.as_str() {
                            "remove" => {
                                let _ = search_for_reindex.remove_message(msg_id);
                            }
                            _ => {
                                match store_for_reindex.get_message(msg_id) {
                                    Ok(Some(msg)) if !msg.is_deleted => {
                                        let folder_ids = store_for_reindex.get_message_folder_ids(msg_id).unwrap_or_default();
                                        if folder_ids.is_empty() {
                                            let _ = search_for_reindex.remove_message(msg_id);
                                        } else {
                                            let _ = search_for_reindex.index_message(&msg, &folder_ids);
                                        }
                                    }
                                    _ => { let _ = search_for_reindex.remove_message(msg_id); }
                                }
                            }
                        }
                        ids_to_clear.push(msg_id.clone());
                    }
                    let _ = search_for_reindex.commit();
                    let _ = store_for_reindex.clear_search_pending(&ids_to_clear);
                }

                // 2. Full rebuild if schema changed or counts diverge.
                let needs_rebuild = if search_needs_reindex {
                    tracing::info!("Search index schema changed, rebuild required");
                    true
                } else {
                    let idx_count = search_for_reindex.doc_count();
                    let db_count = store_for_reindex.count_all_messages().unwrap_or(0);
                    if idx_count == 0 && db_count > 0 {
                        tracing::info!("Search index empty but DB has {db_count} messages, rebuild required");
                        true
                    } else if idx_count > 0 && idx_count != db_count {
                        tracing::warn!(
                            "SQLite/Tantivy count mismatch (db={db_count}, index={idx_count}), rebuilding"
                        );
                        true
                    } else {
                        false
                    }
                };

                if needs_rebuild {
                    tracing::info!("Starting background search index rebuild...");
                    match commands::indexing::do_reindex(&store_for_reindex, &search_for_reindex) {
                        Ok(n) => {
                            tracing::info!("Background reindex complete: {n} messages indexed");
                            let _ = app_for_reindex.emit("search:reindex-complete", n);
                        }
                        Err(e) => tracing::error!("Background reindex failed: {e}"),
                    }
                    let _ = store_for_reindex.clear_all_search_pending();
                }
            });

            // Auto-resume sync for all existing accounts
            let app_for_sync = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                commands::sync_cmd::resume_all_syncs(app_for_sync).await;
            });

            let app_for_pending_ops = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                commands::pending_mail_ops::run_pending_mail_ops_worker(app_for_pending_ops).await;
            });
            log_startup_phase(startup_start, &mut startup_phase, "background workers scheduled");
            tracing::info!(
                "[startup] tauri setup complete: {}ms total",
                startup_start.elapsed().as_millis()
            );

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health::health_check,
            commands::health::check_for_update,
            commands::health::open_external_url,
            commands::accounts::add_account,
            commands::accounts::update_account,
            commands::accounts::list_accounts,
            commands::accounts::delete_account,
            commands::accounts::test_imap_connection,
            commands::accounts::test_account_connection,
            commands::folders::list_folders,
            commands::messages::query::list_messages,
            commands::messages::query::list_starred_messages,
            commands::messages::query::get_message,
            commands::messages::query::get_messages_batch,
            commands::messages::rendering::get_rendered_html,
            commands::messages::rendering::get_message_with_html,
            commands::messages::flags::update_message_flags,
            commands::messages::rendering::is_trusted_sender,
            commands::messages::lifecycle::archive_message,
            commands::messages::lifecycle::delete_message,
            commands::messages::lifecycle::restore_message,
            commands::messages::lifecycle::empty_trash,
            commands::messages::lifecycle::move_to_folder,
            commands::search::search_messages,
            commands::sync_cmd::start_sync,
            commands::sync_cmd::stop_sync,
            commands::kanban::move_to_kanban,
            commands::kanban::list_kanban_cards,
            commands::kanban::remove_from_kanban,
            commands::labels::get_message_labels,
            commands::labels::get_message_labels_batch,
            commands::labels::add_message_label,
            commands::labels::remove_message_label,
            commands::labels::list_labels,
            commands::snooze::snooze_message,
            commands::snooze::unsnooze_message,
            commands::snooze::list_snoozed,
            commands::rules::create_rule,
            commands::rules::list_rules,
            commands::rules::update_rule,
            commands::rules::delete_rule,
            commands::compose::send_email,
            commands::trusted_senders::trust_sender,
            commands::trusted_senders::list_trusted_senders,
            commands::trusted_senders::remove_trusted_sender,
            commands::translate::translate_text,
            commands::translate::get_translate_config,
            commands::translate::save_translate_config,
            commands::translate::test_translate_connection,
            commands::threads::list_thread_messages,
            commands::threads::list_threads,
            commands::oauth::complete_oauth_flow,
            commands::attachments::list_attachments,
            commands::attachments::get_attachment_path,
            commands::attachments::download_attachment,
            commands::batch::batch_archive,
            commands::batch::batch_delete,
            commands::batch::batch_mark_read,
            commands::batch::batch_star,
            commands::cloud_sync::test_webdav_connection,
            commands::cloud_sync::backup_to_webdav,
            commands::cloud_sync::preview_webdav_backup,
            commands::cloud_sync::restore_from_webdav,
            commands::contacts::search_contacts,
            commands::advanced_search::advanced_search,
            commands::sync_cmd::reindex_search,
            commands::notifications::set_notifications_enabled,
            commands::pending_mail_ops::get_pending_mail_ops_summary,
            commands::pending_mail_ops::list_pending_mail_ops,
            commands::drafts::save_draft,
            commands::drafts::delete_draft,
            commands::folder_counts::get_folder_unread_counts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
