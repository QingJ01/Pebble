use pebble_core::{Result, Rule};
use rusqlite::params;

use crate::Store;

fn row_to_rule(row: &rusqlite::Row) -> rusqlite::Result<Rule> {
    let is_enabled: i32 = row.get(5)?;
    Ok(Rule {
        id: row.get(0)?,
        name: row.get(1)?,
        priority: row.get(2)?,
        conditions: row.get(3)?,
        actions: row.get(4)?,
        is_enabled: is_enabled != 0,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

/// A value that changes whenever any rule is added, removed or edited: the
/// rules' full content (the table is tiny). `updated_at` alone is not
/// reliable, since a caller can save an edit with its old timestamp.
pub fn rules_fingerprint(conn: &rusqlite::Connection) -> Result<String> {
    Ok(conn.query_row(
        "SELECT COALESCE(group_concat(fingerprint, char(30)), '') FROM (
             SELECT id || char(31) || name || char(31) || priority || char(31)
                    || conditions || char(31) || actions || char(31)
                    || is_enabled || char(31) || updated_at AS fingerprint
             FROM rules ORDER BY id
         )",
        [],
        |row| row.get(0),
    )?)
}

impl Store {
    pub fn insert_rule(&self, rule: &Rule) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO rules (id, name, priority, conditions, actions, is_enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    rule.id,
                    rule.name,
                    rule.priority,
                    rule.conditions,
                    rule.actions,
                    rule.is_enabled as i32,
                    rule.created_at,
                    rule.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_rules(&self) -> Result<Vec<Rule>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, priority, conditions, actions, is_enabled, created_at, updated_at
                     FROM rules ORDER BY priority ASC",
            )?;
            let rows = stmt.query_map([], row_to_rule)?;
            let mut rules = Vec::new();
            for row in rows {
                rules.push(row?);
            }
            Ok(rules)
        })
    }

    pub fn update_rule(&self, rule: &Rule) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE rules SET name = ?1, priority = ?2, conditions = ?3, actions = ?4,
                 is_enabled = ?5, updated_at = MAX(?6, ?8) WHERE id = ?7",
                params![
                    rule.name,
                    rule.priority,
                    rule.conditions,
                    rule.actions,
                    rule.is_enabled as i32,
                    rule.updated_at,
                    rule.id,
                    pebble_core::now_timestamp(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_rule(&self, id: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute("DELETE FROM rules WHERE id = ?1", params![id])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn test_rule_crud() {
        let store = Store::open_in_memory().unwrap();
        let now = pebble_core::now_timestamp();

        let rule = Rule {
            id: pebble_core::new_id(),
            name: "Auto-archive".to_string(),
            priority: 10,
            conditions: r#"{"from": "noreply@example.com"}"#.to_string(),
            actions: r#"["archive"]"#.to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.insert_rule(&rule).unwrap();

        let rules = store.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "Auto-archive");
        assert!(rules[0].is_enabled);

        // Insert a second rule with higher priority (lower number = higher priority)
        let rule2 = Rule {
            id: pebble_core::new_id(),
            name: "Urgent".to_string(),
            priority: 1,
            conditions: r#"{"subject": "URGENT"}"#.to_string(),
            actions: r#"["star"]"#.to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.insert_rule(&rule2).unwrap();

        let rules = store.list_rules().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "Urgent"); // priority 1 first
        assert_eq!(rules[1].name, "Auto-archive"); // priority 10 second

        // Update
        let mut updated = rule.clone();
        updated.name = "Auto-archive v2".to_string();
        updated.is_enabled = false;
        updated.updated_at = now + 1;
        store.update_rule(&updated).unwrap();

        let rules = store.list_rules().unwrap();
        let found = rules.iter().find(|r| r.id == rule.id).unwrap();
        assert_eq!(found.name, "Auto-archive v2");
        assert!(!found.is_enabled);

        // Delete
        store.delete_rule(&rule.id).unwrap();
        let rules = store.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "Urgent");
    }

    #[test]
    fn fingerprint_changes_when_a_rule_is_edited_with_its_old_timestamp() {
        let store = Store::open_in_memory().unwrap();
        let fingerprint = || store.with_read(rules_fingerprint).unwrap();
        let empty = fingerprint();
        let mut rule = Rule {
            id: "r1".to_string(),
            name: "Archive newsletters".to_string(),
            priority: 1,
            conditions: r#"{"from": "news@example.com"}"#.to_string(),
            actions: r#"["archive"]"#.to_string(),
            is_enabled: true,
            created_at: 1,
            updated_at: 1,
        };
        store.insert_rule(&rule).unwrap();
        let inserted = fingerprint();
        assert_ne!(inserted, empty);

        // The UI saves edits with the rule's existing updated_at.
        rule.is_enabled = false;
        store.update_rule(&rule).unwrap();
        let disabled = fingerprint();
        assert_ne!(disabled, inserted);
        assert!(store.list_rules().unwrap()[0].updated_at > 1);

        rule.conditions = r#"{"from": "other@example.com"}"#.to_string();
        store.update_rule(&rule).unwrap();
        assert_ne!(fingerprint(), disabled);

        store.delete_rule("r1").unwrap();
        assert_eq!(fingerprint(), empty);
    }
}
