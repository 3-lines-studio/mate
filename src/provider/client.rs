use super::types::{
    CacheControl, ChatRequest, ModelProfile, ProviderError, ReasoningConfig, StreamEvent,
    StreamOptions, Usage,
};
use crate::message::Role;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct Client {
    base_url: String,
    model: String,
    api_key: String,
    http_client: reqwest::Client,
    debug: bool,
    pub profile: ModelProfile,
}

#[async_trait::async_trait]
pub trait ChatClient: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<mpsc::Receiver<StreamEvent>, ProviderError>;
    fn model(&self) -> &str;
    fn context_window(&self) -> i32;
    fn pricing(&self) -> (f64, f64, f64);

    fn cost_for(&self, usage: &Usage) -> f64 {
        if usage.cost > 0.0 {
            return usage.cost;
        }
        let (in_p, cache_p, out_p) = self.pricing();
        if in_p == 0.0 && cache_p == 0.0 && out_p == 0.0 {
            return 0.0;
        }
        let cached = usage.prompt_cache_hit_tokens.min(usage.prompt_tokens);
        let non_cached = usage.prompt_tokens - cached;
        non_cached as f64 * in_p / 1e6
            + cached as f64 * cache_p / 1e6
            + usage.completion_tokens as f64 * out_p / 1e6
    }
}

impl Client {
    pub fn new(base_url: &str, model: &str, api_key: &str, profile: ModelProfile) -> Self {
        Client {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key: api_key.to_string(),
            http_client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            debug: false,
            profile,
        }
    }

    pub fn set_debug(&mut self, enabled: bool) {
        self.debug = enabled;
    }

    pub fn debug_enabled(&self) -> bool {
        self.debug
    }
}

#[async_trait::async_trait]
impl ChatClient for Client {
    async fn chat(
        &self,
        mut req: ChatRequest,
    ) -> Result<mpsc::Receiver<StreamEvent>, ProviderError> {
        req.model = self.model.clone();
        req.stream = true;
        req.stream_options = Some(StreamOptions {
            include_usage: true,
        });
        if req.max_tokens <= 0 {
            req.max_tokens = self.profile.max_output_tokens;
        }
        apply_profile(&mut req, &self.profile);

        for msg in &mut req.messages {
            msg.tool_duration = String::new();
            if msg.role == Role::Assistant && msg.content.is_empty() && msg.tool_calls.is_empty() {
                // Some providers reject an empty content field on assistant messages.
                msg.content = " ".to_string();
            }
        }

        let body = serde_json::to_vec(&req).map_err(|e| ProviderError {
            status_code: 0,
            body: format!("marshal request: {}", e),
        })?;

        if self.debug {
            eprintln!("chat request body: {}", String::from_utf8_lossy(&body));
        }

        let url = format!("{}/chat/completions", self.base_url);
        let http_req = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("User-Agent", "mate/1.0")
            .body(body);

        let resp =
            match tokio::time::timeout(std::time::Duration::from_secs(60), http_req.send()).await {
                Err(_elapsed) => {
                    log::warn!("request head timeout");
                    return Err(ProviderError {
                        status_code: 0,
                        body: "request head timeout".to_string(),
                    });
                }
                Ok(Err(e)) => {
                    return Err(ProviderError {
                        status_code: 0,
                        body: format!("http request: {}", e),
                    });
                }
                Ok(Ok(resp)) => resp,
            };

        if resp.status().as_u16() != 200 {
            let status = resp.status().as_u16();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(ProviderError {
                status_code: status,
                body: resp_body,
            });
        }

        let (tx, rx) = mpsc::channel(10);
        let debug = self.debug;

        tokio::spawn(async move {
            super::stream::read_stream(resp, tx, debug).await;
        });

        Ok(rx)
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn context_window(&self) -> i32 {
        self.profile.context_window
    }

    fn pricing(&self) -> (f64, f64, f64) {
        (
            self.profile.input_price,
            self.profile.cached_input_price,
            self.profile.output_price,
        )
    }
}

pub fn apply_profile(req: &mut ChatRequest, profile: &ModelProfile) {
    if profile.open_router {
        if !profile.reasoning_effort.is_empty() {
            req.reasoning = Some(ReasoningConfig {
                effort: profile.reasoning_effort.clone(),
                max_tokens: 0,
                exclude: None,
                enabled: None,
            });
        }
    } else {
        if !profile.reasoning_effort.is_empty() {
            req.reasoning_effort = profile.reasoning_effort.clone();
        }
    }

    if profile.prompt_cache {
        req.cache_control = Some(CacheControl {
            cc_type: "ephemeral".to_string(),
            ttl: profile.cache_ttl.clone(),
        });
    }
}
