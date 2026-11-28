//! What the bridge and the Pebble app say to each other over the local
//! socket (one JSON object per line, one request per connection).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A request from the bridge to Pebble.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BridgeRequest {
    /// Ask the user to approve a new client. Pebble answers when the user
    /// decides (or after a while with `pending`).
    Pair { client: String },
    /// Run a tool with the client's token.
    Call {
        token: String,
        tool: String,
        #[serde(default)]
        args: Value,
    },
}

/// Pebble's answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub ok: bool,
    /// The new token (after an approved pairing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// A message for the person using the MCP client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The user said no to the pairing.
    Denied,
    /// The user hasn't decided yet.
    Pending,
    /// No valid token (never paired, or revoked).
    Unauthorized,
    /// The token is fine but the grant doesn't allow this.
    Forbidden,
    RateLimited,
    /// MCP is turned off in Pebble.
    Disabled,
    /// Bad arguments or unknown tool.
    Invalid,
    NotFound,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Denied => "denied",
            ErrorCode::Pending => "pending",
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::Forbidden => "forbidden",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::Disabled => "disabled",
            ErrorCode::Invalid => "invalid",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Internal => "internal",
        }
    }
}

impl BridgeResponse {
    pub fn ok(result: Value) -> Self {
        Self {
            ok: true,
            result: Some(result),
            ..Default::default()
        }
    }

    pub fn token(token: String) -> Self {
        Self {
            ok: true,
            token: Some(token),
            ..Default::default()
        }
    }

    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            code: Some(code),
            ..Default::default()
        }
    }
}

/// Tool names (MCP clients accept `[A-Za-z0-9_-]`, so no dots).
pub const TOOL_SEARCH: &str = "mail_search";
pub const TOOL_GET_THREAD: &str = "mail_get_thread";
pub const TOOL_PROPOSE_DRAFT: &str = "mail_propose_draft";

/// Access levels, lowest first. Each includes the ones before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Search results: sender, subject, date, folder, ids.
    Metadata,
    /// Plus snippets.
    Summary,
    /// Plus whole threads (`mail_get_thread`).
    Body,
    /// Plus draft proposals (`mail_propose_draft`).
    Drafts,
}

impl Level {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "metadata" => Level::Metadata,
            "summary" => Level::Summary,
            "body" => Level::Body,
            "drafts" => Level::Drafts,
            _ => return None,
        })
    }

    /// The level a tool needs.
    pub fn required_for(tool: &str) -> Option<Self> {
        Some(match tool {
            TOOL_SEARCH => Level::Metadata,
            TOOL_GET_THREAD => Level::Body,
            TOOL_PROPOSE_DRAFT => Level::Drafts,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_levels_round_trip() {
        let call = BridgeRequest::Call {
            token: "t".into(),
            tool: TOOL_SEARCH.into(),
            args: serde_json::json!({"query": "x"}),
        };
        let text = serde_json::to_string(&call).unwrap();
        assert!(text.contains("\"op\":\"call\""));
        assert_eq!(serde_json::from_str::<BridgeRequest>(&text).unwrap(), call);
        assert!(Level::Drafts > Level::Body && Level::Body > Level::Summary);
        assert_eq!(Level::required_for(TOOL_GET_THREAD), Some(Level::Body));
        assert_eq!(Level::required_for("mail_send"), None);
    }
}
