use base64::Engine;
use image::{ImageEncoder, ImageFormat};
use pebble_core::{Attachment, PebbleError};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Component, Path};

pub(super) const MAX_PREVIEW_BYTES: u64 = 20 * 1024 * 1024;
static PREVIEW_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Serialize)]
pub struct AttachmentPreview {
    pub mime_type: String,
    pub base64: String,
}

fn invalid(message: &str) -> PebbleError {
    PebbleError::Validation(message.to_string())
}

pub(super) fn open_attachment_source(
    root: &Path,
    attachment: &Attachment,
) -> Result<std::fs::File, PebbleError> {
    let path = attachment
        .local_path
        .as_deref()
        .map(Path::new)
        .ok_or_else(|| {
            invalid("Attachment is not available locally. Sync the message and retry.")
        })?;
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid("Invalid attachment storage path"));
    }
    let root = root
        .canonicalize()
        .map_err(|_| invalid("Attachment storage is unavailable"))?;
    let canonical = path
        .canonicalize()
        .map_err(|_| invalid("Attachment file is missing. Sync the message and retry."))?;
    if !canonical.starts_with(&root) || canonical == root {
        return Err(invalid("Attachment is outside attachment storage"));
    }
    // Reject links in the stored path, including links to other files inside the root.
    let mut ancestor = Some(path);
    while let Some(part) = ancestor {
        if std::fs::symlink_metadata(part)
            .map_err(|_| invalid("Attachment file is unavailable"))?
            .file_type()
            .is_symlink()
        {
            return Err(invalid("Attachment storage path contains a symbolic link"));
        }
        if part.canonicalize().ok().as_deref() == Some(root.as_path()) {
            break;
        }
        ancestor = part.parent();
    }
    if !std::fs::metadata(&canonical)
        .map_err(|_| invalid("Attachment file is unavailable"))?
        .is_file()
    {
        return Err(invalid("Attachment is not a regular file"));
    }
    let file = std::fs::File::open(canonical)
        .map_err(|_| invalid("Attachment file could not be opened"))?;
    if !file
        .metadata()
        .map_err(|_| invalid("Attachment metadata is unavailable"))?
        .is_file()
    {
        return Err(invalid("Attachment is not a regular file"));
    }
    Ok(file)
}

pub(super) async fn preview(
    root: std::path::PathBuf,
    attachment: Attachment,
) -> Result<AttachmentPreview, PebbleError> {
    let slot = PREVIEW_SLOTS
        .try_acquire()
        .map_err(|_| invalid("Another preview is loading. Please retry."))?;
    let result = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        read_preview(&root, &attachment)
    })
    .await
    .map_err(|_| invalid("Preview could not be loaded. Download the attachment instead."))?;
    result
}

pub(super) fn read_preview(
    root: &Path,
    attachment: &Attachment,
) -> Result<AttachmentPreview, PebbleError> {
    let file = open_attachment_source(root, attachment)?;
    let length = file
        .metadata()
        .map_err(|_| invalid("Attachment metadata is unavailable"))?
        .len();
    if length == 0 || length > MAX_PREVIEW_BYTES {
        return Err(invalid(
            "Preview supports files up to 20 MB. Download this attachment instead.",
        ));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("Attachment could not be read. Download it instead."))?;
    if bytes.len() as u64 > MAX_PREVIEW_BYTES {
        return Err(invalid("Attachment exceeds the 20 MB preview limit"));
    }
    let declared = attachment
        .mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let generic = declared.is_empty() || declared == "application/octet-stream";
    let (mime, safe_bytes) = if bytes.starts_with(b"%PDF-1.") || bytes.starts_with(b"%PDF-2.") {
        if !generic && declared != "application/pdf" {
            return Err(invalid("Attachment content does not match its media type"));
        }
        if !bytes[bytes.len().saturating_sub(1024)..]
            .windows(5)
            .any(|value| value == b"%%EOF")
        {
            return Err(invalid(
                "PDF is incomplete or corrupt. Download it instead.",
            ));
        }
        ("application/pdf", bytes)
    } else {
        let format = image::guess_format(&bytes).map_err(|_| {
            invalid("Only PNG, JPEG, GIF, WebP images and PDF files can be previewed")
        })?;
        let detected = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::WebP => "image/webp",
            _ => {
                return Err(invalid(
                    "This image format cannot be previewed. Download it instead.",
                ))
            }
        };
        if !generic && declared != detected {
            return Err(invalid("Attachment content does not match its media type"));
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|_| {
            invalid(
                "Image is corrupt or exceeds the 4096 × 4096 preview limit. Download it instead.",
            )
        })?;
        let rgba = decoded.into_rgba8();
        let mut output = LimitedBuffer(Vec::new());
        image::codecs::png::PngEncoder::new(&mut output)
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|_| invalid("Image exceeds preview limits. Download it instead."))?;
        // Re-encoding strips active/polyglot payloads, metadata and animation.
        ("image/png", output.0)
    };
    Ok(AttachmentPreview {
        mime_type: mime.to_string(),
        base64: base64::engine::general_purpose::STANDARD.encode(safe_bytes),
    })
}

struct LimitedBuffer(Vec<u8>);

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_PREVIEW_BYTES as usize {
            return Err(std::io::Error::other("Preview output exceeds limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        base: std::path::PathBuf,
        root: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let base =
                std::env::temp_dir().join(format!("pebble-preview-{}", pebble_core::new_id()));
            let root = base.join("attachments");
            std::fs::create_dir_all(&root).unwrap();
            Self { base, root }
        }
        fn attachment(&self, bytes: &[u8], mime: &str) -> Attachment {
            let path = self.root.join(pebble_core::new_id());
            std::fs::write(&path, bytes).unwrap();
            Attachment {
                id: pebble_core::new_id(),
                message_id: "message".into(),
                filename: "untrusted.html".into(),
                mime_type: mime.into(),
                size: 0,
                local_path: Some(path.to_string_lossy().into_owned()),
                content_id: None,
                is_inline: false,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn image_bytes(format: ImageFormat) -> Vec<u8> {
        let image = image::DynamicImage::new_rgb8(2, 2);
        let mut output = std::io::Cursor::new(Vec::new());
        image.write_to(&mut output, format).unwrap();
        output.into_inner()
    }

    #[test]
    fn previews_allowlisted_images_by_content_and_reencodes_to_static_png() {
        let fixture = Fixture::new();
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::Gif, "image/gif"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let mut bytes = image_bytes(format);
            bytes.extend_from_slice(b"<script>untrusted polyglot tail</script>");
            let attachment = fixture.attachment(&bytes, mime);
            let result = read_preview(&fixture.root, &attachment).unwrap();
            assert_eq!(result.mime_type, "image/png");
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(result.base64)
                .unwrap();
            assert_eq!(image::guess_format(&decoded).unwrap(), ImageFormat::Png);
            assert!(!decoded.windows(8).any(|value| value == b"<script>"));
        }
        let attachment =
            fixture.attachment(&image_bytes(ImageFormat::Png), "application/octet-stream");
        assert!(
            read_preview(&fixture.root, &attachment).is_ok(),
            "staged images need content detection, not extension/MIME trust"
        );
    }

    #[test]
    fn rejects_active_mime_spoofing_and_corrupt_content() {
        let fixture = Fixture::new();
        for (bytes, mime) in [
            (b"<svg onload='evil()'/>".as_slice(), "image/svg+xml"),
            (b"<html><script>evil()</script>".as_slice(), "image/png"),
            (b"\x89PNG\r\n\x1a\ncorrupt".as_slice(), "image/png"),
            (b"%PDF-1.7\nno eof".as_slice(), "application/pdf"),
        ] {
            assert!(read_preview(&fixture.root, &fixture.attachment(bytes, mime)).is_err());
        }
        for mime in [
            "text/html",
            "image/svg+xml",
            "application/javascript",
            "image/jpeg",
        ] {
            assert!(read_preview(
                &fixture.root,
                &fixture.attachment(&image_bytes(ImageFormat::Png), mime)
            )
            .is_err());
        }
        assert!(read_preview(
            &fixture.root,
            &fixture.attachment(b"%PDF-1.7\n%%EOF", "image/png")
        )
        .is_err());
    }

    #[test]
    fn caps_actual_size_and_dimensions_ignoring_untrusted_record_size() {
        let fixture = Fixture::new();
        let attachment = fixture.attachment(b"", "image/png");
        assert!(read_preview(&fixture.root, &attachment).is_err());
        std::fs::OpenOptions::new()
            .write(true)
            .open(attachment.local_path.unwrap())
            .unwrap()
            .set_len(MAX_PREVIEW_BYTES + 1)
            .unwrap();
        let mut huge = fixture.attachment(b"x", "application/pdf");
        std::fs::OpenOptions::new()
            .write(true)
            .open(huge.local_path.as_ref().unwrap())
            .unwrap()
            .set_len(MAX_PREVIEW_BYTES + 1)
            .unwrap();
        huge.size = 1;
        assert!(read_preview(&fixture.root, &huge).is_err());
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4097, 1)
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        assert!(read_preview(
            &fixture.root,
            &fixture.attachment(&output.into_inner(), "image/png")
        )
        .is_err());
        let mut output = LimitedBuffer(vec![0; MAX_PREVIEW_BYTES as usize]);
        assert!(output.write_all(b"x").is_err());
    }

    #[test]
    fn accepts_generic_pdf_with_bounded_framing_without_executing_it() {
        let fixture = Fixture::new();
        let bytes = b"%PDF-1.7\n1 0 obj <</S /JavaScript /JS (evil())>> endobj\n%%EOF";
        let result = read_preview(
            &fixture.root,
            &fixture.attachment(bytes, "application/octet-stream"),
        )
        .unwrap();
        assert_eq!(result.mime_type, "application/pdf");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(result.base64)
                .unwrap(),
            bytes
        );
        // Full parsing and rejection of corrupt/encrypted PDFs belongs to the canvas-only renderer.
    }

    #[test]
    fn source_gate_rejects_missing_relative_directory_traversal_and_outside_paths() {
        let fixture = Fixture::new();
        let mut attachment = fixture.attachment(b"x", "image/png");
        for path in [
            None,
            Some("relative.png".into()),
            Some(fixture.root.to_string_lossy().into_owned()),
            Some(fixture.root.join("missing").to_string_lossy().into_owned()),
            Some(
                fixture
                    .root
                    .join("../attachments/file.png")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ] {
            attachment.local_path = path;
            assert!(open_attachment_source(&fixture.root, &attachment).is_err());
        }
        let outside = fixture.base.join("outside.png");
        std::fs::write(&outside, b"x").unwrap();
        attachment.local_path = Some(outside.to_string_lossy().into_owned());
        assert!(open_attachment_source(&fixture.root, &attachment).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn source_gate_rejects_external_and_internal_symlinks() {
        let fixture = Fixture::new();
        let outside = fixture.base.join("outside");
        std::fs::write(&outside, b"x").unwrap();
        for target in [outside, fixture.root.join("real")] {
            std::fs::write(&target, b"x").unwrap();
            let link = fixture.root.join(pebble_core::new_id());
            std::os::unix::fs::symlink(target, &link).unwrap();
            let mut attachment = fixture.attachment(b"x", "image/png");
            attachment.local_path = Some(link.to_string_lossy().into_owned());
            assert!(open_attachment_source(&fixture.root, &attachment).is_err());
        }
        let real_dir = fixture.root.join("real-dir");
        std::fs::create_dir(&real_dir).unwrap();
        std::fs::write(real_dir.join("file"), b"x").unwrap();
        let linked_dir = fixture.root.join("linked-dir");
        std::os::unix::fs::symlink(real_dir, &linked_dir).unwrap();
        let mut attachment = fixture.attachment(b"x", "image/png");
        attachment.local_path = Some(linked_dir.join("file").to_string_lossy().into_owned());
        assert!(open_attachment_source(&fixture.root, &attachment).is_err());
    }
}
