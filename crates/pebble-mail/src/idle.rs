use pebble_core::Result;
use tracing::{debug, warn};

/// Result of checking for new mail activity.
#[derive(Debug)]
pub enum IdleEvent {
    /// New mail may be available.
    NewMail,
    /// The wait timed out with no changes.
    Timeout,
    /// An error occurred during the check.
    Error(String),
}

/// Recommended maximum time to remain in one IMAP IDLE command.
///
/// RFC 2177 recommends re-issuing IDLE before 30 minutes, so cap the wait at
/// 29 minutes and enforce a 60-second floor to avoid tight reconnect loops.
pub fn recommended_idle_wait_secs(configured_secs: u64) -> u64 {
    configured_secs.clamp(60, 1740)
}

fn observe_highest_uid(last_exists: &mut Option<u32>, current_highest_uid: u32) -> IdleEvent {
    match *last_exists {
        None => {
            *last_exists = Some(current_highest_uid);
            IdleEvent::Timeout
        }
        Some(previous_highest_uid) if current_highest_uid > previous_highest_uid => {
            debug!(
                "Mailbox highest UID advanced: {} -> {}",
                previous_highest_uid, current_highest_uid
            );
            IdleEvent::NewMail
        }
        Some(_) => IdleEvent::Timeout,
    }
}

/// Check if a mailbox has new messages by comparing highest UID.
///
/// This is a lightweight fallback for servers that do not advertise the
/// IDLE capability. It does a quick UID SEARCH ALL and compares the highest
/// server UID against the last trusted local high-water mark.
pub async fn check_for_changes(
    provider: &super::imap::ImapProvider,
    mailbox: &str,
    last_exists: &mut Option<u32>,
) -> Result<IdleEvent> {
    match provider.fetch_all_uids(mailbox).await {
        Ok(uids) => {
            let current_highest_uid = uids.into_iter().max().unwrap_or(0);
            Ok(observe_highest_uid(last_exists, current_highest_uid))
        }
        Err(e) => Ok(IdleEvent::Error(e.to_string())),
    }
}

/// Check for changes using native IDLE if supported, falling back to
/// UID-count comparison when IDLE is unavailable or fails.
pub async fn check_for_changes_with_idle(
    provider: &super::imap::ImapProvider,
    mailbox: &str,
    last_exists: &mut Option<u32>,
    use_idle: bool,
) -> Result<IdleEvent> {
    if use_idle {
        // Use native IMAP IDLE with a bounded timeout. Callers with a dedicated
        // watcher can pass a longer configured value through `idle_wait`
        // directly; this helper keeps its historical 60-second behavior.
        let timeout = std::time::Duration::from_secs(recommended_idle_wait_secs(60));
        match provider.idle_wait(mailbox, timeout).await {
            Ok(event) => Ok(event),
            Err(e) => {
                warn!("IDLE failed, attempting reconnect before fallback poll: {e}");
                // The IDLE failure may have left the session as None (e.g.
                // when done() fails to recover it). Reconnect so the
                // fallback poll has a usable session.
                if let Err(reconn_err) = provider.connect().await {
                    warn!("Reconnect after IDLE failure also failed: {reconn_err}");
                    return Ok(IdleEvent::Error(format!(
                        "IDLE failed and reconnect failed: {e}; {reconn_err}"
                    )));
                }
                check_for_changes(provider, mailbox, last_exists).await
            }
        }
    } else {
        check_for_changes(provider, mailbox, last_exists).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_highest_uid_observation_seeds_empty_baseline() {
        let mut last_exists = None;

        let event = observe_highest_uid(&mut last_exists, 0);

        assert!(matches!(event, IdleEvent::Timeout));
        assert_eq!(last_exists, Some(0));
    }

    #[test]
    fn new_mail_after_empty_local_uid_baseline_is_detected() {
        let mut last_exists = Some(0);

        let event = observe_highest_uid(&mut last_exists, 1);

        assert!(matches!(event, IdleEvent::NewMail));
        assert_eq!(last_exists, Some(0));
    }

    #[test]
    fn same_count_replacement_is_detected_by_highest_uid_without_advancing_baseline() {
        let mut last_exists = Some(10);

        let event = observe_highest_uid(&mut last_exists, 11);

        assert!(matches!(event, IdleEvent::NewMail));
        assert_eq!(last_exists, Some(10));
    }

    #[test]
    fn first_unknown_non_empty_observation_only_seeds_baseline() {
        let mut last_exists = None;

        let event = observe_highest_uid(&mut last_exists, 4);

        assert!(matches!(event, IdleEvent::Timeout));
        assert_eq!(last_exists, Some(4));
    }

    #[test]
    fn test_idle_event_variants() {
        let new_mail = IdleEvent::NewMail;
        assert!(matches!(new_mail, IdleEvent::NewMail));

        let timeout = IdleEvent::Timeout;
        assert!(matches!(timeout, IdleEvent::Timeout));

        let error = IdleEvent::Error("test error".to_string());
        assert!(matches!(error, IdleEvent::Error(ref s) if s == "test error"));
    }

    #[test]
    fn test_idle_event_debug() {
        let event = IdleEvent::NewMail;
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("NewMail"));
    }

    #[test]
    fn idle_timeout_is_shorter_than_server_disconnect_window() {
        assert_eq!(recommended_idle_wait_secs(1), 60);
        assert_eq!(recommended_idle_wait_secs(120), 120);
        assert_eq!(recommended_idle_wait_secs(1800), 1740);
        assert_eq!(recommended_idle_wait_secs(3600), 1740);
    }
}
