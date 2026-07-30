pub use crate::message::ReasoningDetail;
use crate::message::{Message, ToolDef};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub max_tokens: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

fn is_zero_i32(n: &i32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cc_type: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ttl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
    pub stream: bool,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub max_tokens: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning_effort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

#[derive(Debug, Clone)]
pub struct ModelProfile {
    pub context_window: i32,
    pub max_output_tokens: i32,
    pub reasoning_effort: String,
    pub open_router: bool,
    pub input_price: f64,
    pub cached_input_price: f64,
    pub output_price: f64,
    pub prompt_cache: bool,
    pub cache_ttl: String,
}

impl Default for ModelProfile {
    fn default() -> Self {
        Self {
            context_window: 0,
            max_output_tokens: 0,
            reasoning_effort: String::new(),
            open_router: false,
            input_price: 0.0,
            cached_input_price: 0.0,
            output_price: 0.0,
            prompt_cache: false,
            cache_ttl: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTokensDetails {
    pub cached_tokens: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub cache_write_tokens: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionTokensDetails {
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub reasoning_tokens: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
    #[serde(default)]
    pub prompt_cache_hit_tokens: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub cost: f64,
}

fn is_zero_f64(n: &f64) -> bool {
    *n == 0.0
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta { delta: String },
    ReasoningDelta { delta: String },
    ReasoningDetails { details: Vec<ReasoningDetail> },
    ToolCall { call: StreamToolCall },
    Usage { usage: Usage },
    FinishReason { reason: String },
    Error { error: ProviderError },
}

#[derive(Debug, Clone)]
pub struct StreamToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ProviderError {
    pub status_code: u16,
    pub body: String,
}

impl ProviderError {
    pub fn retryable(&self) -> bool {
        self.status_code == 0 || self.status_code == 429 || self.status_code >= 500
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unexpected status {}: {}", self.status_code, self.body)
    }
}

impl std::error::Error for ProviderError {}
