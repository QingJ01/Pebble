use pebble_core::FolderRole;

#[derive(Debug, PartialEq, Eq)]
pub struct GmailLabelDelta {
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
}

pub fn gmail_move_label_delta(
    source_remote_id: Option<&str>,
    target_remote_id: &str,
    target_role: Option<FolderRole>,
) -> GmailLabelDelta {
    if target_role == Some(FolderRole::Spam) {
        return GmailLabelDelta {
            add_labels: vec!["SPAM".to_string()],
            remove_labels: vec!["INBOX".to_string()],
        };
    }

    if target_role == Some(FolderRole::Archive) {
        return GmailLabelDelta {
            add_labels: vec![],
            remove_labels: vec!["INBOX".to_string()],
        };
    }

    let target = match target_role {
        _ => valid_gmail_label(target_remote_id),
    };
    let source = source_remote_id.and_then(valid_gmail_label);

    let mut add_labels = Vec::new();
    if let Some(label) = target {
        push_unique(&mut add_labels, label);
    }

    let mut remove_labels = Vec::new();
    match source {
        Some(label) if Some(label) != target => push_unique(&mut remove_labels, label),
        Some(_) => {}
        None => push_unique(&mut remove_labels, "INBOX"),
    }

    remove_labels.retain(|label| !add_labels.contains(label));

    GmailLabelDelta {
        add_labels,
        remove_labels,
    }
}

fn valid_gmail_label(label: &str) -> Option<&str> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.starts_with("__local_") {
        None
    } else {
        Some(trimmed)
    }
}

fn push_unique(labels: &mut Vec<String>, label: &str) {
    if !labels.iter().any(|existing| existing == label) {
        labels.push(label.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_move_from_inbox_to_label_adds_target_and_removes_inbox() {
        let delta = gmail_move_label_delta(Some("INBOX"), "Label_B", None);

        assert_eq!(
            delta,
            GmailLabelDelta {
                add_labels: vec!["Label_B".to_string()],
                remove_labels: vec!["INBOX".to_string()],
            }
        );
    }

    #[test]
    fn gmail_move_from_label_to_label_removes_source_label() {
        let delta = gmail_move_label_delta(Some("Label_A"), "Label_B", None);

        assert_eq!(
            delta,
            GmailLabelDelta {
                add_labels: vec!["Label_B".to_string()],
                remove_labels: vec!["Label_A".to_string()],
            }
        );
    }

    #[test]
    fn gmail_move_from_label_to_spam_adds_spam_and_removes_source_label() {
        let delta = gmail_move_label_delta(Some("Label_A"), "ignored", Some(FolderRole::Spam));

        assert_eq!(
            delta,
            GmailLabelDelta {
                add_labels: vec!["SPAM".to_string()],
                remove_labels: vec!["INBOX".to_string()],
            }
        );
    }

    #[test]
    fn gmail_move_from_unknown_source_falls_back_to_inbox_removal() {
        let delta = gmail_move_label_delta(None, "Label_B", None);

        assert_eq!(
            delta,
            GmailLabelDelta {
                add_labels: vec!["Label_B".to_string()],
                remove_labels: vec!["INBOX".to_string()],
            }
        );
    }

    #[test]
    fn gmail_move_to_archive_removes_source_without_adding_label() {
        let delta = gmail_move_label_delta(Some("Label_A"), "", Some(FolderRole::Archive));

        assert_eq!(
            delta,
            GmailLabelDelta {
                add_labels: vec![],
                remove_labels: vec!["INBOX".to_string()],
            }
        );
    }

    #[test]
    fn gmail_move_to_archive_without_source_falls_back_to_inbox() {
        let delta = gmail_move_label_delta(None, "", Some(FolderRole::Archive));

        assert_eq!(
            delta,
            GmailLabelDelta {
                add_labels: vec![],
                remove_labels: vec!["INBOX".to_string()],
            }
        );
    }
}
