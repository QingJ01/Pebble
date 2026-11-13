//! Folder edits keep local IDs stable, including message membership, cursors and
//! failure records. No destructive refresh/delete-and-reinsert reconciliation.
use crate::{Store, SyncState};
use pebble_core::{Folder, PebbleError, Result};
use rusqlite::{params, OptionalExtension};

impl Store {
    pub fn reconcile_folder_rename(&self, account_id: &str, folders: &[Folder]) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let raw: Option<String> = tx.query_row("SELECT sync_state FROM accounts WHERE id=?1", [account_id], |r| r.get(0))?;
            let mut sync = SyncState::from_json_opt(raw.as_deref())?;
            let parent_ids: std::collections::HashMap<String,String> = {
                let mut query=tx.prepare("SELECT remote_id,id FROM folders WHERE account_id=?1")?;
                let rows=query.query_map([account_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?;
                rows.collect::<std::result::Result<_,_>>()?
            };
            for folder in folders {
                if folder.account_id != account_id { return Err(PebbleError::Validation("Cross-account folder rename".into())); }
                let old: String = tx.query_row("SELECT remote_id FROM folders WHERE id=?1 AND account_id=?2", params![folder.id, account_id], |r| r.get(0))?;
                let collision: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM folders WHERE account_id=?1 AND remote_id=?2 AND id<>?3)", params![account_id, folder.remote_id, folder.id], |r| r.get(0))?;
                if collision { return Err(PebbleError::Validation("Folder already exists".into())); }
                let parent=folder.parent_id.as_ref().map(|p|parent_ids.get(p).cloned().unwrap_or_else(||p.clone()));
                tx.execute("UPDATE folders SET name=?1,remote_id=?2,parent_id=?5 WHERE id=?3 AND account_id=?4", params![folder.name, folder.remote_id, folder.id, account_id,parent])?;
                if let Some(selected) = &mut sync.selected_imap_folder_remote_ids {
                    for id in selected { if *id == old { *id = folder.remote_id.clone(); } }
                }
            }
            tx.execute("UPDATE accounts SET sync_state=?1 WHERE id=?2", params![sync.to_json()?, account_id])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn include_managed_imap_folder(&self, account_id: &str, remote_id: &str) -> Result<()> {
        self.update_sync_state(account_id, |s| {
            if let Some(ids) = &mut s.selected_imap_folder_remote_ids {
                if !ids.iter().any(|id| id == remote_id) {
                    ids.push(remote_id.to_owned());
                }
            }
        })
    }

    /// Gmail Move To only removes the source label. Other labels are retained.
    pub fn move_message_label_membership(
        &self,
        message_id: &str,
        source_id: &str,
        target_id: &str,
    ) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN folders s ON s.id=?2 JOIN folders t ON t.id=?3 WHERE m.id=?1 AND m.account_id=s.account_id AND m.account_id=t.account_id)", params![message_id, source_id, target_id], |r| r.get(0))?;
            if !valid || source_id == target_id { return Err(PebbleError::Validation("Invalid folder move".into())); }
            tx.execute("INSERT OR IGNORE INTO message_folders(message_id,folder_id) VALUES(?1,?2)", params![message_id,target_id])?;
            tx.execute("DELETE FROM message_folders WHERE message_id=?1 AND folder_id=?2", params![message_id,source_id])?;
            tx.execute("UPDATE messages SET is_deleted=0,deleted_at=NULL,updated_at=?2 WHERE id=?1", params![message_id,pebble_core::now_timestamp()])?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Insert and claim in one transaction so the background worker cannot race
    /// the foreground move. An existing queued mutation must finish first.
    pub fn stage_managed_move(
        &self,
        account_id: &str,
        message_id: &str,
        payload: &str,
    ) -> Result<String> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND account_id=?2)",params![message_id,account_id],|r|r.get(0))?;
            if !owned { return Err(PebbleError::Validation("Message does not belong to this account".into())); }
            let active: Option<String> = tx.query_row("SELECT id FROM pending_mail_ops WHERE message_id=?1 AND status IN ('pending','in_progress','failed') LIMIT 1", [message_id], |r| r.get(0)).optional()?;
            if active.is_some() { return Err(PebbleError::Validation("This message has a pending operation; retry after it finishes".into())); }
            let id = pebble_core::new_id();
            tx.execute("INSERT INTO pending_mail_ops(id,account_id,message_id,op_type,payload_json,status,attempts,created_at,updated_at) VALUES(?1,?2,?3,'managed_move',?4,'in_progress',0,?5,?5)", params![id,account_id,message_id,payload,pebble_core::now_timestamp()])?;
            tx.commit()?;
            Ok(id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{Account, FolderType, Message, ProviderType};
    fn setup() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_account(&Account {
                id: "a".into(),
                email: "test@example.invalid".into(),
                display_name: "Test".into(),
                account_label: None,
                provider_display_name: None,
                color: None,
                provider: ProviderType::Gmail,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        store
    }
    fn folder(id: &str, remote: &str) -> Folder {
        Folder {
            id: id.into(),
            account_id: "a".into(),
            remote_id: remote.into(),
            name: remote.into(),
            folder_type: FolderType::Label,
            role: None,
            parent_id: None,
            color: None,
            is_system: false,
            sort_order: 0,
        }
    }
    fn message(id: &str) -> Message {
        serde_json::from_value(serde_json::json!({"id":id,"account_id":"a","remote_id":"5","message_id_header":"<move@example.invalid>","in_reply_to":null,"references_header":null,"thread_id":null,"subject":"Test","snippet":"","from_address":"test@example.invalid","from_name":"Test","to_list":[],"cc_list":[],"bcc_list":[],"body_text":"cached content","body_html_raw":"","has_attachments":false,"is_read":false,"is_starred":false,"is_draft":false,"date":1,"remote_version":null,"is_deleted":false,"deleted_at":null,"created_at":1,"updated_at":1})).unwrap()
    }
    #[test]
    fn gmail_move_retains_unrelated_labels_and_neighbor() {
        let store = setup();
        for f in [
            folder("source", "INBOX"),
            folder("target", "Label_2"),
            folder("keep", "Label_3"),
        ] {
            store.insert_folder(&f).unwrap();
        }
        store
            .insert_message(&message("moving"), &["source".into(), "keep".into()])
            .unwrap();
        let mut neighbor = message("neighbor");
        neighbor.remote_id = "6".into();
        store.insert_message(&neighbor, &["target".into()]).unwrap();
        store
            .move_message_label_membership("moving", "source", "target")
            .unwrap();
        let mut memberships = store.get_message_folder_ids("moving").unwrap();
        memberships.sort();
        assert_eq!(memberships, vec!["keep", "target"]);
        assert_eq!(
            store.get_message_folder_ids("neighbor").unwrap(),
            vec!["target"]
        );
        assert_eq!(
            store.get_message("neighbor").unwrap().unwrap().remote_id,
            "6"
        );
        assert_eq!(
            store.get_message("moving").unwrap().unwrap().body_text,
            "cached content"
        );
        assert!(store
            .move_message_label_membership("moving", "missing", "source")
            .is_err());
    }
    #[test]
    fn rename_keeps_memberships_cursors_and_selection() {
        let store = setup();
        let parent = folder("parent", "A");
        store.insert_folder(&parent).unwrap();
        let mut child = folder("child", "A.Child");
        child.parent_id = Some("A".into());
        store.insert_folder(&child).unwrap();
        store
            .insert_message(&message("cached"), &["child".into()])
            .unwrap();
        store
            .update_sync_state("a", |s| {
                s.selected_imap_folder_remote_ids = Some(vec!["A".into(), "A.Child".into()])
            })
            .unwrap();
        store
            .with_write(|c| {
                c.execute(
                    "INSERT INTO folder_sync_state VALUES('a','child','{\"last_uid\":9}',1)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let mut renamed = parent;
        renamed.remote_id = "B".into();
        renamed.name = "B".into();
        let mut renamed_child = child;
        renamed_child.remote_id = "B.Child".into();
        renamed_child.name = "B.Child".into();
        store
            .reconcile_folder_rename("a", &[renamed, renamed_child])
            .unwrap();
        let found = store
            .list_folders("a")
            .unwrap()
            .into_iter()
            .find(|f| f.id == "child")
            .unwrap();
        assert_eq!(found.parent_id.as_deref(), Some("parent"));
        assert_eq!(found.remote_id, "B.Child");
        assert_eq!(
            store.get_message_folder_ids("cached").unwrap(),
            vec!["child"]
        );
        assert_eq!(
            store
                .get_sync_state("a")
                .unwrap()
                .unwrap()
                .selected_imap_folder_remote_ids
                .unwrap(),
            vec!["B", "B.Child"]
        );
        let cursor: String = store
            .with_read(|c| {
                Ok(c.query_row(
                    "SELECT state FROM folder_sync_state WHERE folder_id='child'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(cursor, "{\"last_uid\":9}");
    }
    #[test]
    fn rename_collision_rolls_back_every_folder() {
        let store = setup();
        for f in [folder("a", "A"), folder("b", "B")] {
            store.insert_folder(&f).unwrap();
        }
        let mut first = folder("a", "C");
        first.name = "C".into();
        let second = folder("b", "C");
        assert!(store
            .reconcile_folder_rename("a", &[first, second])
            .is_err());
        let folders = store.list_folders("a").unwrap();
        assert!(folders.iter().any(|f| f.id == "a" && f.remote_id == "A"));
        assert!(folders.iter().any(|f| f.id == "b" && f.remote_id == "B"));
    }
    #[test]
    fn managed_move_claim_is_durable_exclusive_and_account_scoped() {
        let store = setup();
        store.insert_folder(&folder("source", "INBOX")).unwrap();
        store
            .insert_message(&message("m"), &["source".into()])
            .unwrap();
        let id = store.stage_managed_move("a", "m", "{}").unwrap();
        assert_eq!(store.list_pending_mail_ops("a").unwrap()[0].id, id);
        let flag = store
            .insert_pending_mail_op("a", "m", "update_flags", "{}")
            .unwrap();
        assert!(store
            .list_retryable_pending_mail_ops(50)
            .unwrap()
            .is_empty());
        store.mark_pending_mail_op_done(&flag).unwrap();
        assert!(store
            .list_retryable_pending_mail_ops(50)
            .unwrap()
            .is_empty());
        assert!(store.stage_managed_move("a", "m", "{}").is_err());
        assert!(store.stage_managed_move("other", "m", "{}").is_err());
        store.mark_pending_mail_op_failed(&id, "offline").unwrap();
        assert_eq!(
            store
                .pending_mail_ops_summary(Some("a"))
                .unwrap()
                .failed_count,
            1
        );
        assert!(store.stage_managed_move("a", "m", "{}").is_err());
    }
}
