use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TranslateProviderConfig {
    #[serde(rename = "deeplx")]
    DeepLX { endpoint: String },
    #[serde(rename = "deepl")]
    DeepL { api_key: String, use_free_api: bool },
    #[serde(rename = "generic_api")]
    GenericApi {
        endpoint: String,
        api_key: Option<String>,
        source_lang_param: String,
        target_lang_param: String,
        text_param: String,
        result_path: String,
    },
    #[serde(rename = "llm")]
    LLM {
        endpoint: String,
        api_key: String,
        model: String,
        mode: LLMMode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LLMMode {
    Completions,
    Responses,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct TranslateResult {
    pub translated: String,
    pub segments: Vec<BilingualSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct BilingualSegment {
    pub source: String,
    pub target: String,
}
