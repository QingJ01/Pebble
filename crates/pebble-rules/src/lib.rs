pub mod types;
pub mod matcher;

use pebble_core::{Message, Rule};
use types::{RuleAction, RuleConditionSet};
use matcher::evaluate_conditions;

pub struct RuleEngine {
    rules: Vec<(RuleConditionSet, Vec<RuleAction>)>,
}

impl RuleEngine {
    pub fn new(rules: &[Rule]) -> Self {
        let mut parsed: Vec<(i32, RuleConditionSet, Vec<RuleAction>)> = rules.iter()
            .filter(|r| r.is_enabled)
            .filter_map(|r| {
                let conditions: RuleConditionSet = match serde_json::from_str(&r.conditions) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(rule_name = %r.name, error = %e, "skipping rule: invalid conditions JSON");
                        return None;
                    }
                };
                let actions: Vec<RuleAction> = match serde_json::from_str(&r.actions) {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::warn!(rule_name = %r.name, error = %e, "skipping rule: invalid actions JSON");
                        return None;
                    }
                };
                Some((r.priority, conditions, actions))
            })
            .collect();
        parsed.sort_by_key(|(p, _, _)| *p);
        Self {
            rules: parsed.into_iter().map(|(_, c, a)| (c, a)).collect(),
        }
    }

    pub fn evaluate(&self, message: &Message) -> Vec<RuleAction> {
        let mut actions = Vec::new();
        for (conditions, rule_actions) in &self.rules {
            if evaluate_conditions(message, conditions) {
                actions.extend(rule_actions.iter().cloned());
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_message(from: &str, subject: &str, body: &str) -> Message {
        Message {
            id: String::new(),
            account_id: String::new(),
            remote_id: String::new(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: subject.to_string(),
            snippet: String::new(),
            from_address: from.to_string(),
            from_name: String::new(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: body.to_string(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 0,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_test_rule(name: &str, priority: i32, conditions_json: &str, actions_json: &str, enabled: bool) -> Rule {
        Rule {
            id: "rule-1".to_string(),
            name: name.to_string(),
            priority,
            conditions: conditions_json.to_string(),
            actions: actions_json.to_string(),
            is_enabled: enabled,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn test_rule_engine_evaluate() {
        let conditions = r#"{"operator":"and","conditions":[{"field":"from","op":"contains","value":"newsletter"}]}"#;
        let actions = r#"[{"type":"AddLabel","value":"newsletters"},{"type":"MarkRead"}]"#;
        let rule = make_test_rule("Label newsletters", 1, conditions, actions, true);

        let engine = RuleEngine::new(&[rule]);
        let msg = make_test_message("newsletter@example.com", "Weekly Update", "Content");
        let result = engine.evaluate(&msg);

        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_disabled_rules_skipped() {
        let conditions = r#"{"operator":"and","conditions":[{"field":"from","op":"contains","value":"newsletter"}]}"#;
        let actions = r#"[{"type":"AddLabel","value":"newsletters"}]"#;
        let rule = make_test_rule("Disabled rule", 1, conditions, actions, false);

        let engine = RuleEngine::new(&[rule]);
        let msg = make_test_message("newsletter@example.com", "Weekly Update", "Content");
        let result = engine.evaluate(&msg);

        assert!(result.is_empty());
    }
}
