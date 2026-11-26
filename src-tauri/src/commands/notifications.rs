use crate::state::AppState;
use pebble_core::PebbleError;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::sync::RwLock;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Runtime, State};
#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;
use tracing::warn;

const TRAY_DEFAULT_TOOLTIP: &str = "Pebble";

/// User-facing notification and tray strings, localized by the frontend (like
/// the tray menu labels) and pushed with `set_notification_labels`. Until the
/// frontend reports in, English defaults are used.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotificationLabels {
    /// Title of a new-mail notification.
    pub new_mail_title: String,
    /// Body used when a message has neither sender nor subject.
    pub new_message_fallback: String,
    /// Summary body for a burst of mail; `{n}` = count, `{latest}` = newest message.
    pub new_mail_summary: String,
    /// Label of the "open" action button.
    pub open_action: String,
    /// Title of a snooze-reminder notification.
    pub snoozed_title: String,
    pub test_title: String,
    pub test_body: String,
    /// Tray tooltip while unread new mail is waiting.
    pub tray_attention_tooltip: String,
}

impl Default for NotificationLabels {
    fn default() -> Self {
        Self {
            new_mail_title: "Pebble - New mail".to_string(),
            new_message_fallback: "New message".to_string(),
            new_mail_summary: "{n} new messages — latest: {latest}".to_string(),
            open_action: "Open".to_string(),
            snoozed_title: "Pebble - Snoozed message".to_string(),
            test_title: "Pebble - Test notification".to_string(),
            test_body: "Desktop notifications are working.".to_string(),
            tray_attention_tooltip: "Pebble - New mail".to_string(),
        }
    }
}

impl NotificationLabels {
    /// Fill the burst-summary template.
    pub fn summary(&self, count: usize, latest: &str) -> String {
        self.new_mail_summary
            .replace("{n}", &count.to_string())
            .replace("{latest}", latest)
    }

    /// Replace blank fields with the English defaults so a partial update can
    /// never produce an empty notification title.
    fn or_defaults(mut self) -> Self {
        let defaults = Self::default();
        let fields = [
            (&mut self.new_mail_title, defaults.new_mail_title),
            (
                &mut self.new_message_fallback,
                defaults.new_message_fallback,
            ),
            (&mut self.new_mail_summary, defaults.new_mail_summary),
            (&mut self.open_action, defaults.open_action),
            (&mut self.snoozed_title, defaults.snoozed_title),
            (&mut self.test_title, defaults.test_title),
            (&mut self.test_body, defaults.test_body),
            (
                &mut self.tray_attention_tooltip,
                defaults.tray_attention_tooltip,
            ),
        ];
        for (field, default) in fields {
            if field.trim().is_empty() {
                *field = default;
            }
        }
        self
    }
}

static NOTIFICATION_LABELS: RwLock<Option<NotificationLabels>> = RwLock::new(None);

/// Current localized notification strings (English until the frontend sets them).
pub fn notification_labels() -> NotificationLabels {
    NOTIFICATION_LABELS
        .read()
        .ok()
        .and_then(|labels| labels.clone())
        .unwrap_or_default()
}

fn store_notification_labels(labels: NotificationLabels) {
    if let Ok(mut slot) = NOTIFICATION_LABELS.write() {
        *slot = Some(labels.or_defaults());
    }
}

/// True while the main window is visible and focused: the user is already
/// looking at Pebble, so new-mail toasts and the tray badge are redundant.
pub fn main_window_has_focus<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window("main")
        .map(|window| window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
        tauri_winrt_notification::Toast::new(&windows_notification_app_id(app))
            .title(title)
            .text1(body)
            .duration(tauri_winrt_notification::Duration::Short)
            .show()
            .map_err(|e| format!("{e:?}"))
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
    !main_window_has_focus(app)
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
            notification_labels().tray_attention_tooltip
        } else {
            TRAY_DEFAULT_TOOLTIP.to_string()
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

    let labels = notification_labels();
    show_desktop_notification(&app, &labels.test_title, &labels.test_body)
        .map_err(PebbleError::Internal)
}

/// Localized notification/tray strings from the frontend (see `useTrayI18n`).
#[tauri::command]
pub async fn set_notification_labels(
    app: AppHandle,
    state: State<'_, AppState>,
    labels: NotificationLabels,
) -> std::result::Result<(), PebbleError> {
    store_notification_labels(labels);
    // Refresh a tooltip that is already showing the attention text.
    if state.notification_attention_active.load(Ordering::SeqCst) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_tooltip(Some(notification_labels().tray_attention_tooltip));
        }
    }
    Ok(())
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
    fn notification_labels_fill_summary_template() {
        let labels = NotificationLabels {
            new_mail_summary: "{n} 封新邮件，最新：{latest}".to_string(),
            ..NotificationLabels::default()
        };
        assert_eq!(labels.summary(5, "Ann: Hi"), "5 封新邮件，最新：Ann: Hi");
        assert_eq!(
            NotificationLabels::default().summary(4, "x"),
            "4 new messages — latest: x"
        );
    }

    #[test]
    fn notification_labels_blank_fields_fall_back_to_defaults() {
        let labels = NotificationLabels {
            new_mail_title: "  ".to_string(),
            snoozed_title: "稍后提醒".to_string(),
            ..NotificationLabels::default()
        }
        .or_defaults();
        assert_eq!(labels.new_mail_title, "Pebble - New mail");
        assert_eq!(labels.snoozed_title, "稍后提醒");
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
