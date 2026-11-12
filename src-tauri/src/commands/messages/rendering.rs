use crate::state::AppState;
use base64::{engine::general_purpose::STANDARD, Engine};
use pebble_core::{Message, PebbleError, PrivacyMode, RenderedHtml, TrustType};
use pebble_privacy::PrivacyGuard;
use pebble_store::Store;
use std::{collections::HashMap, path::Path};
use tauri::State;

const MAX_INLINE_EXPANSION_BYTES: usize = 20 * 1024 * 1024;

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
        let mime = attachment.mime_type.to_ascii_lowercase();
        if !matches!(
            mime.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp"
        ) {
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
        total_size += bytes.len() as u64;
        images.insert(
            cid.trim().trim_matches(['<', '>']).to_string(),
            format!("data:{mime};base64,{}", STANDARD.encode(bytes)),
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
                        let cid = src[4..].trim().trim_matches(['<', '>']);
                        if let Some(data) = images.get(cid).filter(|data| {
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
