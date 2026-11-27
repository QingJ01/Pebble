mod account_colors;
mod commands;
mod events;
#[cfg(test)]
mod ipc_bindings;
#[cfg(target_os = "linux")]
mod linux_migration;
mod profile;
mod realtime;
mod send_controls;
mod snooze_watcher;
mod startup;
mod state;

use serde::Serialize;
use state::AppState;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tauri::{
    menu::{Menu, MenuBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Listener, Manager, Runtime, WindowEvent,
};
use tracing_subscriber::prelude::*;

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();
const TRAY_SHOW_ID: &str = "tray-show";
const TRAY_HIDE_ID: &str = "tray-hide";
const TRAY_QUIT_ID: &str = "tray-quit";
#[cfg(target_os = "macos")]
const APPLICATION_QUIT_ID: &str = "application-quit";
const DEEP_LINK_NEW_URL_EVENT: &str = "deep-link://new-url";

#[derive(Default)]
struct PendingMailtoUrls(Mutex<Vec<String>>);

#[derive(Clone, Serialize)]
struct OpenMailtoPayload {
    urls: Vec<String>,
}

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

fn get_db_path(app_data: &std::path::Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // The caller (setup) has already created app_data.
    let db_dir = app_data.join("db");
    std::fs::create_dir_all(&db_dir)?;
    Ok(db_dir.join("pebble.db"))
}

fn get_index_path(app_data: &std::path::Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let index_dir = app_data.join("search_index");
    std::fs::create_dir_all(&index_dir)?;
    Ok(index_dir)
}

fn restore_main_window<R: Runtime>(app: &AppHandle<R>) {
    commands::notifications::clear_attention_indicator(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn is_mailto_url(value: &str) -> bool {
    value
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("mailto:")
}

fn unique_mailto_urls(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut urls = Vec::new();
    for value in values {
        let trimmed = value.trim().to_string();
        if is_mailto_url(&trimmed) && !urls.contains(&trimmed) {
            urls.push(trimmed);
        }
    }
    urls
}

fn mailto_urls_from_deep_link_payload(payload: &str) -> Vec<String> {
    if let Ok(urls) = serde_json::from_str::<Vec<String>>(payload) {
        return unique_mailto_urls(urls);
    }
    if let Ok(url) = serde_json::from_str::<String>(payload) {
        return unique_mailto_urls([url]);
    }
    unique_mailto_urls([payload.to_string()])
}

fn mailto_urls_from_args(args: &[String]) -> Vec<String> {
    unique_mailto_urls(args.iter().cloned())
}

fn record_mailto_urls<R: Runtime>(app: &AppHandle<R>, urls: Vec<String>) {
    let urls = unique_mailto_urls(urls);
    if urls.is_empty() {
        return;
    }

    let mut urls_to_emit = Vec::new();
    if let Some(state) = app.try_state::<PendingMailtoUrls>() {
        if let Ok(mut pending) = state.0.lock() {
            for url in &urls {
                if !pending.contains(url) {
                    pending.push(url.clone());
                    urls_to_emit.push(url.clone());
                }
            }
        }
    } else {
        urls_to_emit = urls;
    }

    if urls_to_emit.is_empty() {
        return;
    }

    restore_main_window(app);
    let _ = app.emit(
        events::APP_OPEN_MAILTO,
        OpenMailtoPayload { urls: urls_to_emit },
    );
}

fn build_tray_menu<R: Runtime, M: Manager<R>>(
    manager: &M,
    show_label: &str,
    hide_label: &str,
    quit_label: &str,
) -> tauri::Result<Menu<R>> {
    MenuBuilder::new(manager)
        .text(TRAY_SHOW_ID, show_label)
        .text(TRAY_HIDE_ID, hide_label)
        .separator()
        .text(TRAY_QUIT_ID, quit_label)
        .build()
}

#[cfg(target_os = "macos")]
fn setup_application_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadata, MenuItem, PredefinedMenuItem, Submenu};

    let package = app.package_info();
    let menu = Menu::default(app)?;
    let application_menu = Submenu::with_items(
        app,
        package.name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(
                app,
                None,
                Some(AboutMetadata {
                    name: Some(package.name.clone()),
                    version: Some(package.version.to_string()),
                    copyright: app.config().bundle.copyright.clone(),
                    authors: app.config().bundle.publisher.clone().map(|p| vec![p]),
                    ..Default::default()
                }),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            // The predefined Quit action calls NSApplication::terminate directly,
            // bypassing Tauri's cancellable ExitRequested event.
            &MenuItem::with_id(
                app,
                APPLICATION_QUIT_ID,
                format!("Quit {}", package.name),
                true,
                Some("CmdOrCtrl+Q"),
            )?,
        ],
    )?;
    menu.remove_at(0)?;
    menu.prepend(&application_menu)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id().as_ref() == APPLICATION_QUIT_ID {
            app.exit(0);
        }
    });
    Ok(())
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_tray_menu(app, "Show Window", "Hide Window", "Quit Pebble")?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("default window icon is not configured")?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .tooltip("Pebble")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_SHOW_ID => restore_main_window(app),
            TRAY_HIDE_ID => hide_main_window(app),
            TRAY_QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => restore_main_window(tray.app_handle()),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[tauri::command]
fn set_tray_menu_labels(
    app: AppHandle,
    show_label: String,
    hide_label: String,
    quit_label: String,
) -> Result<(), String> {
    let menu =
        build_tray_menu(&app, &show_label, &hide_label, &quit_label).map_err(|e| e.to_string())?;
    let tray = app
        .tray_by_id("main")
        .ok_or_else(|| "tray icon is not initialized".to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())
}

#[tauri::command]
fn take_pending_mailto_urls(state: tauri::State<PendingMailtoUrls>) -> Vec<String> {
    match state.0.lock() {
        Ok(mut pending) => std::mem::take(&mut *pending),
        Err(_) => Vec::new(),
    }
}

#[tauri::command]
fn set_keep_running_in_background(state: tauri::State<AppState>, enabled: bool) {
    state
        .keep_running_in_background
        .store(enabled, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(any(target_os = "linux", test))]
fn should_prefer_wayland(
    wayland_display: Option<&std::ffi::OsStr>,
    gdk_backend: Option<&std::ffi::OsStr>,
    is_appimage: bool,
) -> bool {
    wayland_display.is_some_and(|value| !value.is_empty())
        && (gdk_backend.is_none()
            || (is_appimage && gdk_backend == Some(std::ffi::OsStr::new("x11"))))
}

/// Whether a top-level webview navigation stays on the app's own origin.
///
/// Email content is rendered in a sandbox, but a link or script that manages
/// to navigate the main webview away from the bundled frontend would give a
/// remote page access to the IPC bridge, so every other origin is cancelled.
fn is_allowed_app_navigation(url: &tauri::Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    match url.scheme() {
        "tauri" => host == "localhost",
        "http" | "https" if host == "tauri.localhost" => true,
        "http" if cfg!(debug_assertions) => {
            // Vite dev server (`build.devUrl`).
            matches!(host, "127.0.0.1" | "localhost") && url.port() == Some(1420)
        }
        _ => false,
    }
}

fn navigation_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|webview, url| {
            let allowed = is_allowed_app_navigation(url);
            if !allowed {
                tracing::warn!(
                    webview = webview.label(),
                    scheme = url.scheme(),
                    host = url.host_str().unwrap_or_default(),
                    "blocked webview navigation away from the app origin"
                );
            }
            allowed
        })
        .build()
}

/// Everything in setup that needs the opened database: services, app state
/// and background workers. Runs inline when no upgrade is pending, else on
/// the main thread once the background migration has finished.
fn finish_setup(
    app: &AppHandle,
    store: pebble_store::Store,
    app_data: &std::path::Path,
    startup_start: Instant,
    mut startup_phase: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    // `PRAGMA quick_check` reads the whole file; it runs in the
    // background after startup (see below) instead of delaying the
    // first window.

    let index_path = get_index_path(app_data)?;
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
    log_startup_phase(
        startup_start,
        &mut startup_phase,
        "crypto service initialized",
    );

    let attachments_dir = app_data.join("attachments");
    std::fs::create_dir_all(&attachments_dir)?;
    tracing::info!("Attachments directory: {}", attachments_dir.display());
    log_startup_phase(
        startup_start,
        &mut startup_phase,
        "attachments directory ready",
    );

    let (snooze_stop_tx, snooze_stop_rx) = std::sync::mpsc::channel::<()>();
    app.manage(AppState::new(
        store,
        search,
        crypto,
        snooze_stop_tx,
        attachments_dir,
    ));
    send_controls::setup(app)?;
    log_startup_phase(startup_start, &mut startup_phase, "app state registered");

    // Start snooze watcher on the Tauri async runtime
    let state: tauri::State<AppState> = app.state();
    let store_clone = state.store.clone();
    let app_handle = app.clone();
    tauri::async_runtime::spawn(snooze_watcher::run_snooze_watcher(
        store_clone,
        app_handle.clone(),
        snooze_stop_rx,
    ));

    // Legacy (AAD-less) ciphertext: once a previous run re-encrypted
    // everything, reject it immediately; otherwise migrate in the
    // background (idempotent, compare-and-swap per record).
    if commands::encrypted_store::legacy_ciphertext_migration_done(&state.crypto, &state.store) {
        state.crypto.reject_legacy_ciphertext();
    } else {
        let crypto_for_migration = state.crypto.clone();
        let store_for_migration = state.store.clone();
        tauri::async_runtime::spawn_blocking(move || {
            match commands::encrypted_store::migrate_legacy_ciphertexts(
                &crypto_for_migration,
                &store_for_migration,
            ) {
                Ok(outcome) => tracing::info!(
                    migrated = outcome.migrated,
                    failed = outcome.failed,
                    completed = outcome.completed,
                    "Legacy ciphertext migration finished"
                ),
                Err(error) => tracing::warn!("Legacy ciphertext migration failed: {error}"),
            }
        });
    }

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
        match commands::indexing::recover_pending_search_operations(
            &store_for_reindex,
            &search_for_reindex,
        ) {
            Ok(recovered) if recovered > 0 => tracing::info!(
                "Recovered {recovered} pending search operations from previous session"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                "Search recovery did not commit; pending operations were retained: {error}"
            ),
        }

        // 2. Full rebuild if schema changed or counts diverge.
        let needs_rebuild = if search_needs_reindex {
            tracing::info!("Search index schema changed, rebuild required");
            true
        } else {
            let idx_count = search_for_reindex.doc_count();
            let db_count = store_for_reindex.count_all_messages().unwrap_or(0);
            if idx_count == 0 && db_count > 0 {
                tracing::info!(
                    "Search index empty but DB has {db_count} messages, rebuild required"
                );
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
                    if let Err(error) = store_for_reindex.clear_all_search_pending() {
                        tracing::warn!(
                            "Search rebuild committed but recovery markers could not be cleared: {error}"
                        );
                    }
                    let _ = app_for_reindex.emit("search:reindex-complete", n);
                }
                Err(e) => tracing::error!("Background reindex failed: {e}"),
            }
        }
    });

    // Long-lived workers start immediately: do_reindex is safe to run
    // concurrently with the live indexer (it walks a rowid cursor and
    // serialises each read+index batch with the indexer through
    // `indexing::search_write_guard`), so a schema-upgrade rebuild no
    // longer delays sync.
    let app_for_sync = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        commands::sync_cmd::resume_all_syncs(app_for_sync).await;
    });
    let app_for_pending_ops = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        commands::pending_mail_ops::run_pending_mail_ops_worker(app_for_pending_ops).await;
    });
    let app_for_backup = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        commands::cloud_sync::run_auto_backup_worker(app_for_backup).await;
    });

    // Background integrity check, once the UI and sync are up.
    let store_for_check = state.store.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        match tokio::task::spawn_blocking(move || store_for_check.quick_check()).await {
            Ok(Ok(result)) if result == "ok" => {
                tracing::info!("Database integrity check passed")
            }
            Ok(Ok(result)) => tracing::warn!("Database integrity check warning: {}", result),
            Ok(Err(e)) => tracing::warn!("Database integrity check failed: {}", e),
            Err(e) => tracing::warn!("Database integrity check task failed: {}", e),
        }
    });
    log_startup_phase(
        startup_start,
        &mut startup_phase,
        "background workers scheduled",
    );
    tracing::info!(
        "[startup] tauri setup complete: {}ms total",
        startup_start.elapsed().as_millis()
    );

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Prefer native Wayland when a Wayland compositor is available.
    //
    // Two cases:
    // 1. No GDK_BACKEND set at all — safe to default to wayland.
    // 2. AppImage: the bundled GTK plugin hardcodes GDK_BACKEND=x11 in
    //    its AppRun hook (see tauri#8541).  Detect this via the APPDIR
    //    env var and override the AppImage default so the app actually
    //    runs on Wayland when a compositor is present.
    // We do NOT override any other explicit GDK_BACKEND value (e.g. a
    // user who intentionally set GDK_BACKEND=x11 outside of AppImage).
    #[cfg(target_os = "linux")]
    {
        let wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let gdk_backend = std::env::var_os("GDK_BACKEND");
        if should_prefer_wayland(
            wayland_display.as_deref(),
            gdk_backend.as_deref(),
            std::env::var_os("APPDIR").is_some(),
        ) {
            std::env::set_var("GDK_BACKEND", "wayland,x11");
        }
    }

    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            restore_main_window(app);
        }));
        // The builder's macOS default is MacosLauncher::LaunchAgent, as before.
        let autostart = tauri_plugin_autostart::Builder::new();
        // Linux packages are named "Pebble Mail" (tauri.linux.conf.json); keep the
        // autostart entry at its old name so existing registrations are found.
        #[cfg(target_os = "linux")]
        let autostart = autostart.app_name("Pebble");
        builder = builder.plugin(autostart.build());
    }

    builder
        .plugin(navigation_guard())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::mail_archive::ArchiveJobs::default())
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                WindowEvent::Focused(true) => {
                    commands::notifications::clear_attention_indicator(window.app_handle());
                }
                WindowEvent::CloseRequested { api, .. } => {
                    // Hard guard: hide to tray instead of exiting unless the
                    // frontend explicitly turned background mode off. This also
                    // covers the window shown before the JS close listener
                    // registered, so a close during startup cannot kill the app.
                    let keep_running = window
                        .app_handle()
                        .try_state::<AppState>()
                        .map(|state| {
                            state
                                .keep_running_in_background
                                .load(std::sync::atomic::Ordering::SeqCst)
                        })
                        .unwrap_or(true);
                    if keep_running {
                        api.prevent_close();
                        let _ = window.hide();
                    } else if send_controls::prevent_exit(window.app_handle()) {
                        api.prevent_close();
                    }
                }
                _ => {}
            }
        })
        .setup(|app| {
            let app_data = profile::resolve_app_profile_data_dir(app)?;
            std::fs::create_dir_all(&app_data)?;
            let profile_paths = profile::ProfilePaths::new(app_data.clone()).map_err(|e| {
                format!(
                    "Cannot read or write the profile id in {}: {e}. \
                     If you configured a custom profile directory (PEBBLE_PROFILE_DIR, \
                     pebble-profile.txt, or a portable pebble-profile folder), make sure \
                     Pebble has write access to it.",
                    app_data.display()
                )
            })?;
            app.manage(profile_paths);
            let backgrounds_dir = app_data.join("backgrounds");
            std::fs::create_dir_all(&backgrounds_dir)?;
            app.asset_protocol_scope()
                .allow_directory(&backgrounds_dir, true)?;
            let log_dir = commands::diagnostics::app_log_dir(&app_data);
            std::fs::create_dir_all(&log_dir)?;
            // Rotate daily and keep a bounded number of files so logs cannot
            // grow without limit.
            let file_appender = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix(commands::diagnostics::LOG_FILE_PREFIX)
                .filename_suffix(commands::diagnostics::LOG_FILE_SUFFIX)
                .max_log_files(commands::diagnostics::LOG_MAX_FILES)
                .build(&log_dir)
                .map_err(|e| format!("Failed to initialise log file: {e}"))?;
            let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
            let env_filter =
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    "pebble=info,pebble_store=info,pebble_mail=info,pebble_search=info,pebble_translate=info,pebble_crypto=info,pebble_oauth=info".into()
                });
            let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false);
            if tracing_subscriber::registry()
                .with(env_filter)
                .with(stdout_layer)
                .with(file_layer)
                .try_init()
                .is_ok()
            {
                let _ = LOG_GUARD.set(guard);
            }

            let startup_start = Instant::now();
            let mut startup_phase = startup_start;
            tracing::info!("[startup] tauri setup started");
            if let Err(e) = setup_tray(app) {
                tracing::warn!("Failed to create system tray icon: {e}");
            }
            // Re-register an enabled autostart entry so it launches this binary:
            // it still points at /usr/bin/pebble after the pebble-mail rename.
            #[cfg(target_os = "linux")]
            {
                use tauri_plugin_autostart::ManagerExt;
                let autolaunch = app.autolaunch();
                if autolaunch.is_enabled().unwrap_or(false) {
                    if let Err(e) = autolaunch.enable() {
                        tracing::warn!("Failed to refresh the autostart entry: {e}");
                    }
                }
                // The launcher file is now "Pebble Mail.desktop"; off the startup path.
                std::thread::spawn(linux_migration::migrate_mailto_default);
            }

            // macOS: the overlay title bar, hidden native title and traffic-light
            // position come from tauri.macos.conf.json, so the window is created
            // that way (switching at runtime left the native title showing).
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window
                    .set_background_color(Some(tauri::window::Color(0xf8, 0xf7, 0xf5, 0xff)));
            }
            app.manage(PendingMailtoUrls::default());
            // mailto: links only need the pending queue, so listen before any
            // database work: links opened during a background upgrade are
            // queued and opened once the app mounts.
            let app_for_deep_link = app.handle().clone();
            app.listen(DEEP_LINK_NEW_URL_EVENT, move |event| {
                let urls = mailto_urls_from_deep_link_payload(event.payload());
                record_mailto_urls(&app_for_deep_link, urls);
            });
            record_mailto_urls(
                app.handle(),
                mailto_urls_from_args(&std::env::args().collect::<Vec<_>>()),
            );
            // macOS: replace the default menu (whose predefined Quit bypasses
            // the cancellable exit) before any database work, so Cmd-Q is
            // guarded during a background upgrade too.
            #[cfg(target_os = "macos")]
            setup_application_menu(app.handle())?;

            let db_path = get_db_path(&app_data)?;
            tracing::info!("Database path: {}", db_path.display());
            log_startup_phase(startup_start, &mut startup_phase, "app data paths resolved");

            app.manage(startup::StartupStatus::default());
            match pebble_store::migrations::pending_migration_from(&db_path) {
                Ok(Some(from_version)) => {
                    // Upgrade in the background so the window can show the
                    // splash with a progress bar instead of hanging here.
                    tracing::info!("Database schema v{from_version} needs upgrading; migrating in the background");
                    startup::set_stage(app.handle(), startup::StartupStage::Migrating { done: 0, total: 0 });
                    let handle = app.handle().clone();
                    let app_data = app_data.clone();
                    std::thread::Builder::new()
                        .name("db-migration".into())
                        .spawn(move || {
                            let progress_handle = handle.clone();
                            let opened = pebble_store::Store::open_with_progress(&db_path, &mut |p| {
                                startup::set_stage(
                                    &progress_handle,
                                    startup::StartupStage::Migrating { done: p.done, total: p.total },
                                );
                            });
                            let store = match opened {
                                Ok(store) => store,
                                Err(error) => {
                                    tracing::error!("Database upgrade failed: {error}");
                                    startup::set_stage(&handle, startup::StartupStage::Failed { message: error.to_string() });
                                    return;
                                }
                            };
                            tracing::info!("Database upgrade complete");
                            let main_handle = handle.clone();
                            let scheduled = handle.run_on_main_thread(move || {
                                let phase = Instant::now();
                                match finish_setup(&main_handle, store, &app_data, startup_start, phase) {
                                    Ok(()) => startup::set_stage(&main_handle, startup::StartupStage::Ready),
                                    Err(error) => {
                                        tracing::error!("Startup failed after the database upgrade: {error}");
                                        startup::set_stage(&main_handle, startup::StartupStage::Failed { message: error.to_string() });
                                    }
                                }
                            });
                            if let Err(error) = scheduled {
                                startup::set_stage(&handle, startup::StartupStage::Failed { message: error.to_string() });
                            }
                        })?;
                    Ok(())
                }
                pending => {
                    if let Err(error) = pending {
                        tracing::warn!("Could not read the database version before opening: {error}");
                    }
                    let store = pebble_store::Store::open(&db_path)?;
                    tracing::info!("Database initialized successfully");
                    log_startup_phase(
                        startup_start,
                        &mut startup_phase,
                        "database opened and migrations complete",
                    );
                    finish_setup(app.handle(), store, &app_data, startup_start, startup_phase)?;
                    startup::set_stage(app.handle(), startup::StartupStage::Ready);
                    Ok(())
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_tray_menu_labels,
            take_pending_mailto_urls,
            set_keep_running_in_background,
            profile::get_profile_storage_namespace,
            startup::get_startup_status,
            commands::autostart::get_autostart_enabled,
            commands::autostart::set_autostart_enabled,
            commands::health::health_check,
            commands::health::check_for_update,
            commands::health::open_external_url,
            commands::health::open_default_mail_settings,
            commands::health::sync_titlebar_theme,
            commands::diagnostics::read_app_log,
            commands::appearance::import_background_image,
            commands::appearance::delete_background_image,
            commands::accounts::add_account,
            commands::accounts::get_account_proxy,
            commands::accounts::get_account_proxy_setting,
            commands::accounts::update_account_proxy,
            commands::accounts::update_account_proxy_setting,
            commands::accounts::update_account,
            commands::accounts::list_accounts,
            commands::accounts::delete_account,
            commands::accounts::test_imap_connection,
            commands::accounts::test_pop3_connection,
            commands::accounts::test_account_connection,
            commands::folders::list_folders,
            commands::folder_management::create_mail_folder,
            commands::folder_management::rename_mail_folder,
            commands::folder_management::move_mail_messages,
            commands::folder_management::get_mail_move_context,
            commands::folder_management::get_mail_folder_parents,
            commands::folder_management::rename_mail_folder_namespace,
            commands::mail_archive::mail_archive_choose_files,
            commands::mail_archive::mail_archive_list,
            commands::mail_archive::mail_archive_create,
            commands::mail_archive::mail_archive_folders,
            commands::mail_archive::mail_archive_create_folder,
            commands::mail_archive::mail_archive_messages,
            commands::mail_archive::mail_archive_read,
            commands::mail_archive::mail_archive_update,
            commands::mail_archive::mail_archive_delete,
            commands::mail_archive::mail_archive_cancel,
            commands::mail_archive::mail_archive_import,
            commands::mail_archive::export::mail_archive_export,
            commands::mail_archive::export::mail_archive_save_attachment,
            commands::folders::get_imap_sync_folders,
            commands::folders::update_imap_sync_folders,
            commands::messages::query::list_messages,
            commands::messages::query::list_starred_messages,
            commands::messages::query::get_message,
            commands::messages::query::get_messages_batch,
            commands::messages::rendering::get_rendered_html,
            commands::messages::rendering::get_message_with_html,
            commands::messages::flags::update_message_flags,
            commands::messages::flags::update_thread_read_flags_in_folders,
            commands::messages::rendering::is_trusted_sender,
            commands::messages::lifecycle::archive_message,
            commands::messages::lifecycle::delete_message,
            commands::messages::lifecycle::restore_message,
            commands::messages::lifecycle::empty_trash,
            commands::messages::lifecycle::move_to_folder,
            commands::network::get_global_proxy,
            commands::network::update_global_proxy,
            commands::search::search_messages,
            commands::sync_cmd::start_sync,
            commands::sync_cmd::get_account_sync_range,
            commands::sync_cmd::update_account_sync_range,
            commands::sync_cmd::trigger_sync,
            commands::sync_cmd::stop_sync,
            commands::sync_cmd::set_realtime_preference,
            commands::kanban::move_to_kanban,
            commands::kanban::list_kanban_cards,
            commands::kanban::remove_from_kanban,
            commands::kanban::list_kanban_context_notes,
            commands::kanban::set_kanban_context_note,
            commands::kanban::merge_kanban_context_notes,
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
            send_controls::queue_controlled_send,
            send_controls::list_controlled_sends,
            send_controls::restore_controlled_send,
            send_controls::reschedule_controlled_send,
            send_controls::delete_controlled_send,
            send_controls::resolve_send_control_exit,
            send_controls::send_control_exit_pending,
            commands::compose::stage_compose_attachment,
            commands::compose::cleanup_staged_compose_attachment,
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
            commands::oauth::preview_oauth_identity,
            commands::oauth::apply_oauth_identity,
            commands::oauth::get_oauth_account_proxy,
            commands::oauth::get_oauth_account_proxy_setting,
            commands::oauth::update_oauth_account_proxy,
            commands::oauth::update_oauth_account_proxy_setting,
            commands::attachments::list_attachments,
            commands::attachments::get_attachment_path,
            commands::attachments::download_attachment,
            commands::attachments::preview_attachment,
            commands::attachments::save_all_attachments,
            commands::batch::batch_archive,
            commands::batch::batch_delete,
            commands::batch::batch_mark_read,
            commands::batch::batch_star,
            commands::cloud_sync::test_webdav_connection,
            commands::cloud_sync::backup_to_webdav,
            commands::cloud_sync::export_backup_file,
            commands::cloud_sync::preview_backup_file,
            commands::cloud_sync::import_backup_file,
            commands::cloud_sync::preview_webdav_backup,
            commands::cloud_sync::restore_from_webdav,
            commands::cloud_sync::save_auto_backup_config,
            commands::cloud_sync::load_auto_backup_config,
            commands::cloud_sync::delete_auto_backup_config,
            commands::contacts::list_contacts,
            commands::contacts::get_contact_by_email,
            commands::contacts::save_contact,
            commands::contacts::delete_contact,
            commands::contacts::set_contact_favorite,
            commands::contacts::search_contact_suggestions,
            commands::contacts::suppress_contact_suggestion,
            commands::contacts::import_contacts_vcard,
            commands::contacts::export_contacts_vcard,
            commands::contacts::search_contacts,
            commands::advanced_search::advanced_search,
            commands::sync_cmd::reindex_search,
            commands::notifications::set_notifications_enabled,
            commands::notifications::get_notification_status,
            commands::notifications::show_test_notification,
            commands::notifications::clear_notification_attention,
            commands::notifications::set_notification_labels,
            commands::pending_mail_ops::get_pending_mail_ops_summary,
            commands::pending_mail_ops::list_pending_mail_ops,
            commands::pending_mail_ops::dismiss_failed_pending_mail_ops,
            commands::drafts::save_draft,
            commands::drafts::get_draft,
            commands::pending_mail_ops::retry_failed_send,
            commands::drafts::delete_draft,
            commands::folder_counts::get_folder_unread_counts,
            commands::user_data::list_email_templates,
            commands::user_data::save_email_template,
            commands::user_data::delete_email_template,
            commands::user_data::get_email_signature,
            commands::user_data::set_email_signature,
            commands::user_data::migrate_email_signature_if_absent,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if send_controls::prevent_exit(app) {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod startup_timing_tests {
    use super::{should_prefer_wayland, startup_phase_timing};
    use std::ffi::OsStr;
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

    #[test]
    fn wayland_preference_respects_display_backend_and_appimage_context() {
        let display = Some(OsStr::new("wayland-0"));
        let empty_display = Some(OsStr::new(""));
        let x11 = Some(OsStr::new("x11"));
        let wayland = Some(OsStr::new("wayland"));

        assert!(!should_prefer_wayland(None, None, false));
        assert!(!should_prefer_wayland(empty_display, None, false));
        assert!(should_prefer_wayland(display, None, false));
        assert!(!should_prefer_wayland(display, x11, false));
        assert!(should_prefer_wayland(display, x11, true));
        assert!(!should_prefer_wayland(display, wayland, true));
    }
}

#[cfg(test)]
mod navigation_guard_tests {
    use super::is_allowed_app_navigation;

    fn allowed(url: &str) -> bool {
        is_allowed_app_navigation(&tauri::Url::parse(url).unwrap())
    }

    #[test]
    fn allows_only_the_app_origin() {
        assert!(allowed("tauri://localhost/index.html"));
        assert!(allowed("http://tauri.localhost/"));
        assert!(allowed("https://tauri.localhost/settings"));
        assert!(allowed("http://127.0.0.1:1420/"));

        assert!(!allowed("https://evil.example/"));
        assert!(!allowed("http://tauri.localhost.evil.example/"));
        assert!(!allowed("tauri://evil/"));
        assert!(!allowed("http://127.0.0.1:8080/"));
        assert!(!allowed("file:///etc/passwd"));
        assert!(!allowed("javascript:alert(1)"));
        assert!(!allowed("data:text/html,hi"));
    }
}
