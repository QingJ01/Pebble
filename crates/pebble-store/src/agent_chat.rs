//! Chat messages with the assistant (#107, M3). Payloads arrive encrypted.

use pebble_core::Result;
use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentChatRow {
    pub id: String,
    pub role: String,
    pub run_id: Option<String>,
    pub status: String,
    pub payload: Vec<u8>,
    pub created_at: i64,
}

impl Store {
    pub fn insert_agent_chat(&self, row: &AgentChatRow) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO agent_chat (id, role, run_id, status, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    row.id,
                    row.role,
                    row.run_id,
                    row.status,
                    row.payload,
                    row.created_at
                ],
            )?;
            Ok(())
        })
    }

    /// The newest `limit` messages, oldest first.
    pub fn list_agent_chat(&self, limit: u32) -> Result<Vec<AgentChatRow>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, role, run_id, status, payload, created_at FROM (
                     SELECT rowid AS seq, * FROM agent_chat ORDER BY created_at DESC, seq DESC LIMIT ?1
                 ) ORDER BY created_at, seq",
            )?;
            let rows = stmt
                .query_map(params![limit.min(1_000)], |row| {
                    Ok(AgentChatRow {
                        id: row.get(0)?,
                        role: row.get(1)?,
                        run_id: row.get(2)?,
                        status: row.get(3)?,
                        payload: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn update_agent_chat_payload(&self, id: &str, payload: &[u8]) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE agent_chat SET payload = ?2 WHERE id = ?1",
                params![id, payload],
            )?;
            Ok(())
        })
    }

    pub fn clear_agent_chat(&self) -> Result<usize> {
        self.with_write(|conn| Ok(conn.execute("DELETE FROM agent_chat", [])?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, at: i64) -> AgentChatRow {
        AgentChatRow {
            id: id.into(),
            role: "user".into(),
            run_id: None,
            status: "done".into(),
            payload: vec![1, 2, 3],
            created_at: at,
        }
    }

    #[test]
    fn lists_the_newest_oldest_first_and_clears() {
        let store = Store::open_in_memory().unwrap();
        for (id, at) in [("a", 1), ("b", 2), ("c", 3)] {
            store.insert_agent_chat(&row(id, at)).unwrap();
        }
        let ids: Vec<_> = store
            .list_agent_chat(2)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec!["b", "c"]);
        store.update_agent_chat_payload("c", &[9]).unwrap();
        assert_eq!(store.list_agent_chat(10).unwrap()[2].payload, vec![9]);
        assert_eq!(store.clear_agent_chat().unwrap(), 3);
    }
}
