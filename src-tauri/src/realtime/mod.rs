use serde::Serialize;

pub use pebble_mail::SyncTrigger;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeMode {
    Realtime,
    Polling,
    Backoff,
    Offline,
    AuthRequired,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeStatusPayload {
    pub account_id: String,
    pub mode: RealtimeMode,
    pub provider: String,
    pub last_success_at: Option<i64>,
    pub next_retry_at: Option<i64>,
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realtime_status_serializes_as_frontend_contract() {
        let payload = RealtimeStatusPayload {
            account_id: "account-1".to_string(),
            mode: RealtimeMode::Realtime,
            provider: "imap".to_string(),
            last_success_at: Some(1_700_000_000),
            next_retry_at: None,
            message: None,
        };

        let json = serde_json::to_value(payload).unwrap();

        assert_eq!(json["account_id"], "account-1");
        assert_eq!(json["mode"], "realtime");
        assert_eq!(json["provider"], "imap");
    }
}
