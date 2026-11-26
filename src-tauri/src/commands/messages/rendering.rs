use crate::state::AppState;
use base64::{engine::general_purpose::STANDARD, Engine};
use pebble_core::{Message, PebbleError, PrivacyMode, RenderedHtml, TrustType};
use pebble_privacy::PrivacyGuard;
use pebble_store::Store;
use std::{collections::HashMap, path::Path};
use tauri::State;

const MAX_INLINE_EXPANSION_BYTES: usize = 20 * 1024 * 1024;

/// Raster image types an inline (cid:) image may be shown as. SVG is never
/// inlined (it can carry script and external references).
const INLINE_IMAGE_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

/// The declared MIME type, with parameters dropped and common non-standard
/// aliases (`image/jpg`, `image/x-png`…) mapped to their standard names.
fn normalize_image_mime(declared: &str) -> String {
    let base = declared
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match base.as_str() {
        "image/jpg" | "image/pjpeg" => "image/jpeg".to_string(),
        "image/x-png" => "image/png".to_string(),
        "image/x-bmp" | "image/x-ms-bmp" => "image/bmp".to_string(),
        _ => base,
    }
}

/// Raster format from the file's leading bytes, for parts sent as
/// `application/octet-stream` or another generic type.
fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// Key for matching a `cid:` reference to a Content-ID: angle brackets and
/// whitespace dropped, %XX escapes decoded (cid URLs may be URL-encoded, RFC
/// 2392) and case folded, since mailers disagree on both.
fn normalize_cid(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches(['<', '>']).trim();
    let bytes = trimmed.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                decoded.push(value);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).to_lowercase()
}

fn resolve_inline_images(
    store: &Store,
    root: &Path,
    message_id: &str,
    rendered: &mut RenderedHtml,
) -> Result<(), PebbleError> {
    if !rendered.html.to_ascii_lowercase().contains("cid:") {
        return Ok(());
    }
    let Ok(root) = root.canonicalize() else {
        return Ok(());
    };
    let mut images = HashMap::new();
    let mut total_size = 0;
    for attachment in store.list_attachments_by_message(message_id)? {
        let Some(cid) = attachment.content_id else {
            continue;
        };
        let declared = normalize_image_mime(&attachment.mime_type);
        // Declared SVG or another non-raster image type: never inlined. Generic
        // types (octet-stream…) are decided from the bytes below.
        if declared.starts_with("image/") && !INLINE_IMAGE_TYPES.contains(&declared.as_str()) {
            continue;
        }
        let Some(path) = attachment.local_path else {
            continue;
        };
        let Ok(path) = Path::new(&path).canonicalize() else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        if metadata.len() > 10 * 1024 * 1024 || total_size + metadata.len() > 20 * 1024 * 1024 {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mime = if INLINE_IMAGE_TYPES.contains(&declared.as_str()) {
            declared.as_str()
        } else if let Some(sniffed) = sniff_image_mime(&bytes) {
            sniffed
        } else {
            continue;
        };
        total_size += bytes.len() as u64;
        images.insert(
            normalize_cid(&cid),
            format!("data:{mime};base64,{}", STANDARD.encode(&bytes)),
        );
    }
    let mut expanded_bytes = 0;
    rendered.html = lol_html::rewrite_str(
        &rendered.html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img[src]", |element| {
                if let Some(src) = element.get_attribute("src") {
                    if src
                        .get(..4)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("cid:"))
                    {
                        let cid = normalize_cid(&src[4..]);
                        if let Some(data) = images.get(&cid).filter(|data| {
                            data.len() <= MAX_INLINE_EXPANSION_BYTES - expanded_bytes
                        }) {
                            element.set_attribute("src", data)?;
                            expanded_bytes += data.len();
                        } else {
                            element.remove_attribute("src");
                        }
                    }
                }
                Ok(())
            })],
            ..Default::default()
        },
    )
    .map_err(|error| PebbleError::Internal(format!("Inline image rendering failed: {error}")))?;
    Ok(())
}

#[tauri::command]
pub async fn get_rendered_html(
    state: State<'_, AppState>,
    message_id: String,
    privacy_mode: PrivacyMode,
) -> std::result::Result<RenderedHtml, PebbleError> {
    let store = state.store.clone();
    let attachments_dir = state.attachments_dir.clone();
    tokio::task::spawn_blocking(move || {
        let message = store
            .get_message(&message_id)?
            .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;

        let effective_mode = resolve_privacy_mode(&store, &message, privacy_mode)?;
        let guard = PrivacyGuard::new();
        let mut rendered =
            guard.render_message_html(&message.body_html_raw, &message.body_text, &effective_mode);
        resolve_inline_images(&store, &attachments_dir, &message_id, &mut rendered)?;
        Ok(rendered)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn get_message_with_html(
    state: State<'_, AppState>,
    message_id: String,
    privacy_mode: PrivacyMode,
) -> std::result::Result<Option<(Message, RenderedHtml)>, PebbleError> {
    let store = state.store.clone();
    let attachments_dir = state.attachments_dir.clone();
    tokio::task::spawn_blocking(move || {
        let message = match store.get_message(&message_id)? {
            Some(m) => m,
            None => return Ok(None),
        };

        let effective_mode = resolve_privacy_mode(&store, &message, privacy_mode)?;
        let guard = PrivacyGuard::new();
        let mut rendered =
            guard.render_message_html(&message.body_html_raw, &message.body_text, &effective_mode);
        resolve_inline_images(&store, &attachments_dir, &message_id, &mut rendered)?;
        Ok(Some((message, rendered)))
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn is_trusted_sender(
    state: State<'_, AppState>,
    account_id: String,
    email: String,
) -> std::result::Result<bool, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || Ok(store.is_trusted_sender(&account_id, &email)?.is_some()))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

fn resolve_privacy_mode(
    store: &Store,
    message: &Message,
    privacy_mode: PrivacyMode,
) -> std::result::Result<PrivacyMode, PebbleError> {
    match privacy_mode {
        PrivacyMode::Strict | PrivacyMode::LoadOnce => {
            match store.is_trusted_sender(&message.account_id, &message.from_address)? {
                Some(TrustType::All | TrustType::Images) => Ok(PrivacyMode::LoadOnce),
                None => Ok(privacy_mode),
            }
        }
        PrivacyMode::TrustSender(sender)
            if sender.eq_ignore_ascii_case(message.from_address.trim()) =>
        {
            match store.is_trusted_sender(&message.account_id, &message.from_address)? {
                Some(TrustType::All | TrustType::Images) => Ok(PrivacyMode::LoadOnce),
                None => Ok(PrivacyMode::Strict),
            }
        }
        PrivacyMode::TrustSender(_) => Ok(PrivacyMode::Strict),
        PrivacyMode::Off => Ok(PrivacyMode::Off),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{
        new_id, now_timestamp, Account, EmailAddress, Folder, FolderRole, FolderType, Message,
        ProviderType, TrustType, TrustedSender,
    };

    #[test]
    fn cid_images_resolve_locally_without_loading_external_images() {
        let (store, message) = store_with_trusted_sender(TrustType::Images);
        let root = std::env::temp_dir().join(format!("pebble-cid-{}", new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("image.png");
        std::fs::write(&path, b"png-fixture").unwrap();
        store
            .insert_attachment(&pebble_core::Attachment {
                id: new_id(),
                message_id: message.id.clone(),
                filename: "image.png".into(),
                mime_type: "image/png".into(),
                size: 11,
                local_path: Some(path.to_string_lossy().into_owned()),
                content_id: Some("<image@example.com>".into()),
                is_inline: true,
            })
            .unwrap();
        let mut rendered = PrivacyGuard::new().render_safe_html(
            r#"<img src="cid:image@example.com" width="120" height="60"><img src="https://remote.invalid/image.png"><img src="cid:missing">"#,
            &PrivacyMode::Strict,
        );
        resolve_inline_images(&store, &root, &message.id, &mut rendered).unwrap();
        assert!(rendered
            .html
            .contains("data:image/png;base64,cG5nLWZpeHR1cmU="));
        assert_eq!(rendered.images_blocked, 1);
        assert!(!rendered.html.contains(" src=\"https://remote.invalid"));
        assert!(!rendered.html.contains("cid:missing"));
        let other = make_message("account-1", "other@example.com");
        store.insert_message(&other, &[]).unwrap();
        let mut other_html = PrivacyGuard::new()
            .render_safe_html(r#"<img src="cid:image@example.com">"#, &PrivacyMode::Strict);
        resolve_inline_images(&store, &root, &other.id, &mut other_html).unwrap();
        assert!(!other_html.html.contains("data:image"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Signature images as real mailers send them (issue #102): non-standard
    /// or generic MIME types, and cid references whose case or encoding
    /// differs from the Content-ID.
    #[test]
    fn signature_images_with_loose_types_and_cids_are_inlined() {
        let (store, message) = store_with_trusted_sender(TrustType::Images);
        let root = std::env::temp_dir().join(format!("pebble-cid-loose-{}", new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let png = b"\x89PNG\r\n\x1a\nrest-of-png".to_vec();
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
        let not_an_image = b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec();
        for (content_id, file, mime, bytes) in [
            (
                "<image001.png@01DA1234.ABCD>",
                "logo.png",
                "application/octet-stream",
                &png,
            ),
            (
                "<Banner@Example.COM>",
                "banner.jpg",
                "image/jpg; name=banner.jpg",
                &jpeg,
            ),
            (
                "<vector>",
                "vector.bin",
                "application/octet-stream",
                &not_an_image,
            ),
        ] {
            let path = root.join(file);
            std::fs::write(&path, bytes).unwrap();
            store
                .insert_attachment(&pebble_core::Attachment {
                    id: new_id(),
                    message_id: message.id.clone(),
                    filename: file.into(),
                    mime_type: mime.into(),
                    size: bytes.len() as i64,
                    local_path: Some(path.to_string_lossy().into_owned()),
                    content_id: Some(content_id.into()),
                    is_inline: true,
                })
                .unwrap();
        }
        let mut rendered = PrivacyGuard::new().render_safe_html(
            r#"<img src="cid:image001.png%4001DA1234.ABCD"><img src="CID:banner@example.com"><img src="cid:vector">"#,
            &PrivacyMode::Strict,
        );
        resolve_inline_images(&store, &root, &message.id, &mut rendered).unwrap();
        assert!(rendered
            .html
            .contains(&format!("data:image/png;base64,{}", STANDARD.encode(&png))));
        assert!(rendered.html.contains(&format!(
            "data:image/jpeg;base64,{}",
            STANDARD.encode(&jpeg)
        )));
        assert_eq!(
            rendered.html.matches("data:").count(),
            2,
            "non-images are never inlined"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn image_type_helpers_normalize_aliases_and_sniff_formats() {
        assert_eq!(normalize_image_mime("IMAGE/JPG"), "image/jpeg");
        assert_eq!(normalize_image_mime("image/x-png; name=a.png"), "image/png");
        assert_eq!(sniff_image_mime(b"GIF89a..."), Some("image/gif"));
        assert_eq!(
            sniff_image_mime(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniff_image_mime(b"<svg/>"), None);
        assert_eq!(normalize_cid(" <A%40B.c> "), "a@b.c");
        assert_eq!(normalize_cid("100%"), "100%");
    }

    #[test]
    fn cid_cannot_read_files_outside_attachment_root_or_render_svg() {
        let (store, message) = store_with_trusted_sender(TrustType::Images);
        let base = std::env::temp_dir().join(format!("pebble-cid-safety-{}", new_id()));
        let root = base.join("attachments");
        std::fs::create_dir_all(&root).unwrap();
        for (cid, path, mime) in [
            ("outside", base.join("secret.png"), "image/png"),
            ("vector", root.join("vector.svg"), "image/svg+xml"),
        ] {
            std::fs::write(&path, b"secret").unwrap();
            store
                .insert_attachment(&pebble_core::Attachment {
                    id: new_id(),
                    message_id: message.id.clone(),
                    filename: cid.into(),
                    mime_type: mime.into(),
                    size: 6,
                    local_path: Some(path.to_string_lossy().into_owned()),
                    content_id: Some(cid.into()),
                    is_inline: true,
                })
                .unwrap();
        }
        let mut rendered = PrivacyGuard::new().render_safe_html(
            r#"<img src="cid:outside"><img src="cid:vector">"#,
            &PrivacyMode::Off,
        );
        resolve_inline_images(&store, &root, &message.id, &mut rendered).unwrap();
        assert!(!rendered.html.contains("data:"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn repeated_cid_references_share_a_cumulative_expansion_budget() {
        let (store, message) = store_with_trusted_sender(TrustType::Images);
        let root = std::env::temp_dir().join(format!("pebble-cid-repeat-{}", new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("image.png");
        let bytes = vec![42_u8; 256 * 1024];
        std::fs::write(&path, &bytes).unwrap();
        store
            .insert_attachment(&pebble_core::Attachment {
                id: new_id(),
                message_id: message.id.clone(),
                filename: "image.png".into(),
                mime_type: "image/png".into(),
                size: bytes.len() as i64,
                local_path: Some(path.to_string_lossy().into_owned()),
                content_id: Some("repeated".into()),
                is_inline: true,
            })
            .unwrap();
        let html = "<img src=\"cid:repeated\">".repeat(128);
        let mut rendered = PrivacyGuard::new().render_safe_html(&html, &PrivacyMode::Strict);
        let initial_size = rendered.html.len();
        resolve_inline_images(&store, &root, &message.id, &mut rendered).unwrap();
        let replacements = rendered.html.matches("data:image/png;base64,").count();
        let data_url_size = "data:image/png;base64,".len() + STANDARD.encode(&bytes).len();
        assert_eq!(replacements, MAX_INLINE_EXPANSION_BYTES / data_url_size);
        assert!(replacements < 128);
        assert!(rendered.html.len() <= initial_size + MAX_INLINE_EXPANSION_BYTES);
        assert!(!rendered.html.contains("cid:"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn make_account(id: &str) -> Account {
        let now = now_timestamp();
        Account {
            account_label: None,
            provider_display_name: None,
            id: id.to_string(),
            email: "me@example.com".to_string(),
            display_name: "Me".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_message(account_id: &str, from_address: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: "remote-1".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: Some("thread-1".to_string()),
            subject: "Subject".to_string(),
            snippet: "Snippet".to_string(),
            from_address: from_address.to_string(),
            from_name: "Trusted".to_string(),
            to_list: vec![EmailAddress {
                name: None,
                address: "me@example.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "Body".to_string(),
            body_html_raw: "<p>Body</p>".to_string(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_folder(account_id: &str) -> Folder {
        Folder {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            role: Some(FolderRole::Inbox),
            folder_type: FolderType::Folder,
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    fn store_with_trusted_sender(trust_type: TrustType) -> (Store, Message) {
        let store = Store::open_in_memory().unwrap();
        let account = make_account("account-1");
        store.insert_account(&account).unwrap();
        let folder = make_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = make_message(&account.id, "trusted@example.com");
        store.insert_message(&message, &[folder.id]).unwrap();
        store
            .trust_sender(&TrustedSender {
                account_id: account.id,
                email: "trusted@example.com".to_string(),
                trust_type,
                created_at: now_timestamp(),
            })
            .unwrap();
        (store, message)
    }

    #[test]
    fn persistent_all_trust_resolves_to_tracker_safe_image_loading() {
        let (store, message) = store_with_trusted_sender(TrustType::All);

        let mode = resolve_privacy_mode(&store, &message, PrivacyMode::LoadOnce).unwrap();

        assert!(matches!(mode, PrivacyMode::LoadOnce));
    }

    #[test]
    fn sender_override_with_persistent_all_trust_stays_tracker_safe() {
        let (store, message) = store_with_trusted_sender(TrustType::All);

        let mode = resolve_privacy_mode(
            &store,
            &message,
            PrivacyMode::TrustSender("trusted@example.com".to_string()),
        )
        .unwrap();

        assert!(matches!(mode, PrivacyMode::LoadOnce));
    }

    #[test]
    fn sender_override_cannot_be_reused_for_a_different_message_sender() {
        let (store, mut message) = store_with_trusted_sender(TrustType::All);
        message.from_address = "other@example.com".to_string();

        let mode = resolve_privacy_mode(
            &store,
            &message,
            PrivacyMode::TrustSender("trusted@example.com".to_string()),
        )
        .unwrap();

        assert!(matches!(mode, PrivacyMode::Strict));
    }

    #[test]
    fn matching_sender_override_is_rejected_without_persisted_trust() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account("account-1");
        store.insert_account(&account).unwrap();
        let message = make_message(&account.id, "sender@example.com");

        let mode = resolve_privacy_mode(
            &store,
            &message,
            PrivacyMode::TrustSender("sender@example.com".to_string()),
        )
        .unwrap();

        assert!(matches!(mode, PrivacyMode::Strict));
    }

    #[test]
    fn images_only_trust_cannot_be_escalated_by_sender_override() {
        let (store, message) = store_with_trusted_sender(TrustType::Images);

        let mode = resolve_privacy_mode(
            &store,
            &message,
            PrivacyMode::TrustSender("trusted@example.com".to_string()),
        )
        .unwrap();

        assert!(matches!(mode, PrivacyMode::LoadOnce));
    }
}
