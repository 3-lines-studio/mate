use crate::core::session_manager::SessionManager;
use crate::core::{Deps, Interface};
use crate::integration::{self, ActivePrompt, StreamingBackend};
use crate::markdown::telegram::markdown_to_telegram;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TELEGRAM_TEXT_LIMIT: usize = 4000;
const POLL_TIMEOUT_SECS: u64 = 60;

#[derive(Deserialize)]
struct TgResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct TgUpdate {
    update_id: i64,
    message: Option<TgMessage>,
}

#[derive(Deserialize)]
struct TgMessage {
    message_id: i64,
    chat: TgChat,
    text: Option<String>,
    from: Option<TgUser>,
}

#[derive(Deserialize)]
struct TgChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(Deserialize)]
struct TgUser {
    id: i64,
}

fn api_url(token: &str, method: &str) -> String {
    format!("https://api.telegram.org/bot{}/{}", token, method)
}

async fn tg_get_updates(
    client: &reqwest::Client,
    token: &str,
    offset: i64,
) -> Result<Vec<TgUpdate>, String> {
    let resp: TgResponse<Vec<TgUpdate>> = client
        .post(api_url(token, "getUpdates"))
        .json(&serde_json::json!({"offset": offset, "timeout": POLL_TIMEOUT_SECS}))
        .timeout(Duration::from_secs(POLL_TIMEOUT_SECS + 30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok {
        return Err(resp.description.unwrap_or_else(|| "unknown error".into()));
    }
    Ok(resp.result.unwrap_or_default())
}

async fn tg_send_message(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<i64, String> {
    let mut body = serde_json::json!({"chat_id": chat_id, "text": text});
    if let Some(pm) = parse_mode {
        body["parse_mode"] = serde_json::Value::String(pm.to_string());
    }
    let resp: TgResponse<TgMessage> = client
        .post(api_url(token, "sendMessage"))
        .json(&body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok {
        return Err(resp.description.unwrap_or_else(|| "unknown error".into()));
    }
    Ok(resp.result.map(|m| m.message_id).unwrap_or(0))
}

async fn tg_edit_message_text(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<(), String> {
    let mut body = serde_json::json!({"chat_id": chat_id, "message_id": message_id, "text": text});
    if let Some(pm) = parse_mode {
        body["parse_mode"] = serde_json::Value::String(pm.to_string());
    }
    let resp: TgResponse<serde_json::Value> = client
        .post(api_url(token, "editMessageText"))
        .json(&body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok {
        return Err(resp.description.unwrap_or_else(|| "unknown error".into()));
    }
    Ok(())
}

struct TgStreaming {
    client: reqwest::Client,
    token: String,
    chat_id: i64,
}

#[async_trait]
impl StreamingBackend for TgStreaming {
    type MsgId = i64;

    fn flush_interval(&self) -> Duration {
        Duration::from_secs(3)
    }

    fn text_limit(&self) -> usize {
        TELEGRAM_TEXT_LIMIT
    }

    fn markdown(&self, text: &str) -> String {
        markdown_to_telegram(text)
    }

    async fn send_thinking(&self) -> Result<i64, String> {
        tg_send_message(&self.client, &self.token, self.chat_id, "...", None).await
    }

    async fn edit_message(&self, msg_id: &i64, text: &str) -> Result<(), String> {
        tg_edit_message_text(
            &self.client,
            &self.token,
            self.chat_id,
            *msg_id,
            text,
            Some("MarkdownV2"),
        )
        .await
    }

    async fn post_message(&self, text: &str) -> Result<i64, String> {
        tg_send_message(
            &self.client,
            &self.token,
            self.chat_id,
            text,
            Some("MarkdownV2"),
        )
        .await
    }
}

struct BotInner {
    client: reqwest::Client,
    token: String,
    manager: Arc<Mutex<SessionManager>>,
    allowed_users: HashSet<i64>,
    active_prompts: Mutex<HashMap<i64, ActivePrompt>>,
    current_keys: Mutex<HashMap<i64, String>>,
}

impl BotInner {
    async fn run_loop(self: Arc<Self>) {
        let mut offset: i64 = 0;
        loop {
            match tg_get_updates(&self.client, &self.token, offset).await {
                Ok(updates) => {
                    for update in updates {
                        offset = update.update_id + 1;
                        if let Some(msg) = update.message {
                            let bot = self.clone();
                            tokio::spawn(async move {
                                bot.handle_message(msg).await;
                            });
                        }
                    }
                }
                Err(e) => {
                    log::error!("telegram getUpdates: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn handle_message(&self, msg: TgMessage) {
        let chat_id = msg.chat.id;

        if msg.chat.chat_type != "private" {
            return;
        }

        if !self.allowed_users.is_empty()
            && let Some(ref from) = msg.from
            && !self.allowed_users.contains(&from.id)
        {
            let _ =
                tg_send_message(&self.client, &self.token, chat_id, "Access denied", None).await;
            return;
        }

        let text = match msg.text.as_deref() {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => return,
        };

        if text == "/chatid" {
            let _ = tg_send_message(
                &self.client,
                &self.token,
                chat_id,
                &chat_id.to_string(),
                None,
            )
            .await;
            return;
        }

        if text == "/new" {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let new_key = format!("{}:{}", chat_id, nanos);
            {
                let mut keys = self.current_keys.lock().unwrap();
                keys.insert(chat_id, new_key.clone());
            }
            let reply = format!("New session started: {}", new_key);
            let _ = tg_send_message(&self.client, &self.token, chat_id, &reply, None).await;
            return;
        }

        let session_key = self.session_key(chat_id);
        let backend = TgStreaming {
            client: self.client.clone(),
            token: self.token.clone(),
            chat_id,
        };
        integration::process_prompt(
            &backend,
            &self.manager,
            &self.active_prompts,
            chat_id,
            &session_key,
            &text,
        )
        .await;
    }

    fn session_key(&self, chat_id: i64) -> String {
        let keys = self.current_keys.lock().unwrap();
        keys.get(&chat_id)
            .cloned()
            .unwrap_or_else(|| chat_id.to_string())
    }
}

pub struct BotAdapter;

impl Interface for BotAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn run(&self, deps: Deps) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token = deps.config.telegram.bot_token.clone();
        if token.is_empty() {
            return Err("telegram bot_token is required".into());
        }

        let key_store_path = format!("{}/chat_sessions.json", deps.store.dir());
        let manager = deps.new_session_manager(&key_store_path)?;

        let allowed_users: HashSet<i64> =
            deps.config.telegram.allowed_users.iter().cloned().collect();

        let bot = Arc::new(BotInner {
            client: reqwest::Client::new(),
            token,
            manager: Arc::new(Mutex::new(manager)),
            allowed_users,
            active_prompts: Mutex::new(HashMap::new()),
            current_keys: Mutex::new(HashMap::new()),
        });

        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(bot.run_loop());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_key_default() {
        let bot = BotInner {
            client: reqwest::Client::new(),
            token: String::new(),
            manager: Arc::new(Mutex::new(dummy_manager())),
            allowed_users: HashSet::new(),
            active_prompts: Mutex::new(HashMap::new()),
            current_keys: Mutex::new(HashMap::new()),
        };
        assert_eq!(bot.session_key(42), "42");
        assert_eq!(bot.session_key(-100), "-100");
    }

    #[test]
    fn test_session_key_override() {
        let mut keys = HashMap::new();
        keys.insert(42i64, "42:123456789".to_string());
        let bot = BotInner {
            client: reqwest::Client::new(),
            token: String::new(),
            manager: Arc::new(Mutex::new(dummy_manager())),
            allowed_users: HashSet::new(),
            active_prompts: Mutex::new(HashMap::new()),
            current_keys: Mutex::new(keys),
        };
        assert_eq!(bot.session_key(42), "42:123456789");
        assert_eq!(bot.session_key(99), "99");
    }

    fn dummy_manager() -> SessionManager {
        use crate::session::store::Store;
        use crate::session::{Cache, KeyStore, Session};
        let store =
            Store::new("/tmp/mate-test-nonexistent").unwrap_or_else(|_| Store::new(".").unwrap());
        let ks = KeyStore::new("/tmp/mate-test-ks-dummy.json")
            .unwrap_or_else(|_| KeyStore::new("./dummy-ks.json").unwrap());
        let cache = Cache::new(1);
        SessionManager::new(
            store,
            cache,
            ks,
            Box::new(|_s: Session| panic!("factory should not be called")),
        )
    }
}
