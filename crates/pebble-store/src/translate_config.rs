use pebble_core::{PebbleError, Result, TranslateConfig};
use rusqlite::params;
use crate::Store;

impl Store {
    pub(crate) fn save_translate_config_with_conn(conn: &rusqlite::Connection, config: &TranslateConfig) -> Result<()> {
        conn.execute(
            "INSERT INTO translate_config (id, provider_type, config, is_enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 provider_type = excluded.provider_type,
                 config = excluded.config,
                 is_enabled = excluded.is_enabled,
                 updated_at = excluded.updated_at",
            params![config.id, config.provider_type, config.config, config.is_enabled as i32, config.created_at, config.updated_at],
        )?;
        Ok(())
    }

    pub fn save_translate_config(&self, config: &TranslateConfig) -> Result<()> {
        self.with_write(|conn| Self::save_translate_config_with_conn(conn, config))
    }

    pub fn get_translate_config(&self) -> Result<Option<TranslateConfig>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, provider_type, config, is_enabled, created_at, updated_at FROM translate_config WHERE id = 'active'"
            )?;
            let mut rows = stmt.query_map([], |row| {
                Ok(TranslateConfig {
                    id: row.get(0)?,
                    provider_type: row.get(1)?,
                    config: row.get(2)?,
                    is_enabled: row.get::<_, i32>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })?;
            match rows.next() {
                Some(Ok(config)) => Ok(Some(config)),
                Some(Err(e)) => Err(PebbleError::Storage(e.to_string())),
                None => Ok(None),
            }
        })
    }

    pub fn delete_translate_config(&self) -> Result<()> {
        self.with_write(|conn| {
            conn.execute("DELETE FROM translate_config WHERE id = 'active'", [])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::now_timestamp;

    #[test]
    fn test_translate_config_save_and_load() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();
        let config = TranslateConfig {
            id: "active".to_string(),
            provider_type: "deeplx".to_string(),
            config: r#"{"endpoint":"http://localhost:1188/translate"}"#.to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.save_translate_config(&config).unwrap();
        let loaded = store.get_translate_config().unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.provider_type, "deeplx");
        assert!(loaded.is_enabled);
    }

    #[test]
    fn test_translate_config_upsert() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();
        let config1 = TranslateConfig {
            id: "active".to_string(),
            provider_type: "deeplx".to_string(),
            config: "{}".to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.save_translate_config(&config1).unwrap();

        let config2 = TranslateConfig {
            id: "active".to_string(),
            provider_type: "deepl".to_string(),
            config: r#"{"api_key":"test"}"#.to_string(),
            is_enabled: false,
            created_at: now,
            updated_at: now + 1,
        };
        store.save_translate_config(&config2).unwrap();

        let loaded = store.get_translate_config().unwrap().unwrap();
        assert_eq!(loaded.provider_type, "deepl");
        assert!(!loaded.is_enabled);
    }

    #[test]
    fn test_translate_config_delete() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();
        let config = TranslateConfig {
            id: "active".to_string(),
            provider_type: "llm".to_string(),
            config: "{}".to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.save_translate_config(&config).unwrap();
        store.delete_translate_config().unwrap();
        let loaded = store.get_translate_config().unwrap();
        assert!(loaded.is_none());
    }
}
