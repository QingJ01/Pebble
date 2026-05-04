use crate::state::AppState;
use pebble_core::PebbleError;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Runtime, State, UserAttentionType};
#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;
use tracing::warn;

const TRAY_DEFAULT_TOOLTIP: &str = "Pebble";
const TRAY_ATTENTION_TOOLTIP: &str = "Pebble - New mail";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NotificationStatus {
    pub enabled: bool,
    pub attention_active: bool,
    pub platform: String,
    pub app_id: Option<String>,
}

pub fn notification_status_payload(
    enabled: bool,
    attention_active: bool,
    platform: &str,
    app_id: Option<String>,
) -> NotificationStatus {
    NotificationStatus {
        enabled,
        attention_active,
        platform: platform.to_string(),
        app_id,
    }
}

fn notification_platform() -> &'static str {
    #[cfg(windows)]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(all(not(windows), not(target_os = "macos"), target_os = "linux"))]
    {
        "linux"
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        "desktop"
    }
}

#[cfg(windows)]
fn is_tauri_target_build_dir(curr_dir: &str) -> bool {
    use std::path::MAIN_SEPARATOR as SEP;

    curr_dir.ends_with(format!("{SEP}target{SEP}debug").as_str())
        || curr_dir.ends_with(format!("{SEP}target{SEP}release").as_str())
}

#[cfg(windows)]
pub fn windows_notification_app_id<R: Runtime>(app: &AppHandle<R>) -> String {
    let is_dev_build_dir = tauri::utils::platform::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.display().to_string()))
        .is_some_and(|curr_dir| is_tauri_target_build_dir(&curr_dir));

    if is_dev_build_dir {
        tauri_winrt_notification::Toast::POWERSHELL_APP_ID.to_string()
    } else {
        app.config().identifier.clone()
    }
}

fn notification_app_id<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    #[cfg(windows)]
    {
        Some(windows_notification_app_id(app))
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

#[cfg(windows)]
fn register_windows_app_user_model_id<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    use windows_registry::{CURRENT_USER, HSTRING};

    let app_id = windows_notification_app_id(app);
    if app_id == tauri_winrt_notification::Toast::POWERSHELL_APP_ID {
        return Ok(());
    }

    let key = CURRENT_USER
        .create(format!(r"SOFTWARE\Classes\AppUserModelId\{app_id}"))
        .map_err(|e| e.to_string())?;
    key.set_string("DisplayName", &app.package_info().name)
        .map_err(|e| e.to_string())?;
    key.set_string("IconBackgroundColor", "0")
        .map_err(|e| e.to_string())?;

    if let Ok(exe) = tauri::utils::platform::current_exe() {
        let icon_uri = HSTRING::from(exe.to_string_lossy().as_ref());
        key.set_hstring("IconUri", &icon_uri)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(windows)]
pub fn ensure_notification_environment<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    register_windows_app_user_model_id(app)
}

#[cfg(not(windows))]
pub fn ensure_notification_environment<R: Runtime>(_app: &AppHandle<R>) -> Result<(), String> {
    Ok(())
}

pub fn show_desktop_notification<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    body: &str,
) -> Result<(), String> {
    ensure_notification_environment(app)?;

    #[cfg(windows)]
    {
        return tauri_winrt_notification::Toast::new(&windows_notification_app_id(app))
            .title(title)
            .text1(body)
            .duration(tauri_winrt_notification::Duration::Short)
            .show()
            .map_err(|e| format!("{e:?}"));
    }

    #[cfg(not(windows))]
    {
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| e.to_string())
    }
}

fn draw_dot(
    rgba: &mut [u8],
    width: usize,
    height: usize,
    center_x: f32,
    center_y: f32,
    radius: f32,
    border: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 + 0.5 - center_x;
            let dy = y as f32 + 0.5 - center_y;
            let distance = (dx * dx + dy * dy).sqrt();
            let offset = (y * width + x) * 4;

            if distance <= radius {
                rgba[offset] = 239;
                rgba[offset + 1] = 68;
                rgba[offset + 2] = 68;
                rgba[offset + 3] = 255;
            } else if distance <= radius + border {
                rgba[offset] = 255;
                rgba[offset + 1] = 255;
                rgba[offset + 2] = 255;
                rgba[offset + 3] = 255;
            }
        }
    }
}

pub fn attention_overlay_icon() -> Image<'static> {
    let width = 16usize;
    let height = 16usize;
    let mut rgba = vec![0u8; width * height * 4];
    draw_dot(&mut rgba, width, height, 8.0, 8.0, 4.8, 1.2);
    Image::new_owned(rgba, width as u32, height as u32)
}

pub fn tray_attention_icon(base: &Image<'_>) -> Image<'static> {
    let width = base.width() as usize;
    let height = base.height() as usize;
    let mut rgba = base.rgba().to_vec();
    let radius = ((width.min(height) as f32) * 0.18).clamp(4.0, 10.0);
    let center_x = width as f32 - radius - 2.0;
    let center_y = radius + 2.0;

    draw_dot(&mut rgba, width, height, center_x, center_y, radius, 1.5);
    Image::new_owned(rgba, base.width(), base.height())
}

fn should_mark_attention<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window("main")
        .map(|window| {
            let visible = window.is_visible().unwrap_or(false);
            let focused = window.is_focused().unwrap_or(false);
            !(visible && focused)
        })
        .unwrap_or(true)
}

pub fn mark_attention_indicator<R: Runtime>(app: &AppHandle<R>) {
    if should_mark_attention(app) {
        set_attention_indicator(app, true);
    }
}

pub fn clear_attention_indicator<R: Runtime>(app: &AppHandle<R>) {
    set_attention_indicator(app, false);
}

fn set_attention_indicator<R: Runtime>(app: &AppHandle<R>, active: bool) {
    if let Some(state) = app.try_state::<AppState>() {
        let previous = state
            .notification_attention_active
            .swap(active, Ordering::SeqCst);
        if previous == active {
            return;
        }
    }

    if let Some(window) = app.get_webview_window("main") {
        #[cfg(windows)]
        {
            let _ = window.set_overlay_icon(if active {
                Some(attention_overlay_icon())
            } else {
                None
            });
        }
        #[cfg(not(windows))]
        {
            let _ = window.set_badge_count(if active { Some(1) } else { None });
        }

        let _ = window.request_user_attention(if active {
            Some(UserAttentionType::Informational)
        } else {
            None
        });
    }

    if let Some(tray) = app.tray_by_id("main") {
        if let Some(default_icon) = app.default_window_icon() {
            let icon = if active {
                tray_attention_icon(default_icon)
            } else {
                default_icon.clone().to_owned()
            };
            if let Err(e) = tray.set_icon(Some(icon)) {
                warn!("Failed to update tray notification icon: {e}");
            }
        }

        let tooltip = if active {
            TRAY_ATTENTION_TOOLTIP
        } else {
            TRAY_DEFAULT_TOOLTIP
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

#[tauri::command]
pub async fn set_notifications_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> std::result::Result<(), PebbleError> {
    state.notifications_enabled.store(enabled, Ordering::SeqCst);
    if !enabled {
        clear_attention_indicator(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_notification_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> std::result::Result<NotificationStatus, PebbleError> {
    Ok(notification_status_payload(
        state.notifications_enabled.load(Ordering::SeqCst),
        state.notification_attention_active.load(Ordering::SeqCst),
        notification_platform(),
        notification_app_id(&app),
    ))
}

#[tauri::command]
pub async fn show_test_notification(
    app: AppHandle,
    state: State<'_, AppState>,
) -> std::result::Result<(), PebbleError> {
    if !state.notifications_enabled.load(Ordering::SeqCst) {
        return Err(PebbleError::Validation(
            "Desktop notifications are disabled".to_string(),
        ));
    }

    show_desktop_notification(
        &app,
        "Pebble - Test Notification",
        "Desktop notifications are working.",
    )
    .map_err(PebbleError::Internal)
}

#[tauri::command]
pub async fn clear_notification_attention(app: AppHandle) -> std::result::Result<(), PebbleError> {
    clear_attention_indicator(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::image::Image;

    #[test]
    fn notification_status_payload_reports_gate_attention_and_app_id() {
        let status = notification_status_payload(
            true,
            false,
            "windows",
            Some("com.qingj01.pebble".to_string()),
        );

        assert!(status.enabled);
        assert!(!status.attention_active);
        assert_eq!(status.platform, "windows");
        assert_eq!(status.app_id.as_deref(), Some("com.qingj01.pebble"));
    }

    #[test]
    fn attention_overlay_icon_is_transparent_with_a_red_dot() {
        let icon = attention_overlay_icon();

        assert_eq!(icon.width(), 16);
        assert_eq!(icon.height(), 16);
        assert!(icon
            .rgba()
            .chunks_exact(4)
            .any(|px| { px[0] > 220 && px[1] < 80 && px[2] < 80 && px[3] > 220 }));
        assert!(icon.rgba().chunks_exact(4).any(|px| px[3] == 0));
    }

    #[test]
    fn tray_attention_icon_preserves_base_size_and_marks_top_right() {
        let base = Image::new_owned(vec![24; 32 * 32 * 4], 32, 32);
        let icon = tray_attention_icon(&base);

        assert_eq!(icon.width(), 32);
        assert_eq!(icon.height(), 32);

        let top_right_dot = ((5 * 32 + 26) * 4) as usize;
        assert!(icon.rgba()[top_right_dot] > 220);
        assert!(icon.rgba()[top_right_dot + 1] < 80);
        assert!(icon.rgba()[top_right_dot + 2] < 80);
        assert_eq!(icon.rgba()[top_right_dot + 3], 255);

        let bottom_left = ((30 * 32 + 1) * 4) as usize;
        assert_eq!(
            &icon.rgba()[bottom_left..bottom_left + 4],
            &[24, 24, 24, 24]
        );
    }
}
