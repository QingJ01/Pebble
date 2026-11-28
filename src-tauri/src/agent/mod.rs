//! The assistant's chat (#107, M3): a tool-calling loop over an
//! OpenAI-compatible model, working through [`capabilities`].

pub mod capabilities;
pub mod chat;
pub mod llm;
