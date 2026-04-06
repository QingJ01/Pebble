use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    // Only allow safe URL schemes to prevent command injection via opener::open / ShellExecuteW
    if !url.starts_with("https://") && !url.starts_with("http://") && !url.starts_with("mailto:") {
        return Err("Only https://, http://, and mailto: URLs are permitted".to_string());
    }
    opener::open(&url).map_err(|e| format!("Failed to open URL: {e}"))
}

#[tauri::command]
pub fn health_check(state: State<'_, AppState>) -> Result<String, String> {
    match state.store.list_accounts() {
        Ok(accounts) => Ok(format!(
            "Pebble is healthy. {} account(s) configured.",
            accounts.len()
        )),
        Err(e) => Err(format!("Health check failed: {}", e)),
    }
}
