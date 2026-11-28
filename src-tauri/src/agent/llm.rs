//! A minimal OpenAI-compatible Chat Completions client with tool calls.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn tool(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(id.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
    pub function: FunctionCall,
}

fn function_type() -> String {
    "function".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON text. Some servers send an object; it is normalised to text.
    #[serde(deserialize_with = "arguments_as_text")]
    pub arguments: String,
}

fn arguments_as_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::String(text) => text,
        Value::Null => "{}".into(),
        other => other.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmError {
    pub status: Option<u16>,
    pub message: String,
}

impl LlmError {
    /// The server rejected the request because of `tools` (no function calling).
    pub fn tools_unsupported(&self) -> bool {
        let text = self.message.to_lowercase();
        matches!(self.status, Some(400 | 404 | 422 | 501))
            && (text.contains("tool") || text.contains("function"))
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(f, "HTTP {status}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

pub type LlmFuture<'a> = Pin<Box<dyn Future<Output = Result<ChatMessage, LlmError>> + Send + 'a>>;

/// A chat model. `tools` is `None` to ask for a plain answer.
pub trait Llm: Send + Sync {
    fn complete<'a>(
        &'a self,
        messages: &'a [ChatMessage],
        tools: Option<&'a Value>,
    ) -> LlmFuture<'a>;
}

pub struct OpenAiCompatible {
    pub client: reqwest::Client,
    pub url: String,
    pub model: String,
    pub api_key: String,
}

/// Parse a Chat Completions response body into the assistant message.
pub fn parse_completion(body: &Value) -> Result<ChatMessage, LlmError> {
    let message = body.pointer("/choices/0/message").ok_or_else(|| LlmError {
        status: None,
        message: "The model returned no message".into(),
    })?;
    let mut parsed: ChatMessage =
        serde_json::from_value(message.clone()).map_err(|e| LlmError {
            status: None,
            message: format!("Unexpected reply from the model: {e}"),
        })?;
    parsed.role = "assistant".into();
    if parsed
        .tool_calls
        .as_ref()
        .is_some_and(|calls| calls.is_empty())
    {
        parsed.tool_calls = None;
    }
    Ok(parsed)
}

impl Llm for OpenAiCompatible {
    fn complete<'a>(
        &'a self,
        messages: &'a [ChatMessage],
        tools: Option<&'a Value>,
    ) -> LlmFuture<'a> {
        Box::pin(async move {
            let mut body = serde_json::json!({
                "model": self.model,
                "messages": messages,
                "temperature": 0.2,
            });
            if let Some(tools) = tools {
                body["tools"] = tools.clone();
                body["tool_choice"] = "auto".into();
            }
            let mut request = self
                .client
                .post(&self.url)
                .timeout(Duration::from_secs(90))
                .json(&body);
            if !self.api_key.is_empty() {
                request = request.bearer_auth(&self.api_key);
            }
            let response = request.send().await.map_err(|e| LlmError {
                status: None,
                message: if e.is_timeout() {
                    "The model didn't answer in time".into()
                } else {
                    e.to_string()
                },
            })?;
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(LlmError {
                    status: Some(status.as_u16()),
                    message: text.chars().take(300).collect(),
                });
            }
            let json: Value = response.json().await.map_err(|e| LlmError {
                status: None,
                message: format!("Unreadable reply from the model: {e}"),
            })?;
            parse_completion(&json)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_tool_calls_in_both_argument_styles() {
        let text = parse_completion(
            &serde_json::json!({"choices":[{"message":{"role":"assistant","content":"Hi"}}]}),
        )
        .unwrap();
        assert_eq!(text.content.as_deref(), Some("Hi"));
        assert!(text.tool_calls.is_none());

        let call = parse_completion(&serde_json::json!({"choices":[{"message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"c1","type":"function","function":{"name":"search_mail","arguments":"{\"query\":\"x\"}"}}]}}]})).unwrap();
        assert_eq!(
            call.tool_calls.as_ref().unwrap()[0].function.arguments,
            r#"{"query":"x"}"#
        );

        // Ollama-style: arguments as an object, no `type`.
        let ollama = parse_completion(&serde_json::json!({"choices":[{"message":{"role":"assistant","content":"",
            "tool_calls":[{"id":"c2","function":{"name":"read_thread","arguments":{"message_id":"m1"}}}]}}]})).unwrap();
        let call = &ollama.tool_calls.unwrap()[0];
        assert_eq!(call.kind, "function");
        assert_eq!(call.function.arguments, r#"{"message_id":"m1"}"#);

        assert!(parse_completion(&serde_json::json!({"error": "x"})).is_err());
    }

    #[test]
    fn recognises_servers_without_function_calling() {
        let no_tools = LlmError {
            status: Some(400),
            message: "model does not support tools".into(),
        };
        assert!(no_tools.tools_unsupported());
        let auth = LlmError {
            status: Some(401),
            message: "bad key".into(),
        };
        assert!(!auth.tools_unsupported());
    }

    #[test]
    fn serialises_tool_results_for_the_api() {
        let json = serde_json::to_value(ChatMessage::tool("c1", "{}")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"role":"tool","content":"{}","tool_call_id":"c1"})
        );
    }
}
