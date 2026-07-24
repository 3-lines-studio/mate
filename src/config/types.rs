use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub base_url: String,
    #[serde(default)]
    pub open_router: bool,
    #[serde(skip)]
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub provider: String,
    pub name: String,
    #[serde(default)]
    pub context_window: i32,
    #[serde(default)]
    pub max_output_tokens: i32,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default)]
    pub input_price: f64,
    #[serde(default)]
    pub cached_input_price: f64,
    #[serde(default)]
    pub output_price: f64,
    #[serde(default)]
    pub prompt_cache: bool,
    #[serde(default)]
    pub cache_ttl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_all_tools")]
    pub tools: Vec<String>,
    #[serde(default)]
    pub interfaces: Vec<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub compaction_model: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            tools: default_all_tools(),
            interfaces: vec![],
            prompt: String::new(),
            compaction_model: String::new(),
        }
    }
}

fn default_all_tools() -> Vec<String> {
    vec!["*".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentConfig {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionConfig {
    #[serde(default)]
    pub dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlackConfig {
    #[serde(default)]
    #[serde(skip_serializing)]
    pub bot_token: String,
    #[serde(default)]
    #[serde(skip_serializing)]
    pub app_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    #[serde(default)]
    #[serde(skip_serializing)]
    pub bot_token: String,
    #[serde(default)]
    pub allowed_users: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduledJob {
    pub cron: String,
    pub prompt: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduleConfig {
    #[serde(default)]
    pub jobs: Vec<ScheduledJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TUIConfig {
    #[serde(default)]
    pub tools_expanded: bool,
    #[serde(default)]
    pub show_thinking: bool,
    #[serde(default = "default_show_subagent_calls")]
    pub show_subagent_calls: bool,
}

fn default_show_subagent_calls() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub subagents: Vec<SubagentConfig>,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub services: HashMap<String, HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub tui: TUIConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Secrets {
    #[serde(default)]
    pub providers: HashMap<String, String>,
    #[serde(default)]
    pub slack: SlackSecrets,
    #[serde(default)]
    pub telegram: TelegramSecrets,
    #[serde(default)]
    pub services: HashMap<String, HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlackSecrets {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub app_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramSecrets {
    #[serde(default)]
    pub bot_token: String,
}

impl Config {
    pub fn merge(&mut self, secrets: Secrets) {
        for provider in &mut self.providers {
            if let Some(key) = secrets.providers.get(&provider.id) {
                provider.api_key = key.clone();
            }
        }
        if !secrets.slack.bot_token.is_empty() {
            self.slack.bot_token = secrets.slack.bot_token;
        }
        if !secrets.slack.app_token.is_empty() {
            self.slack.app_token = secrets.slack.app_token;
        }
        if !secrets.telegram.bot_token.is_empty() {
            self.telegram.bot_token = secrets.telegram.bot_token;
        }
        if !secrets.services.is_empty() {
            self.services = secrets.services;
        }
    }
}
