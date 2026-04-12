use pebble_core::{KanbanCard, KanbanColumn, Result};
use rusqlite::params;

use crate::Store;

fn column_to_str(col: &KanbanColumn) -> &'static str {
    match col {
        KanbanColumn::Todo => "todo",
        KanbanColumn::Waiting => "waiting",
        KanbanColumn::Done => "done",
    }
}

fn str_to_column(s: &str) -> KanbanColumn {
    match s {
        "waiting" => KanbanColumn::Waiting,
        "done" => KanbanColumn::Done,
        _ => KanbanColumn::Todo,
    }
}

fn row_to_kanban_card(row: &rusqlite::Row) -> rusqlite::Result<KanbanCard> {
    Ok(KanbanCard {
        message_id: row.get(0)?,
        column: str_to_column(&row.get::<_, String>(1)?),
        position: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

impl Store {
    pub(crate) fn upsert_kanban_card_with_conn(conn: &rusqlite::Connection, card: &KanbanCard) -> Result<()> {
        conn.execute(
            "INSERT INTO kanban_cards (message_id, column_name, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(message_id) DO UPDATE SET
               column_name = excluded.column_name,
               position = excluded.position,
               updated_at = excluded.updated_at",
            params![
                card.message_id,
                column_to_str(&card.column),
                card.position,
                card.created_at,
                card.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_kanban_card(&self, card: &KanbanCard) -> Result<()> {
        self.with_write(|conn| Self::upsert_kanban_card_with_conn(conn, card))
    }

    pub fn list_kanban_cards(&self, column: Option<&KanbanColumn>) -> Result<Vec<KanbanCard>> {
        self.with_read(|conn| {
            match column {
                Some(col) => {
                    let mut stmt = conn
                        .prepare(
                            "SELECT message_id, column_name, position, created_at, updated_at
                             FROM kanban_cards WHERE column_name = ?1 ORDER BY position ASC",
                        )?;
                    let rows = stmt
                        .query_map(params![column_to_str(col)], row_to_kanban_card)?;
                    let mut cards = Vec::new();
                    for row in rows {
                        cards.push(row?);
                    }
                    Ok(cards)
                }
                None => {
                    let mut stmt = conn
                        .prepare(
                            "SELECT message_id, column_name, position, created_at, updated_at
                             FROM kanban_cards ORDER BY position ASC",
                        )?;
                    let rows = stmt
                        .query_map([], row_to_kanban_card)?;
                    let mut cards = Vec::new();
                    for row in rows {
                        cards.push(row?);
                    }
                    Ok(cards)
                }
            }
        })
    }

    pub fn move_kanban_card(
        &self,
        message_id: &str,
        column: &KanbanColumn,
        position: i32,
    ) -> Result<()> {
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            conn.execute(
                "UPDATE kanban_cards SET column_name = ?1, position = ?2, updated_at = ?3
                 WHERE message_id = ?4",
                params![column_to_str(column), position, now, message_id],
            )?;
            Ok(())
        })
    }

    pub fn delete_kanban_card(&self, message_id: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "DELETE FROM kanban_cards WHERE message_id = ?1",
                params![message_id],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn setup_store_with_message() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let now = pebble_core::now_timestamp();
        let account = pebble_core::Account {
            id: pebble_core::new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            provider: pebble_core::ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();
        let folder = pebble_core::Folder {
            id: pebble_core::new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role: Some(pebble_core::FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        let msg_id = pebble_core::new_id();
        let msg = pebble_core::Message {
            id: msg_id.clone(),
            account_id: account.id.clone(),
            remote_id: "1".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Test".to_string(),
            snippet: "Test snippet".to_string(),
            from_address: "sender@example.com".to_string(),
            from_name: "Sender".to_string(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "body".to_string(),
            body_html_raw: "<p>body</p>".to_string(),
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
        };
        store.insert_message(&msg, &[folder.id.clone()]).unwrap();
        (store, msg_id)
    }

    #[test]
    fn test_kanban_upsert_and_list() {
        let (store, msg_id) = setup_store_with_message();
        let now = pebble_core::now_timestamp();
        let card = KanbanCard {
            message_id: msg_id.clone(),
            column: KanbanColumn::Todo,
            position: 0,
            created_at: now,
            updated_at: now,
        };
        store.upsert_kanban_card(&card).unwrap();

        let all = store.list_kanban_cards(None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].message_id, msg_id);
        assert_eq!(all[0].column, KanbanColumn::Todo);

        let todos = store.list_kanban_cards(Some(&KanbanColumn::Todo)).unwrap();
        assert_eq!(todos.len(), 1);

        let waiting = store.list_kanban_cards(Some(&KanbanColumn::Waiting)).unwrap();
        assert_eq!(waiting.len(), 0);

        // Upsert should update
        let card2 = KanbanCard {
            message_id: msg_id.clone(),
            column: KanbanColumn::Done,
            position: 1,
            created_at: now,
            updated_at: now + 1,
        };
        store.upsert_kanban_card(&card2).unwrap();
        let all = store.list_kanban_cards(None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].column, KanbanColumn::Done);
    }

    #[test]
    fn test_kanban_move() {
        let (store, msg_id) = setup_store_with_message();
        let now = pebble_core::now_timestamp();
        let card = KanbanCard {
            message_id: msg_id.clone(),
            column: KanbanColumn::Todo,
            position: 0,
            created_at: now,
            updated_at: now,
        };
        store.upsert_kanban_card(&card).unwrap();
        store.move_kanban_card(&msg_id, &KanbanColumn::Waiting, 5).unwrap();

        let cards = store.list_kanban_cards(Some(&KanbanColumn::Waiting)).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].position, 5);
    }

    #[test]
    fn test_kanban_delete() {
        let (store, msg_id) = setup_store_with_message();
        let now = pebble_core::now_timestamp();
        let card = KanbanCard {
            message_id: msg_id.clone(),
            column: KanbanColumn::Todo,
            position: 0,
            created_at: now,
            updated_at: now,
        };
        store.upsert_kanban_card(&card).unwrap();
        store.delete_kanban_card(&msg_id).unwrap();

        let all = store.list_kanban_cards(None).unwrap();
        assert_eq!(all.len(), 0);
    }
}
