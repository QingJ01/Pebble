use pebble_core::{new_id, PebbleError};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub const MAX_BACKGROUND_IMAGE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ImportedBackgroundImage {
    pub path: String,
    pub filename: String,
    pub size: u64,
}

fn detect_background_image_extension(bytes: &[u8]) -> Result<&'static str, PebbleError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok("png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Ok("jpg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok("gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok("webp");
    }
    Err(PebbleError::Validation(
        "Background image must be PNG, JPEG, GIF, or WebP".to_string(),
    ))
}

fn validate_background_image_bytes(bytes: &[u8]) -> Result<&'static str, PebbleError> {
    if bytes.is_empty() {
        return Err(PebbleError::Validation(
            "Background image file is empty".to_string(),
        ));
    }
    if bytes.len() > MAX_BACKGROUND_IMAGE_BYTES {
        return Err(PebbleError::Validation(format!(
            "Background image is too large (max {} MB)",
            MAX_BACKGROUND_IMAGE_BYTES / 1024 / 1024
        )));
    }
    detect_background_image_extension(bytes)
}

fn background_images_dir(app: &AppHandle) -> Result<PathBuf, PebbleError> {
    let app_data =
        crate::profile::require_managed_app_data_dir(app).map_err(PebbleError::Internal)?;
    Ok(app_data.join("backgrounds"))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), PebbleError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| PebbleError::Internal(format!("Failed to create background image: {e}")))?;
    file.write_all(bytes)
        .map_err(|e| PebbleError::Internal(format!("Failed to write background image: {e}")))?;
    file.sync_all()
        .map_err(|e| PebbleError::Internal(format!("Failed to flush background image: {e}")))?;
    Ok(())
}

#[tauri::command]
pub async fn import_background_image(
    app: AppHandle,
    filename: String,
    bytes: Vec<u8>,
) -> std::result::Result<ImportedBackgroundImage, PebbleError> {
    let extension = validate_background_image_bytes(&bytes)?;
    let backgrounds_dir = background_images_dir(&app)?;
    std::fs::create_dir_all(&backgrounds_dir).map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to create background image directory {}: {e}",
            backgrounds_dir.display()
        ))
    })?;

    let stored_filename = format!("background-{}.{}", new_id(), extension);
    let target_path = backgrounds_dir.join(&stored_filename);
    let size = bytes.len() as u64;
    let target_for_write = target_path.clone();
    tokio::task::spawn_blocking(move || write_new_file(&target_for_write, &bytes))
        .await
        .map_err(|e| PebbleError::Internal(format!("Background image write task failed: {e}")))??;

    tracing::info!(
        source_filename = %filename,
        stored_filename = %stored_filename,
        size,
        "Imported background image"
    );

    Ok(ImportedBackgroundImage {
        path: target_path.to_string_lossy().to_string(),
        filename: stored_filename,
        size,
    })
}

#[tauri::command]
pub async fn delete_background_image(
    app: AppHandle,
    path: String,
) -> std::result::Result<(), PebbleError> {
    let backgrounds_dir = background_images_dir(&app)?;
    std::fs::create_dir_all(&backgrounds_dir).map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to create background image directory {}: {e}",
            backgrounds_dir.display()
        ))
    })?;
    let canonical_backgrounds_dir = backgrounds_dir.canonicalize().map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to resolve background image directory {}: {e}",
            backgrounds_dir.display()
        ))
    })?;

    let target = PathBuf::from(&path);
    let canonical_target = match target.canonicalize() {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(PebbleError::Internal(format!(
                "Failed to resolve background image path: {e}"
            )))
        }
    };

    if !canonical_target.starts_with(&canonical_backgrounds_dir) {
        return Err(PebbleError::Validation(
            "Background image path is outside Pebble's background directory".to_string(),
        ));
    }

    if canonical_target.is_file() {
        std::fs::remove_file(&canonical_target).map_err(|e| {
            PebbleError::Internal(format!("Failed to delete background image: {e}"))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_background_image_extension_accepts_common_raster_formats() {
        assert_eq!(
            detect_background_image_extension(b"\x89PNG\r\n\x1a\nrest").unwrap(),
            "png"
        );
        assert_eq!(
            detect_background_image_extension(b"\xff\xd8\xff\xe0rest").unwrap(),
            "jpg"
        );
        assert_eq!(
            detect_background_image_extension(b"GIF89arest").unwrap(),
            "gif"
        );

        let mut webp = b"RIFF\x10\x00\x00\x00WEBPrest".to_vec();
        webp.resize(24, b'0');
        assert_eq!(detect_background_image_extension(&webp).unwrap(), "webp");
    }

    #[test]
    fn validate_background_image_bytes_rejects_non_images_and_large_files() {
        assert!(validate_background_image_bytes(b"<svg></svg>").is_err());

        let oversized = vec![0u8; MAX_BACKGROUND_IMAGE_BYTES + 1];
        assert!(validate_background_image_bytes(&oversized).is_err());
    }
}
