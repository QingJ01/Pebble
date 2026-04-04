use pebble_core::{Account, PebbleError, ProviderType, Result};
use rusqlite::OptionalExtension;

use crate::Store;

fn provider_to_str(p: &ProviderType) -> &'static str {
    match p {
        ProviderType::Imap => "imap",
        ProviderType::Gmail => "gmail",
        ProviderType::Outlook => "outlook",
    }
}

fn str_to_provider(s: &str) -> ProviderType {
    match s {
        "gmail" => ProviderType::Gmail,
        "outlook" => ProviderType::Outlook,
        _ => ProviderType::Imap,
    }
}

impl Store {
    pub fn insert_account(&self, account: &Account) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, provider, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    account.id,
                    account.email,
                    account.display_name,
                    provider_to_str(&account.provider),
                    account.created_at,
                    account.updated_at,
                ],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        self.with_conn(|conn| {
            let result = conn
                .query_row(
                    "SELECT id, email, display_name, provider, created_at, updated_at
                     FROM accounts WHERE id = ?1",
                    rusqlite::params![id],
                    |row| {
                        Ok(Account {
                            id: row.get(0)?,
                            email: row.get(1)?,
                            display_name: row.get(2)?,
                            provider: str_to_provider(&row.get::<_, String>(3)?),
                            created_at: row.get(4)?,
                            updated_at: row.get(5)?,
                        })
                    },
                )
                .optional()
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(result)
        })
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, provider, created_at, updated_at
                     FROM accounts ORDER BY created_at ASC",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Account {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        provider: str_to_provider(&row.get::<_, String>(3)?),
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let mut accounts = Vec::new();
            for row in rows {
                accounts.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
            }
            Ok(accounts)
        })
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM accounts WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }
}
