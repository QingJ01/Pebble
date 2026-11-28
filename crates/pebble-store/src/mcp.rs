//! Local MCP clients, their grants and the audit log (#107, M4).

use pebble_core::Result;
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use crate::Store;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClient {
    pub id: String,
    pub name: String,
    /// Account id → access level (`metadata`, `summary`, `body`, `drafts`).
    pub grants: std::collections::BTreeMap<String, String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAuditRow {
    pub id: i64,
    pub client_id: String,
    pub tool: String,
    /// Account ids the call touched, comma separated.
    pub accounts: String,
    pub results: u32,
    /// `ok`, or a short error code (`unauthorized`, `forbidden`, …).
    pub outcome: String,
    pub created_at: i64,
}

const CLIENT_COLUMNS: &str = "id, name, grants_json, created_at, last_used_at, revoked_at";

fn row_to_client(row: &Row<'_>) -> rusqlite::Result<McpClient> {
    let grants: String = row.get(2)?;
    Ok(McpClient {
        id: row.get(0)?,
        name: row.get(1)?,
        grants: serde_json::from_str(&grants).unwrap_or_default(),
        created_at: row.get(3)?,
        last_used_at: row.get(4)?,
        revoked_at: row.get(5)?,
    })
}

impl Store {
    pub fn mcp_enabled(&self) -> Result<bool> {
        self.with_read(|conn| {
            Ok(conn
                .query_row("SELECT enabled FROM mcp_settings WHERE id = 1", [], |row| {
                    row.get::<_, bool>(0)
                })
                .optional()?
                .unwrap_or(false))
        })
    }

    pub fn set_mcp_enabled(&self, enabled: bool) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO mcp_settings (id, enabled) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled",
                params![enabled],
            )?;
            Ok(())
        })
    }

    pub fn insert_mcp_client(
        &self,
        id: &str,
        name: &str,
        token_hash: &str,
        grants: &std::collections::BTreeMap<String, String>,
        now: i64,
    ) -> Result<()> {
        let grants = serde_json::to_string(grants).unwrap_or_else(|_| "{}".into());
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO mcp_clients (id, name, token_hash, grants_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, name, token_hash, grants, now],
            )?;
            Ok(())
        })
    }

    /// The live (not revoked) client holding a token with this hash.
    pub fn mcp_client_by_token_hash(&self, token_hash: &str) -> Result<Option<McpClient>> {
        self.with_read(|conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "SELECT {CLIENT_COLUMNS} FROM mcp_clients
                         WHERE token_hash = ?1 AND revoked_at IS NULL"
                    ),
                    params![token_hash],
                    row_to_client,
                )
                .optional()?)
        })
    }

    /// Clients, live ones first, newest first.
    pub fn list_mcp_clients(&self) -> Result<Vec<McpClient>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CLIENT_COLUMNS} FROM mcp_clients
                 ORDER BY revoked_at IS NOT NULL, created_at DESC"
            ))?;
            let rows = stmt
                .query_map([], row_to_client)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn set_mcp_grants(
        &self,
        id: &str,
        grants: &std::collections::BTreeMap<String, String>,
    ) -> Result<bool> {
        let grants = serde_json::to_string(grants).unwrap_or_else(|_| "{}".into());
        self.with_write(|conn| {
            Ok(conn.execute(
                "UPDATE mcp_clients SET grants_json = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id, grants],
            )? == 1)
        })
    }

    /// Revoke a client: its token stops working at once.
    pub fn revoke_mcp_client(&self, id: &str, now: i64) -> Result<bool> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "UPDATE mcp_clients SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id, now],
            )? == 1)
        })
    }

    pub fn touch_mcp_client(&self, id: &str, now: i64) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE mcp_clients SET last_used_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            Ok(())
        })
    }

    /// Drop an account from every client's grants (account removed).
    pub fn remove_account_from_mcp_grants(&self, account_id: &str) -> Result<()> {
        for client in self.list_mcp_clients()? {
            if client.grants.contains_key(account_id) {
                let mut grants = client.grants.clone();
                grants.remove(account_id);
                self.set_mcp_grants(&client.id, &grants)?;
            }
        }
        Ok(())
    }

    pub fn insert_mcp_audit(
        &self,
        client_id: &str,
        tool: &str,
        accounts: &[String],
        results: u32,
        outcome: &str,
        now: i64,
    ) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO mcp_audit (client_id, tool, accounts, results, outcome, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![client_id, tool, accounts.join(","), results, outcome, now],
            )?;
            Ok(())
        })
    }

    /// Recent calls, newest first (all clients when `client_id` is None).
    pub fn list_mcp_audit(&self, client_id: Option<&str>, limit: u32) -> Result<Vec<McpAuditRow>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, client_id, tool, accounts, results, outcome, created_at FROM mcp_audit
                 WHERE (?1 IS NULL OR client_id = ?1)
                 ORDER BY created_at DESC, id DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![client_id, limit], |row| {
                    Ok(McpAuditRow {
                        id: row.get(0)?,
                        client_id: row.get(1)?,
                        tool: row.get(2)?,
                        accounts: row.get(3)?,
                        results: row.get(4)?,
                        outcome: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Date of the oldest message stored for an account (how far back
    /// search reaches).
    pub fn earliest_message_date(&self, account_id: &str) -> Result<Option<i64>> {
        self.with_read(|conn| {
            Ok(conn.query_row(
                "SELECT MIN(date) FROM messages WHERE account_id = ?1 AND is_deleted = 0",
                params![account_id],
                |row| row.get(0),
            )?)
        })
    }

    /// Delete audit rows older than `before`.
    pub fn prune_mcp_audit(&self, before: i64) -> Result<usize> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "DELETE FROM mcp_audit WHERE created_at < ?1",
                params![before],
            )?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn clients_pair_revoke_and_audit() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.mcp_enabled().unwrap(), "off by default");
        store.set_mcp_enabled(true).unwrap();
        assert!(store.mcp_enabled().unwrap());

        let grants = BTreeMap::from([("work".to_string(), "summary".to_string())]);
        store
            .insert_mcp_client("c1", "Claude Desktop", "hash1", &grants, 10)
            .unwrap();
        let client = store.mcp_client_by_token_hash("hash1").unwrap().unwrap();
        assert_eq!(
            (client.name.as_str(), client.grants["work"].as_str()),
            ("Claude Desktop", "summary")
        );
        assert!(store.mcp_client_by_token_hash("other").unwrap().is_none());

        store.remove_account_from_mcp_grants("work").unwrap();
        assert!(store.list_mcp_clients().unwrap()[0].grants.is_empty());

        store
            .insert_mcp_audit("c1", "mail_search", &["work".into()], 3, "ok", 11)
            .unwrap();
        store
            .insert_mcp_audit("c1", "mail_get_thread", &[], 0, "forbidden", 12)
            .unwrap();
        let audit = store.list_mcp_audit(Some("c1"), 10).unwrap();
        assert_eq!(audit[0].outcome, "forbidden");
        assert_eq!(audit[1].accounts, "work");

        assert!(store.revoke_mcp_client("c1", 20).unwrap());
        assert!(
            store.mcp_client_by_token_hash("hash1").unwrap().is_none(),
            "revoked tokens stop working"
        );
        assert!(!store.set_mcp_grants("c1", &grants).unwrap());
        assert_eq!(store.prune_mcp_audit(12).unwrap(), 1);
    }
}
