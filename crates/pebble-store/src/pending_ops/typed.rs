//! Typed view of a `pending_mail_ops` row.
//!
//! On disk a row is `op_type` plus `payload_json`, where the JSON is an
//! envelope:
//!
//! ```json
//! { "provider_account_id": "...", "remote_id": "...", "op": "archive",
//!   "payload": { ...op specific... },
//!   "local_committed": true, "previous_state": { ... },
//!   "remote_succeeded": true, "compensated": "reverted" }
//! ```
//!
//! [`MailOpEnvelope`] reads every shape that has ever been written (bare
//! payloads without an envelope, batch `archive_folder_*` keys, rows without
//! the `local_committed` marker, managed moves without `op`) and writes the
//! same shape back, so rows queued by older builds keep replaying and no
//! schema migration is needed.

use pebble_core::{PebbleError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

fn is_false(value: &bool) -> bool {
    !*value
}

/// `update_flags`. Booleans are always written (as `null` when unchanged)
/// because the merge and revert logic key off their presence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagsOp {
    #[serde(default)]
    pub is_read: Option<bool>,
    #[serde(default)]
    pub is_starred: Option<bool>,
    /// Gmail label edit recorded when the op was queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_labels: Option<Vec<String>>,
    /// IMAP mailbox the message's UID belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_remote_id: Option<String>,
}

impl FlagsOp {
    pub fn has_flag_change(&self) -> bool {
        self.is_read.is_some() || self.is_starred.is_some()
    }
}

/// `archive`, `unarchive`, `restore` and `move_to_folder`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderChangeOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_folder_remote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_folder_remote_id: Option<String>,
    /// Legacy batch-archive spelling of `target_folder_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_folder_id: Option<String>,
    /// Legacy batch-archive spelling of `target_folder_remote_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_folder_remote_id: Option<String>,
    /// Gmail label edit recorded when the op was queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_labels: Option<Vec<String>>,
}

impl FolderChangeOp {
    pub fn target_id(&self) -> Option<&str> {
        self.target_folder_id
            .as_deref()
            .or(self.archive_folder_id.as_deref())
    }

    pub fn target_remote_id(&self) -> Option<&str> {
        self.target_folder_remote_id
            .as_deref()
            .or(self.archive_folder_remote_id.as_deref())
    }

    /// The recorded label edit, when both halves were recorded.
    pub fn labels(&self) -> Option<(&[String], &[String])> {
        match (&self.add_labels, &self.remove_labels) {
            (Some(add), Some(remove)) => Some((add, remove)),
            (Some(add), None) => Some((add, &[])),
            (None, Some(remove)) => Some((&[], remove)),
            (None, None) => None,
        }
    }
}

/// `delete` (to trash) and `delete_permanent`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_folder_remote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trash_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trash_folder_remote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permanent: Option<bool>,
}

/// `managed_move` (Move To). Both folder ids are required at replay time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedMoveOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_folder_remote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_folder_remote_id: Option<String>,
}

/// `send`. Unknown keys are preserved; compose owns this payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SendOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_finalize: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_folder_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A queued mail operation. Serialises adjacently tagged as
/// `{"op": "<op_type>", "payload": {...}}`, the envelope's own keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "payload", rename_all = "snake_case")]
pub enum MailOp {
    UpdateFlags(FlagsOp),
    Archive(FolderChangeOp),
    Unarchive(FolderChangeOp),
    Restore(FolderChangeOp),
    Delete(DeleteOp),
    DeletePermanent(DeleteOp),
    MoveToFolder(FolderChangeOp),
    ManagedMove(ManagedMoveOp),
    Send(SendOp),
}

impl MailOp {
    /// The `op_type` column value.
    pub fn op_type(&self) -> &'static str {
        match self {
            Self::UpdateFlags(_) => "update_flags",
            Self::Archive(_) => "archive",
            Self::Unarchive(_) => "unarchive",
            Self::Restore(_) => "restore",
            Self::Delete(_) => "delete",
            Self::DeletePermanent(_) => "delete_permanent",
            Self::MoveToFolder(_) => "move_to_folder",
            Self::ManagedMove(_) => "managed_move",
            Self::Send(_) => "send",
        }
    }

    /// Decode from the `op_type` column and the inner payload object.
    pub fn from_parts(op_type: &str, payload: &Value) -> Result<Self> {
        let payload = if payload.is_null() {
            Value::Object(Map::new())
        } else {
            payload.clone()
        };
        let tagged = serde_json::json!({ "op": op_type, "payload": payload });
        serde_json::from_value(tagged).map_err(|e| {
            if e.to_string().contains("unknown variant") {
                PebbleError::Internal(format!("Unsupported pending mail op type: {op_type}"))
            } else {
                PebbleError::Internal(format!("Invalid pending op payload: {e}"))
            }
        })
    }

    /// The inner payload object, as stored under `"payload"`.
    pub fn payload(&self) -> Value {
        serde_json::to_value(self)
            .ok()
            .and_then(|mut value| value.get_mut("payload").map(Value::take))
            .unwrap_or_else(|| Value::Object(Map::new()))
    }

    /// Whether the user's change may be committed locally before the remote
    /// acknowledges it when the provider is unreachable. A permanent delete
    /// cannot be reverted locally, so it waits for the server.
    pub fn commits_locally_when_offline(&self) -> bool {
        !matches!(
            self,
            Self::DeletePermanent(_) | Self::ManagedMove(_) | Self::Send(_)
        )
    }
}

/// Local state captured before an optimistic local commit, so an abandoned
/// op can be reverted instead of forcing a resync.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousLocalState {
    #[serde(default)]
    pub is_read: Option<bool>,
    #[serde(default)]
    pub is_starred: Option<bool>,
    #[serde(default)]
    pub is_deleted: Option<bool>,
    #[serde(default)]
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub remote_id: Option<String>,
    #[serde(default)]
    pub folder_ids: Option<Vec<String>>,
}

/// One `pending_mail_ops` row's payload, decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct MailOpEnvelope {
    pub provider_account_id: Option<String>,
    pub remote_id: Option<String>,
    pub op: MailOp,
    /// `Some(true)`: the local change was committed before the remote ack.
    /// `Some(false)`: local state waits for replay. `None`: legacy row.
    pub local_committed: Option<bool>,
    pub previous_state: Option<PreviousLocalState>,
    /// The remote side effect happened; only local finalisation remains.
    pub remote_succeeded: bool,
    /// Compensation marker written when the op was abandoned.
    pub compensated: Option<String>,
}

const ENVELOPE_KEYS: [&str; 8] = [
    "provider_account_id",
    "remote_id",
    "op",
    "payload",
    "local_committed",
    "previous_state",
    "remote_succeeded",
    "compensated",
];

impl MailOpEnvelope {
    pub fn new(account_id: &str, remote_id: &str, op: MailOp) -> Self {
        Self {
            provider_account_id: Some(account_id.to_string()),
            remote_id: Some(remote_id.to_string()),
            op,
            local_committed: None,
            previous_state: None,
            remote_succeeded: false,
            compensated: None,
        }
    }

    /// Mark whether local state is committed ahead of the remote, recording
    /// the pre-change state when it is.
    pub fn with_local_commit(mut self, previous_state: Option<PreviousLocalState>) -> Self {
        self.local_committed = Some(previous_state.is_some());
        self.previous_state = previous_state;
        self
    }

    pub fn op_type(&self) -> &'static str {
        self.op.op_type()
    }

    /// Decode a stored row. Rows without an envelope keep their payload at
    /// the top level.
    pub fn decode(op_type: &str, payload_json: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(payload_json)
            .map_err(|e| PebbleError::Internal(format!("Invalid pending op payload: {e}")))?;
        Self::decode_value(op_type, &value)
    }

    pub fn decode_value(op_type: &str, value: &Value) -> Result<Self> {
        let payload = match value.get("payload") {
            Some(payload) => payload.clone(),
            None => {
                let mut bare = value.as_object().cloned().unwrap_or_default();
                for key in ENVELOPE_KEYS {
                    bare.remove(key);
                }
                Value::Object(bare)
            }
        };
        let string = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        Ok(Self {
            provider_account_id: string("provider_account_id"),
            remote_id: string("remote_id"),
            op: MailOp::from_parts(op_type, &payload)?,
            local_committed: value.get("local_committed").and_then(Value::as_bool),
            previous_state: value
                .get("previous_state")
                .filter(|state| !state.is_null())
                .map(|state| serde_json::from_value(state.clone()))
                .transpose()
                .map_err(|e| PebbleError::Internal(format!("Invalid pending op state: {e}")))?,
            remote_succeeded: value
                .get("remote_succeeded")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            compensated: value.get("compensated").map(|marker| match marker {
                Value::String(marker) => marker.clone(),
                other => other.to_string(),
            }),
        })
    }

    pub fn to_value(&self) -> Value {
        #[derive(Serialize)]
        struct Wire<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            provider_account_id: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            remote_id: &'a Option<String>,
            #[serde(flatten)]
            op: &'a MailOp,
            #[serde(skip_serializing_if = "Option::is_none")]
            local_committed: &'a Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            previous_state: &'a Option<PreviousLocalState>,
            #[serde(skip_serializing_if = "is_false")]
            remote_succeeded: &'a bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            compensated: &'a Option<String>,
        }
        serde_json::to_value(Wire {
            provider_account_id: &self.provider_account_id,
            remote_id: &self.remote_id,
            op: &self.op,
            local_committed: &self.local_committed,
            previous_state: &self.previous_state,
            remote_succeeded: &self.remote_succeeded,
            compensated: &self.compensated,
        })
        .unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Envelopes exactly as earlier builds wrote them.
    fn legacy_fixtures() -> Vec<(&'static str, Value)> {
        vec![
            // messages::queue_pending_remote_op (flags, Gmail, connect failure)
            (
                "update_flags",
                json!({"provider_account_id":"a","remote_id":"r1","op":"update_flags",
                    "payload":{"is_read":true,"is_starred":null,"add_labels":[],"remove_labels":["UNREAD"]},
                    "local_committed":true,
                    "previous_state":{"is_read":false,"is_starred":false,"is_deleted":false,
                        "deleted_at":null,"remote_id":"r1","folder_ids":["f-inbox"]}}),
            ),
            // batch archive (legacy archive_folder_* keys, plain insert)
            (
                "archive",
                json!({"provider_account_id":"a","remote_id":"r1","op":"archive",
                    "payload":{"add_labels":[],"remove_labels":["INBOX"],
                        "archive_folder_id":"f-archive","archive_folder_remote_id":"ARCHIVE"}}),
            ),
            // batch delete after a connection failure
            (
                "delete",
                json!({"provider_account_id":"a","remote_id":"r1","op":"delete","payload":{"trash":true}}),
            ),
            // empty_trash queue_permanent_delete_failure
            (
                "delete_permanent",
                json!({"provider_account_id":"a","remote_id":"7","op":"delete_permanent",
                    "payload":{"source_folder_id":"f-trash","source_folder_remote_id":"Trash","permanent":true}}),
            ),
            // folder_management managed move (no "op" key)
            (
                "managed_move",
                json!({"provider_account_id":"a","remote_id":"5","payload":{"source_folder_id":"source",
                    "target_folder_id":"target","source_folder_remote_id":"Source","target_folder_remote_id":"Target"}}),
            ),
            // send with a remote receipt
            (
                "send",
                json!({"op":"send","payload":{"local_finalize":"move_to_sent","sent_folder_id":"f-sent"},
                    "remote_succeeded":true}),
            ),
            // an abandoned op that was already compensated
            (
                "unarchive",
                json!({"remote_id":"r1","op":"unarchive","payload":{},"local_committed":true,
                    "previous_state":{"folder_ids":["f-archive"]},"compensated":"reverted"}),
            ),
        ]
    }

    #[test]
    fn legacy_rows_decode_and_round_trip_to_the_same_json() {
        for (op_type, fixture) in legacy_fixtures() {
            let decoded = MailOpEnvelope::decode(op_type, &fixture.to_string())
                .unwrap_or_else(|e| panic!("{op_type}: {e}"));
            assert_eq!(decoded.op_type(), op_type);
            let reencoded = decoded.to_value();
            for key in [
                "provider_account_id",
                "remote_id",
                "local_committed",
                "remote_succeeded",
            ] {
                assert_eq!(reencoded.get(key), fixture.get(key), "{op_type}.{key}");
            }
            assert_eq!(
                reencoded.get("compensated"),
                fixture.get("compensated"),
                "{op_type}"
            );
            let again = MailOpEnvelope::decode_value(op_type, &reencoded).unwrap();
            assert_eq!(again, decoded, "{op_type} round trip");
        }
    }

    #[test]
    fn legacy_markers_and_aliases_are_understood() {
        let fixtures = legacy_fixtures();
        let flags = MailOpEnvelope::decode_value("update_flags", &fixtures[0].1).unwrap();
        assert_eq!(flags.local_committed, Some(true));
        assert_eq!(
            flags.previous_state.as_ref().unwrap().folder_ids,
            Some(vec!["f-inbox".to_string()])
        );
        let MailOp::UpdateFlags(op) = &flags.op else {
            panic!("expected flags");
        };
        assert_eq!(op.is_read, Some(true));
        assert_eq!(
            op.remove_labels.as_deref(),
            Some(&["UNREAD".to_string()][..])
        );
        // The `null` flag is written back explicitly.
        assert!(flags.to_value()["payload"]["is_starred"].is_null());
        assert!(flags.to_value()["payload"]
            .as_object()
            .unwrap()
            .contains_key("is_starred"));

        let archive = MailOpEnvelope::decode_value("archive", &fixtures[1].1).unwrap();
        let MailOp::Archive(op) = &archive.op else {
            panic!("expected archive");
        };
        assert_eq!(op.target_id(), Some("f-archive"));
        assert_eq!(op.target_remote_id(), Some("ARCHIVE"));
        assert_eq!(archive.local_committed, None, "legacy rows have no marker");

        let send = MailOpEnvelope::decode_value("send", &fixtures[5].1).unwrap();
        assert!(send.remote_succeeded);
        let compensated = MailOpEnvelope::decode_value("unarchive", &fixtures[6].1).unwrap();
        assert_eq!(compensated.compensated.as_deref(), Some("reverted"));
    }

    #[test]
    fn bare_payloads_without_envelope_and_unknown_ops() {
        let bare = MailOpEnvelope::decode(
            "move_to_folder",
            r#"{"target_folder_id":"t","remote_succeeded":false}"#,
        )
        .unwrap();
        let MailOp::MoveToFolder(op) = bare.op else {
            panic!("expected move");
        };
        assert_eq!(op.target_id(), Some("t"));
        assert!(MailOpEnvelope::decode("archive", "{}").is_ok());
        let unknown = MailOpEnvelope::decode("teleport", "{}").unwrap_err();
        assert!(unknown
            .to_string()
            .contains("Unsupported pending mail op type"));
        assert!(MailOpEnvelope::decode("archive", "not json").is_err());
    }

    #[test]
    fn new_envelopes_record_local_commit_markers() {
        let op = MailOp::Archive(FolderChangeOp {
            target_folder_id: Some("f".into()),
            ..FolderChangeOp::default()
        });
        let queued = MailOpEnvelope::new("a", "r", op.clone()).with_local_commit(None);
        let value = queued.to_value();
        assert_eq!(value["op"], "archive");
        assert_eq!(value["local_committed"], false);
        assert!(value.get("previous_state").is_none());
        let committed =
            MailOpEnvelope::new("a", "r", op).with_local_commit(Some(PreviousLocalState {
                folder_ids: Some(vec!["inbox".into()]),
                ..PreviousLocalState::default()
            }));
        assert_eq!(committed.to_value()["local_committed"], true);
        assert_eq!(
            committed.to_value()["previous_state"]["folder_ids"],
            json!(["inbox"])
        );
        assert!(!MailOp::DeletePermanent(DeleteOp::default()).commits_locally_when_offline());
    }
}
