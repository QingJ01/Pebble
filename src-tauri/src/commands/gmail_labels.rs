//! Gmail label rules now live with the Gmail `RemoteMailOps` implementation;
//! re-exported here for the command modules that build queued ops.
#[cfg(test)]
use pebble_core::traits::LabelDelta as GmailLabelDelta;
pub use pebble_mail::provider::mutations::gmail_move_label_delta;

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::FolderRole;

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
