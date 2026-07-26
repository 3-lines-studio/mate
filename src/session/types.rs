use crate::message::{Message, Role};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMeta {
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: String,
    pub label: String,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subagent: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        rename = "toolCallId"
    )]
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: String,
    pub messages: Vec<Message>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subagent: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        rename = "toolCallId"
    )]
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub hash: String,
    pub named: bool,
    #[serde(rename = "currentTurn")]
    pub current_turn: String,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
    #[serde(rename = "turnCount")]
    pub turn_count: i32,
    #[serde(rename = "promptTokens")]
    pub prompt_tokens: i32,
    #[serde(rename = "completionTokens")]
    pub completion_tokens: i32,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i32,
    #[serde(rename = "contextTokens")]
    pub context_tokens: i32,
    pub cost: f64,
}

pub fn compute_turn_id(parent_id: &str, messages: &[Message]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(parent_id.as_bytes());
    for msg in messages {
        hasher.update([0u8]);
        let json = serde_json::to_string(msg).expect("message serialization infallible");
        hasher.update(json.as_bytes());
    }
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

pub fn turn_label(messages: &[Message]) -> String {
    for msg in messages {
        if msg.role == Role::User && !msg.content.is_empty() {
            let clean = msg.content.replace('\n', " ");
            return crate::util::truncate_with_ellipsis(&clean, 40, "...");
        }
    }
    "(empty)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_turn_id_deterministic() {
        let msgs = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }];
        let id1 = compute_turn_id("parent1", &msgs);
        let id2 = compute_turn_id("parent1", &msgs);
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 16);
    }

    #[test]
    fn test_compute_turn_id_different_messages() {
        let id1 = compute_turn_id(
            "",
            &[Message {
                role: Role::User,
                content: "hello".to_string(),
                reasoning_content: String::new(),
                reasoning_details: vec![],
                tool_calls: vec![],
                tool_call_id: String::new(),
                name: String::new(),
                tool_duration: String::new(),
            }],
        );
        let id2 = compute_turn_id(
            "",
            &[Message {
                role: Role::User,
                content: "world".to_string(),
                reasoning_content: String::new(),
                reasoning_details: vec![],
                tool_calls: vec![],
                tool_call_id: String::new(),
                name: String::new(),
                tool_duration: String::new(),
            }],
        );
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_compute_turn_id_different_parent() {
        let msgs = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }];
        let id1 = compute_turn_id("parent1", &msgs);
        let id2 = compute_turn_id("parent2", &msgs);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_turn_label() {
        let msgs = vec![Message {
            role: Role::User,
            content: "what is the answer to life the universe and everything?".to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }];
        let label = turn_label(&msgs);
        assert!(label.contains("what is the answer"));
        assert!(label.len() <= 40);
    }

    #[test]
    fn test_turn_label_short_message() {
        let msgs = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }];
        let label = turn_label(&msgs);
        assert_eq!(label, "hello");
    }

    #[test]
    fn test_turn_label_no_user_message() {
        let msgs = vec![Message {
            role: Role::Assistant,
            content: "response".to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }];
        let label = turn_label(&msgs);
        assert_eq!(label, "(empty)");
    }

    #[test]
    fn test_turn_label_multiline() {
        let msgs = vec![Message {
            role: Role::User,
            content: "line one\nline two\nline three".to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }];
        let label = turn_label(&msgs);
        assert!(!label.contains('\n'));
    }

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::User,
            content: content.to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }
    }

    #[test]
    fn test_turn_label_multibyte_does_not_panic() {
        let long_cjk = "日本語のテキスト".repeat(10);
        let msgs = vec![user_msg(&long_cjk)];
        let label = turn_label(&msgs);
        assert!(label.ends_with("..."));
        assert!(label.chars().count() <= 40);
        let _ = std::str::from_utf8(label.as_bytes()).unwrap();
    }

    #[test]
    fn test_turn_label_emoji_does_not_panic() {
        let msgs = vec![user_msg(&"🦀".repeat(50))];
        let label = turn_label(&msgs);
        assert!(label.ends_with("..."));
    }
}
