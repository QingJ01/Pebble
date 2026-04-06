use pebble_core::{new_id, PebbleError, Result};
use rusqlite::OptionalExtension;
use std::collections::HashMap;

use crate::Store;

/// A label entity (matches the `labels` table).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    pub color: String,
    pub is_system: bool,
    pub rule_id: Option<String>,
}

impl Store {
    /// Find or create a label by name, returning its id.
    pub fn find_or_create_label(&self, name: &str) -> Result<String> {
        self.with_write(|conn| {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM labels WHERE name = ?1",
                    rusqlite::params![name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            if let Some(id) = existing {
                return Ok(id);
            }

            let id = new_id();
            conn.execute(
                "INSERT INTO labels (id, name, color, is_system) VALUES (?1, ?2, '#808080', 0)",
                rusqlite::params![id, name],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(id)
        })
    }

    /// Add a label to a message (by label name).
    pub fn add_label(&self, message_id: &str, label_name: &str) -> Result<()> {
        let label_id = self.find_or_create_label(label_name)?;
        self.with_write(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
                rusqlite::params![message_id, label_id],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    /// Remove a label from a message (by label name).
    pub fn remove_label(&self, message_id: &str, label_name: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "DELETE FROM message_labels WHERE message_id = ?1
                 AND label_id IN (SELECT id FROM labels WHERE name = ?2)",
                rusqlite::params![message_id, label_name],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    /// Get all labels for a message.
    pub fn get_message_labels(&self, message_id: &str) -> Result<Vec<Label>> {
        self.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT l.id, l.name, l.color, l.is_system, l.rule_id
                     FROM labels l
                     INNER JOIN message_labels ml ON ml.label_id = l.id
                     WHERE ml.message_id = ?1
                     ORDER BY l.name",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params![message_id], |row| {
                    let is_system: i32 = row.get(3)?;
                    Ok(Label {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                        is_system: is_system != 0,
                        rule_id: row.get(4)?,
                    })
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let mut labels = Vec::new();
            for row in rows {
                labels.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
            }
            Ok(labels)
        })
    }

    /// Get labels for multiple messages in one query.
    pub fn get_message_labels_batch(&self, message_ids: &[String]) -> Result<HashMap<String, Vec<Label>>> {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }

        self.with_read(|conn| {
            let placeholders: Vec<String> = (1..=message_ids.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "SELECT ml.message_id, l.id, l.name, l.color, l.is_system, l.rule_id
                 FROM message_labels ml
                 INNER JOIN labels l ON l.id = ml.label_id
                 WHERE ml.message_id IN ({})
                 ORDER BY ml.message_id, l.name",
                placeholders.join(", ")
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
                Vec::with_capacity(message_ids.len());
            for message_id in message_ids {
                param_values.push(Box::new(message_id.clone()));
            }
            let params: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|v| v.as_ref()).collect();

            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    let is_system: i32 = row.get(4)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        Label {
                            id: row.get(1)?,
                            name: row.get(2)?,
                            color: row.get(3)?,
                            is_system: is_system != 0,
                            rule_id: row.get(5)?,
                        },
                    ))
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            let mut result: HashMap<String, Vec<Label>> = HashMap::new();
            for message_id in message_ids {
                result.entry(message_id.clone()).or_default();
            }
            for row in rows {
                let (message_id, label) = row.map_err(|e| PebbleError::Storage(e.to_string()))?;
                result.entry(message_id).or_default().push(label);
            }
            Ok(result)
        })
    }

    /// List all labels.
    pub fn list_labels(&self) -> Result<Vec<Label>> {
        self.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, name, color, is_system, rule_id FROM labels ORDER BY name")
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    let is_system: i32 = row.get(3)?;
                    Ok(Label {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                        is_system: is_system != 0,
                        rule_id: row.get(4)?,
                    })
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let mut labels = Vec::new();
            for row in rows {
                labels.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
            }
            Ok(labels)
        })
    }
}
