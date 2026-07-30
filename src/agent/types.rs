use crate::message::{Message, ReasoningDetail};
use crate::provider::{ChatClient, StreamToolCall, Usage};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Event {
    pub kind: EventKind,
    pub subagent: String,
    pub subagent_id: String,
}

#[derive(Debug, Clone)]
pub enum EventKind {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart {
        id: String,
        name: String,
        args: String,
    },
    ToolResult {
        id: String,
        name: String,
        args: String,
        result: String,
        duration: String,
    },
    ToolError {
        id: String,
        name: String,
        error: String,
        duration: String,
    },
    Error(String),
    Retry(String),
    RetryAvailable(String),
    AgentDone(String),
    Usage(Usage),
}

impl Event {
    pub fn new(kind: EventKind) -> Self {
        Event {
            kind,
            subagent: String::new(),
            subagent_id: String::new(),
        }
    }

    pub fn from_subagent(kind: EventKind, subagent: String, subagent_id: String) -> Self {
        Event {
            kind,
            subagent,
            subagent_id,
        }
    }

    pub fn text_delta(delta: &str) -> Self {
        Event::new(EventKind::TextDelta(delta.into()))
    }
    pub fn reasoning_delta(delta: &str) -> Self {
        Event::new(EventKind::ReasoningDelta(delta.into()))
    }
    pub fn tool_call_start(tc: &StreamToolCall) -> Self {
        Event::new(EventKind::ToolCallStart {
            id: tc.id.clone(),
            name: tc.name.clone(),
            args: tc.arguments.clone(),
        })
    }
    pub fn tool_result_ev(tc: &StreamToolCall, result: &str, duration: &str) -> Self {
        Event::new(EventKind::ToolResult {
            id: tc.id.clone(),
            name: tc.name.clone(),
            args: tc.arguments.clone(),
            result: result.into(),
            duration: duration.into(),
        })
    }
    pub fn tool_error_ev(tc: &StreamToolCall, msg: &str, duration: &str) -> Self {
        Event::new(EventKind::ToolError {
            id: tc.id.clone(),
            name: tc.name.clone(),
            error: msg.into(),
            duration: duration.into(),
        })
    }
    pub fn error_msg(msg: &str) -> Self {
        Event::new(EventKind::Error(msg.into()))
    }
    pub fn retry_ev(msg: &str) -> Self {
        Event::new(EventKind::Retry(msg.into()))
    }
    pub fn retry_available(msg: &str) -> Self {
        Event::new(EventKind::RetryAvailable(msg.into()))
    }
    pub fn agent_done(finish_reason: &str) -> Self {
        Event::new(EventKind::AgentDone(finish_reason.into()))
    }
    pub fn usage_ev(usage: Usage) -> Self {
        Event::new(EventKind::Usage(usage))
    }
}

#[derive(Clone)]
pub struct SubagentDef {
    pub id: String,
    pub description: String,
    pub client: Arc<dyn ChatClient>,
    pub registry: Arc<crate::tools::Registry>,
    pub system_prompt: String,
    pub model_name: String,
}

#[derive(Clone)]
pub(super) struct SubagentTurn {
    pub msgs: Vec<Message>,
    pub subagent: String,
    pub tool_call_id: String,
}

pub(super) struct PendingTool {
    pub call: StreamToolCall,
    pub result: String,
    pub duration: String,
}

pub(super) struct RoundResult {
    pub content: String,
    pub reasoning: String,
    pub reasoning_details: Vec<ReasoningDetail>,
    pub tool_calls: Vec<StreamToolCall>,
    pub finish_reason: String,
}

#[derive(Clone)]
pub(super) struct SubagentState {
    pub subagents: HashMap<String, SubagentDef>,
    pub subagent_turns: Vec<SubagentTurn>,
    pub is_subagent: bool,
}

pub(super) struct DelegateData {
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cost: f64,
    pub msgs: Vec<Message>,
    pub subagent_id: String,
    pub tool_call_id: String,
}
