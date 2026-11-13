//! RFC 3501 §§5.1.3, 6.3.3, 6.3.5: names on the wire use modified UTF-7;
//! RENAME includes descendants, and the delimiter comes from LIST, never a guess.
use super::*;

#[derive(serde::Serialize, Clone)]
pub struct ImapFolderParent {
    pub remote_id: String,
    pub name: String,
    pub delimiter: Option<String>,
    pub selectable: bool,
    pub is_system: bool,
}

impl ImapProvider {
    /// Include nonselectable hierarchy containers as CREATE parents, without
    /// treating them as message-bearing folders in sync or Move To.
    pub async fn folder_parents(&self) -> Result<Vec<ImapFolderParent>> {
        let mut guard = self.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?;
        let names: Vec<_> = session
            .list(Some(""), Some("*"))
            .await
            .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?
            .try_collect()
            .await
            .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?;
        Ok(names
            .iter()
            .map(|n| ImapFolderParent {
                remote_id: n.name().into(),
                name: decode_folder_name(n.name()),
                delimiter: n.delimiter().map(str::to_owned),
                selectable: should_sync_listed_mailbox(n.attributes()),
                is_system: detect_folder_role_from_attributes(n.attributes()).is_some()
                    || detect_folder_role(&decode_folder_name(n.name())).is_some()
                    || n.attributes()
                        .iter()
                        .any(|a| matches!(a, NameAttribute::All | NameAttribute::Flagged)),
            })
            .collect())
    }
    pub async fn folder_delimiter(&self, mailbox: Option<&str>) -> Result<Option<String>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?;
        let names: Vec<_> = sess
            .list(Some(""), Some(mailbox.unwrap_or("")))
            .await
            .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?
            .try_collect()
            .await
            .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?;
        let name = names
            .first()
            .ok_or_else(|| PebbleError::Validation("Mailbox namespace not found".into()))?;
        Ok(name.delimiter().map(str::to_owned))
    }

    pub async fn create_folder(&self, remote_name: &str) -> Result<()> {
        let mut guard = self.session.lock().await;
        guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?
            .create(remote_name)
            .await
            .map_err(|e| PebbleError::Network(format!("CREATE failed: {e}")))
    }

    pub async fn rename_folder(&self, old_remote_name: &str, new_remote_name: &str) -> Result<()> {
        let mut guard = self.session.lock().await;
        guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?
            .rename(old_remote_name, new_remote_name)
            .await
            .map_err(|e| PebbleError::Network(format!("RENAME failed: {e}")))
    }
}

pub fn encode_folder_segment(name: &str, delimiter: Option<&str>) -> Result<String> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.chars().any(char::is_control)
        || delimiter.is_some_and(|d| !d.is_empty() && name.contains(d))
        || name.len() > 255
    {
        return Err(PebbleError::Validation(
            "Use a nonempty folder name without the hierarchy delimiter or control characters"
                .into(),
        ));
    }
    Ok(utf7_imap::encode_utf7_imap(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf7_segments_obey_server_delimiter() {
        assert_eq!(
            encode_folder_segment("收件", Some(".")).unwrap(),
            utf7_imap::encode_utf7_imap("收件".into())
        );
        assert!(encode_folder_segment("a.b", Some(".")).is_err());
        assert!(encode_folder_segment("a/b", Some(".")).is_ok());
        for name in ["", " ", "a\r\nCREATE x", " trailing "] {
            assert!(encode_folder_segment(name, None).is_err());
        }
    }
}
