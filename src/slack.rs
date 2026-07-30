use crate::core::session_manager::SessionManager;
use crate::core::{Deps, Interface};
use crate::integration::{self, ActivePrompt, StreamingBackend};
use crate::markdown::slack::markdown_to_slack;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const SLACK_TEXT_LIMIT: usize = 3500;

static IMG_EXT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[\w./\-]+\.(?:png|jpg|jpeg|gif|webp)").unwrap());

fn api_url(method: &str) -> String {
    format!("https://www.slack.com/api/{}", method)
}

#[derive(Deserialize)]
struct SlackApiResp {
    ok: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ConnectionsOpenResp {
    ok: bool,
    url: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct AuthTestResp {
    ok: bool,
    user_id: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct PostMessageResp {
    ok: bool,
    ts: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UploadUrlResp {
    ok: bool,
    error: Option<String>,
    upload_url: Option<String>,
    file_id: Option<String>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    env_type: String,
    envelope_id: Option<String>,
    payload: Option<serde_json::Value>,
}

struct SlackEvent {
    text: String,
    ts: String,
    channel: String,
    thread_ts: String,
}

async fn auth_test(client: &reqwest::Client, bot_token: &str) -> Result<String, String> {
    let resp: AuthTestResp = client
        .post(api_url("auth.test"))
        .header("Authorization", format!("Bearer {}", bot_token))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok {
        return Err(resp.error.unwrap_or_else(|| "auth.test failed".into()));
    }
    Ok(resp.user_id.unwrap_or_default())
}

async fn connections_open(client: &reqwest::Client, app_token: &str) -> Result<String, String> {
    let resp: ConnectionsOpenResp = client
        .post(api_url("apps.connections.open"))
        .header("Authorization", format!("Bearer {}", app_token))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "connections.open failed".into()));
    }
    resp.url.ok_or_else(|| "no url in connections.open".into())
}

async fn post_message(
    client: &reqwest::Client,
    bot_token: &str,
    channel: &str,
    text: &str,
    thread_ts: Option<&str>,
) -> Result<String, String> {
    let mut body = serde_json::json!({"channel": channel, "text": text});
    if let Some(ts) = thread_ts
        && !ts.is_empty()
    {
        body["thread_ts"] = serde_json::Value::String(ts.to_string());
    }
    let resp: PostMessageResp = client
        .post(api_url("chat.postMessage"))
        .header("Authorization", format!("Bearer {}", bot_token))
        .json(&body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok {
        return Err(resp.error.unwrap_or_else(|| "postMessage failed".into()));
    }
    resp.ts.ok_or_else(|| "no ts in postMessage".into())
}

async fn update_message(
    client: &reqwest::Client,
    bot_token: &str,
    channel: &str,
    ts: &str,
    text: &str,
) -> Result<(), String> {
    let body = serde_json::json!({"channel": channel, "ts": ts, "text": text});
    let resp: SlackApiResp = client
        .post(api_url("chat.update"))
        .header("Authorization", format!("Bearer {}", bot_token))
        .json(&body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok {
        return Err(resp.error.unwrap_or_else(|| "chat.update failed".into()));
    }
    Ok(())
}

struct SlackStreaming {
    client: reqwest::Client,
    bot_token: String,
    channel: String,
    thread_ts: String,
    images: Mutex<Vec<String>>,
}

#[async_trait]
impl StreamingBackend for SlackStreaming {
    type MsgId = String;

    fn flush_interval(&self) -> Duration {
        Duration::from_millis(800)
    }

    fn text_limit(&self) -> usize {
        SLACK_TEXT_LIMIT
    }

    fn markdown(&self, text: &str) -> String {
        markdown_to_slack(text)
    }

    async fn send_thinking(&self) -> Result<String, String> {
        post_message(
            &self.client,
            &self.bot_token,
            &self.channel,
            "...",
            Some(&self.thread_ts),
        )
        .await
    }

    async fn edit_message(&self, msg_id: &String, text: &str) -> Result<(), String> {
        update_message(&self.client, &self.bot_token, &self.channel, msg_id, text).await
    }

    async fn post_message(&self, text: &str) -> Result<String, String> {
        post_message(
            &self.client,
            &self.bot_token,
            &self.channel,
            text,
            Some(&self.thread_ts),
        )
        .await
    }

    fn on_event(&self, event: &crate::agent::Event) {
        if let crate::agent::EventKind::ToolResult {
            ref args,
            ref result,
            ..
        } = event.kind
        {
            let mut images = self.images.lock().unwrap();
            collect_images(&mut images, &[args.as_str(), result.as_str()]);
        }
    }

    async fn after_finalize(&self) {
        let images = self.images.lock().unwrap().clone();
        upload_images(
            &self.client,
            &self.bot_token,
            &self.channel,
            &self.thread_ts,
            &images,
        )
        .await;
    }
}

async fn upload_images(
    client: &reqwest::Client,
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    images: &[String],
) {
    for img_path in images {
        let path = std::path::Path::new(img_path);
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let file_body = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("slack upload {}: read {}", filename, e);
                continue;
            }
        };
        let length = file_body.len();
        let length_str = length.to_string();

        let mut filename_enc = String::with_capacity(filename.len());
        for byte in filename.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    filename_enc.push(byte as char);
                }
                _ => filename_enc.push_str(&format!("%{:02X}", byte)),
            }
        }
        let endpoint = format!(
            "{}?filename={}&length={}",
            api_url("files.getUploadURLExternal"),
            filename_enc,
            length_str,
        );

        let resp: UploadUrlResp = match client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", bot_token))
            .timeout(Duration::from_secs(30))
            .send()
            .await
        {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    log::error!("slack upload {}: getUploadURL parse: {}", filename, e);
                    continue;
                }
            },
            Err(e) => {
                log::error!("slack upload {}: getUploadURL: {}", filename, e);
                continue;
            }
        };
        if !resp.ok {
            log::error!(
                "slack upload {}: getUploadURL: {}",
                filename,
                resp.error.unwrap_or_default()
            );
            continue;
        }
        let upload_url = resp.upload_url.unwrap_or_default();
        let file_id = resp.file_id.unwrap_or_default();
        if upload_url.is_empty() || file_id.is_empty() {
            log::error!("slack upload {}: getUploadURL: missing url/id", filename);
            continue;
        }

        if let Err(e) = client
            .post(&upload_url)
            .header("Content-Type", "application/octet-stream")
            .header("Content-Length", length_str.as_str())
            .body(file_body)
            .timeout(Duration::from_secs(60))
            .send()
            .await
        {
            log::error!("slack upload {}: upload to url: {}", filename, e);
            continue;
        }

        let mut body = serde_json::json!({
            "files": [{"id": file_id, "title": filename}],
            "channel_id": channel,
        });
        if !thread_ts.is_empty() {
            body["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
        }
        let ok = match client
            .post(api_url("files.completeUploadExternal"))
            .header("Authorization", format!("Bearer {}", bot_token))
            .json(&body)
            .timeout(Duration::from_secs(30))
            .send()
            .await
        {
            Ok(r) => match r.json::<SlackApiResp>().await {
                Ok(resp) => {
                    if !resp.ok {
                        log::error!(
                            "slack upload {}: completeUpload: {}",
                            filename,
                            resp.error.unwrap_or_default()
                        );
                    }
                    resp.ok
                }
                Err(e) => {
                    log::error!("slack upload {}: completeUpload parse: {}", filename, e);
                    false
                }
            },
            Err(e) => {
                log::error!("slack upload {}: completeUpload: {}", filename, e);
                false
            }
        };
        if ok {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn collect_images(images: &mut Vec<String>, sources: &[&str]) {
    for src in sources {
        for m in IMG_EXT_RE.find_iter(src) {
            let path = std::path::Path::new(m.as_str())
                .components()
                .collect::<std::path::PathBuf>();
            let path_str = path.to_string_lossy().to_string();
            if !std::path::Path::new(&path_str).exists() {
                continue;
            }
            if !images.contains(&path_str) {
                images.push(path_str);
            }
        }
    }
}

struct BotInner {
    client: reqwest::Client,
    bot_token: String,
    app_token: String,
    bot_user_id: String,
    manager: Arc<Mutex<SessionManager>>,
    active_prompts: Mutex<HashMap<String, ActivePrompt>>,
}

impl BotInner {
    async fn run_loop(self: Arc<Self>) {
        loop {
            let url = match connections_open(&self.client, &self.app_token).await {
                Ok(u) => u,
                Err(e) => {
                    log::error!("slack connections.open: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let ws = match tokio_tungstenite::connect_async(&url).await {
                Ok((ws, _)) => ws,
                Err(e) => {
                    log::error!("slack websocket connect: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let mut ws = ws;

            loop {
                let msg = match ws.next().await {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        log::error!("slack websocket: {}", e);
                        break;
                    }
                    None => break,
                };

                if !msg.is_text() {
                    continue;
                }

                let text = msg.into_text().unwrap_or_default();
                let env: Envelope = match serde_json::from_str(&text) {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!("slack envelope parse: {}", e);
                        continue;
                    }
                };

                match env.env_type.as_str() {
                    "hello" => {
                        log::info!("slack connected");
                    }
                    "events_api" => {
                        if let Some(ref eid) = env.envelope_id {
                            let ack = serde_json::json!({"envelope_id": eid}).to_string();
                            let _ = ws.send(Message::Text(ack.into())).await;
                        }
                        if let Some(payload) = env.payload
                            && let Some(event) = parse_event(&payload, &self.bot_user_id)
                        {
                            let bot = self.clone();
                            tokio::spawn(async move {
                                bot.handle_event(event).await;
                            });
                        }
                    }
                    "disconnect" => {
                        log::info!("slack disconnect, reconnecting");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    async fn handle_event(&self, event: SlackEvent) {
        let effective = if event.thread_ts.is_empty() {
            event.ts.clone()
        } else {
            event.thread_ts.clone()
        };
        let backend = SlackStreaming {
            client: self.client.clone(),
            bot_token: self.bot_token.clone(),
            channel: event.channel,
            thread_ts: effective.clone(),
            images: Mutex::new(Vec::new()),
        };
        integration::process_prompt(
            &backend,
            &self.manager,
            &self.active_prompts,
            effective.clone(),
            &effective,
            &event.text,
        )
        .await;
    }
}

fn parse_event(payload: &serde_json::Value, bot_user_id: &str) -> Option<SlackEvent> {
    let event = payload.get("event")?;
    let event_type = event.get("type")?.as_str()?;

    match event_type {
        "app_mention" => {
            let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let mention = format!("<@{}>", bot_user_id);
            let text = text.replace(&mention, "").trim().to_string();
            if text.is_empty() {
                return None;
            }
            let ts = event
                .get("ts")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let thread_ts = event
                .get("thread_ts")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let channel = event
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(SlackEvent {
                text,
                ts,
                channel,
                thread_ts,
            })
        }
        "message" => {
            let channel_type = event
                .get("channel_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if channel_type != "im" {
                return None;
            }
            let subtype = event.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            if !subtype.is_empty() {
                return None;
            }
            let text = event
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                return None;
            }
            let ts = event
                .get("ts")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let channel = event
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(SlackEvent {
                text,
                ts,
                channel,
                thread_ts: String::new(),
            })
        }
        _ => None,
    }
}

pub struct BotAdapter;

impl Interface for BotAdapter {
    fn name(&self) -> &str {
        "slack"
    }

    fn run(&self, deps: Deps) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let bot_token = deps.config.slack.bot_token.clone();
        let app_token = deps.config.slack.app_token.clone();
        if bot_token.is_empty() || app_token.is_empty() {
            return Err("slack bot_token and app_token are required".into());
        }

        let http = reqwest::Client::new();

        let auth = {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(auth_test(&http, &bot_token))
        };
        let bot_user_id = match auth {
            Ok(id) => id,
            Err(e) => {
                log::error!("slack auth test: {}", e);
                String::new()
            }
        };

        let key_store_path = format!("{}/thread_sessions.json", deps.store.dir());
        let manager = deps.new_session_manager(&key_store_path)?;

        let bot = Arc::new(BotInner {
            client: http,
            bot_token,
            app_token,
            bot_user_id,
            manager: Arc::new(Mutex::new(manager)),
            active_prompts: Mutex::new(HashMap::new()),
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
    fn test_parse_app_mention() {
        let payload = serde_json::json!({
            "event": {
                "type": "app_mention",
                "text": "<@U12345> hello world",
                "ts": "123.456",
                "channel": "Cabc",
                "thread_ts": "100.000"
            }
        });
        let ev = parse_event(&payload, "U12345").unwrap();
        assert_eq!(ev.text, "hello world");
        assert_eq!(ev.ts, "123.456");
        assert_eq!(ev.channel, "Cabc");
        assert_eq!(ev.thread_ts, "100.000");
    }

    #[test]
    fn test_parse_app_mention_no_thread() {
        let payload = serde_json::json!({
            "event": {
                "type": "app_mention",
                "text": "<@U12345> hi",
                "ts": "123.456",
                "channel": "Cabc"
            }
        });
        let ev = parse_event(&payload, "U12345").unwrap();
        assert_eq!(ev.text, "hi");
        assert!(ev.thread_ts.is_empty());
    }

    #[test]
    fn test_parse_app_mention_empty_text() {
        let payload = serde_json::json!({
            "event": {
                "type": "app_mention",
                "text": "<@U12345>",
                "ts": "123.456",
                "channel": "Cabc"
            }
        });
        assert!(parse_event(&payload, "U12345").is_none());
    }

    #[test]
    fn test_parse_dm_message() {
        let payload = serde_json::json!({
            "event": {
                "type": "message",
                "text": "hello",
                "ts": "123.456",
                "channel": "Dabc",
                "channel_type": "im"
            }
        });
        let ev = parse_event(&payload, "U12345").unwrap();
        assert_eq!(ev.text, "hello");
        assert_eq!(ev.ts, "123.456");
        assert_eq!(ev.channel, "Dabc");
        assert!(ev.thread_ts.is_empty());
    }

    #[test]
    fn test_parse_dm_message_wrong_type() {
        let payload = serde_json::json!({
            "event": {
                "type": "message",
                "text": "hello",
                "ts": "123.456",
                "channel": "Cabc",
                "channel_type": "channel"
            }
        });
        assert!(parse_event(&payload, "U12345").is_none());
    }

    #[test]
    fn test_parse_dm_message_with_subtype() {
        let payload = serde_json::json!({
            "event": {
                "type": "message",
                "text": "hello",
                "ts": "123.456",
                "channel": "Dabc",
                "channel_type": "im",
                "subtype": "message_changed"
            }
        });
        assert!(parse_event(&payload, "U12345").is_none());
    }

    #[test]
    fn test_parse_unknown_event() {
        let payload = serde_json::json!({
            "event": {
                "type": "reaction_added",
                "user": "U123"
            }
        });
        assert!(parse_event(&payload, "U12345").is_none());
    }

    #[test]
    fn test_session_key_from_mention() {
        let event = SlackEvent {
            text: "hello".into(),
            ts: "123.456".into(),
            channel: "Cabc".into(),
            thread_ts: "100.000".into(),
        };
        let key = if event.thread_ts.is_empty() {
            event.ts.clone()
        } else {
            event.thread_ts.clone()
        };
        assert_eq!(key, "100.000");
    }

    #[test]
    fn test_session_key_from_dm() {
        let event = SlackEvent {
            text: "hello".into(),
            ts: "123.456".into(),
            channel: "Dabc".into(),
            thread_ts: String::new(),
        };
        let key = if event.thread_ts.is_empty() {
            event.ts.clone()
        } else {
            event.thread_ts.clone()
        };
        assert_eq!(key, "123.456");
    }

    #[test]
    fn test_collect_images() {
        use std::fs;
        let dir = std::env::temp_dir();
        let p1 = dir.join(format!("mate_test_collect_{}_a.png", std::process::id()));
        let p2 = dir.join(format!("mate_test_collect_{}_b.jpg", std::process::id()));
        fs::write(&p1, b"x").unwrap();
        fs::write(&p2, b"x").unwrap();
        let p1s = p1.to_string_lossy().to_string();
        let p2s = p2.to_string_lossy().to_string();

        let mut images = Vec::new();
        let src = format!("result: {} and {} also {}", p1s, p2s, p2s);
        collect_images(&mut images, &[&src, "/nonexistent/missing.png"]);

        assert_eq!(images.len(), 2);
        assert!(images.contains(&p1s));
        assert!(images.contains(&p2s));

        let _ = fs::remove_file(&p1);
        let _ = fs::remove_file(&p2);
    }
}
