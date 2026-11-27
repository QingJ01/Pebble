//! Startup status for the boot splash.
//!
//! A database that needs upgrading is migrated on a background thread after
//! the window is up, so the splash can show progress instead of the app
//! appearing frozen. The frontend polls [`get_startup_status`] and mounts the
//! app once the stage is `ready`.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "stage", rename_all = "camelCase")]
pub enum StartupStage {
    /// Opening the database and services (no upgrade pending).
    Starting,
    /// Upgrading the database: `done` of `total` schema versions applied.
    Migrating {
        done: u32,
        total: u32,
    },
    Ready,
    Failed {
        message: String,
    },
}

#[derive(Debug)]
pub struct StartupStatus(Mutex<StartupStage>);

impl Default for StartupStatus {
    fn default() -> Self {
        Self(Mutex::new(StartupStage::Starting))
    }
}

pub const STARTUP_STATUS_EVENT: &str = "startup:status";

pub fn set_stage<R: Runtime>(app: &AppHandle<R>, stage: StartupStage) {
    if let Some(status) = app.try_state::<StartupStatus>() {
        if let Ok(mut current) = status.0.lock() {
            *current = stage.clone();
        }
    }
    let _ = app.emit(STARTUP_STATUS_EVENT, stage);
}

/// Whether startup stopped with an error (e.g. a failed database upgrade).
/// Nothing after the database is set up then, so exit must not wait on it.
pub fn has_failed<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<StartupStatus>()
        .and_then(|status| {
            status
                .0
                .lock()
                .ok()
                .map(|stage| matches!(*stage, StartupStage::Failed { .. }))
        })
        .unwrap_or(false)
}

#[tauri::command]
pub fn get_startup_status<R: Runtime>(app: AppHandle<R>) -> StartupStage {
    app.try_state::<StartupStatus>()
        .and_then(|status| status.0.lock().ok().map(|stage| stage.clone()))
        .unwrap_or(StartupStage::Starting)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_serialize_for_the_frontend() {
        assert_eq!(
            serde_json::to_value(StartupStage::Migrating { done: 1, total: 3 }).unwrap(),
            serde_json::json!({ "stage": "migrating", "done": 1, "total": 3 })
        );
        assert_eq!(
            serde_json::to_value(StartupStage::Ready).unwrap(),
            serde_json::json!({ "stage": "ready" })
        );
        assert_eq!(
            serde_json::to_value(StartupStage::Failed {
                message: "x".into()
            })
            .unwrap(),
            serde_json::json!({ "stage": "failed", "message": "x" })
        );
    }
}
