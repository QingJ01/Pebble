mod commands;
mod events;
mod snooze_watcher;
mod state;

use state::AppState;
use std::path::PathBuf;
use tauri::Manager;

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
            let db_path = get_db_path(app)?;
            tracing::info!("Database path: {}", db_path.display());
            let store = pebble_store::Store::open(&db_path)?;
            tracing::info!("Database initialized successfully");

            let index_path = get_index_path(app)?;
            tracing::info!("Search index path: {}", index_path.display());
            let search = pebble_search::TantivySearch::open(&index_path)?;
            let search_needs_reindex = search.needs_reindex();
            tracing::info!("Search index initialized successfully");

            // Determine if search index needs rebuilding
            let needs_rebuild = if search_needs_reindex {
                tracing::info!("Search index schema changed, rebuild required");
                true
            } else if search.doc_count() == 0 {
                let db_count = store.count_all_messages().unwrap_or(0);
                if db_count > 0 {
                    tracing::info!("Search index empty but DB has {db_count} messages, rebuild required");
                    true
                } else {
                    false
                }
            } else {
                // Consistency check: if counts diverge by >10%, rebuild
                let db_count = store.count_all_messages().unwrap_or(0);
                let idx_count = search.doc_count();
                let diff = (db_count as i64 - idx_count as i64).unsigned_abs();
                let threshold = (db_count.max(idx_count) / 10).max(5);
                if diff > threshold {
                    tracing::warn!(
                        "SQLite/Tantivy count mismatch (db={db_count}, index={idx_count}), rebuilding"
                    );
                    true
                } else {
                    false
                }
            };

            if needs_rebuild {
                match commands::sync_cmd::do_reindex(&store, &search) {
                    Ok(n) => tracing::info!("Reindexed {n} messages"),
                    Err(e) => tracing::error!("Failed to reindex: {e}"),
                }
            }

            let crypto = pebble_crypto::CryptoService::init()?;
            tracing::info!("Crypto service initialized successfully");

            let app_data = app
                .path()
                .app_data_dir()?;
            let attachments_dir = app_data.join("attachments");
            std::fs::create_dir_all(&attachments_dir)?;
            tracing::info!("Attachments directory: {}", attachments_dir.display());

            let (snooze_stop_tx, snooze_stop_rx) = std::sync::mpsc::channel::<()>();
            app.manage(AppState::new(store, search, crypto, snooze_stop_tx, attachments_dir));

            // Start snooze watcher on the Tauri async runtime
            let state: tauri::State<AppState> = app.state();
            let store_clone = state.store.clone();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(snooze_watcher::run_snooze_watcher(
                store_clone,
                app_handle.clone(),
                snooze_stop_rx,
            ));

            // Auto-resume sync for all existing accounts
            tauri::async_runtime::spawn(async move {
                commands::sync_cmd::resume_all_syncs(app_handle).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health::health_check,
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
            commands::cloud_sync::test_webdav_connection,
            commands::cloud_sync::backup_to_webdav,
            commands::cloud_sync::restore_from_webdav,
            commands::contacts::search_contacts,
            commands::advanced_search::advanced_search,
            commands::sync_cmd::reindex_search,
            commands::notifications::set_notifications_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
