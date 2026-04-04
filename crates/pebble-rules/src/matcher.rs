use pebble_core::Message;
use crate::types::*;

pub fn evaluate_condition(msg: &Message, condition: &RuleCondition) -> bool {
    let field_value = match condition.field {
        ConditionField::From => &msg.from_address,
        ConditionField::To => {
            let matches_any = msg.to_list.iter().any(|a| match_op(&a.address, &condition.op, &condition.value));
            // For NotContains, we want ALL addresses to not contain (none contain)
            if matches!(condition.op, ConditionOp::NotContains) {
                return msg.to_list.iter().all(|a| match_op(&a.address, &condition.op, &condition.value));
            }
            return matches_any;
        }
        ConditionField::Subject => &msg.subject,
        ConditionField::Body => &msg.body_text,
        ConditionField::HasAttachment => {
            let expected = condition.value.to_lowercase() == "true";
            return msg.has_attachments == expected;
        }
        ConditionField::Domain => {
            let domain = msg.from_address.split('@').nth(1).unwrap_or("");
            return match_op(domain, &condition.op, &condition.value);
        }
    };
    match_op(field_value, &condition.op, &condition.value)
}

fn match_op(field_value: &str, op: &ConditionOp, value: &str) -> bool {
    let fv = field_value.to_lowercase();
    let v = value.to_lowercase();
    match op {
        ConditionOp::Contains => fv.contains(&v),
        ConditionOp::NotContains => !fv.contains(&v),
        ConditionOp::Equals => fv == v,
        ConditionOp::StartsWith => fv.starts_with(&v),
        ConditionOp::EndsWith => fv.ends_with(&v),
    }
}

pub fn evaluate_conditions(msg: &Message, conditions: &RuleConditionSet) -> bool {
    match conditions.operator {
        LogicalOp::And => conditions.conditions.iter().all(|c| evaluate_condition(msg, c)),
        LogicalOp::Or => conditions.conditions.iter().any(|c| evaluate_condition(msg, c)),
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

    #[test]
    fn test_contains_match() {
        let msg = make_test_message("newsletter@example.com", "Weekly Update", "Hello");
        let condition = RuleCondition {
            field: ConditionField::From,
            op: ConditionOp::Contains,
            value: "newsletter".to_string(),
        };
        assert!(evaluate_condition(&msg, &condition));
    }

    #[test]
    fn test_not_contains() {
        let msg = make_test_message("user@example.com", "Hello World", "Body text");
        let condition = RuleCondition {
            field: ConditionField::Subject,
            op: ConditionOp::NotContains,
            value: "spam".to_string(),
        };
        assert!(evaluate_condition(&msg, &condition));
    }

    #[test]
    fn test_domain_match() {
        let msg = make_test_message("alice@company.com", "Meeting", "Let's meet");
        let condition = RuleCondition {
            field: ConditionField::Domain,
            op: ConditionOp::Equals,
            value: "company.com".to_string(),
        };
        assert!(evaluate_condition(&msg, &condition));
    }

    #[test]
    fn test_and_conditions() {
        let msg = make_test_message("newsletter@company.com", "Weekly Report", "Content");
        let conditions = RuleConditionSet {
            operator: LogicalOp::And,
            conditions: vec![
                RuleCondition {
                    field: ConditionField::From,
                    op: ConditionOp::Contains,
                    value: "newsletter".to_string(),
                },
                RuleCondition {
                    field: ConditionField::Subject,
                    op: ConditionOp::Contains,
                    value: "weekly".to_string(),
                },
            ],
        };
        assert!(evaluate_conditions(&msg, &conditions));
    }

    #[test]
    fn test_or_conditions() {
        let msg = make_test_message("random@other.com", "Weekly Report", "Content");
        let conditions = RuleConditionSet {
            operator: LogicalOp::Or,
            conditions: vec![
                RuleCondition {
                    field: ConditionField::From,
                    op: ConditionOp::Contains,
                    value: "newsletter".to_string(),
                },
                RuleCondition {
                    field: ConditionField::Subject,
                    op: ConditionOp::Contains,
                    value: "weekly".to_string(),
                },
            ],
        };
        assert!(evaluate_conditions(&msg, &conditions));
    }
}
