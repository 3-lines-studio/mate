use super::*;
use crate::provider::{
    ChatClient, ChatRequest, Client, ModelProfile, ProviderError, StreamEvent, StreamToolCall,
};
use crate::session::Session;
use crate::session::store::Store;
use crate::tools::Registry;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::mpsc;

fn se(event_type: &str, delta: &str) -> StreamEvent {
    match event_type {
        "text_delta" => StreamEvent::TextDelta {
            delta: delta.to_string(),
        },
        "reasoning_delta" => StreamEvent::ReasoningDelta {
            delta: delta.to_string(),
        },
        _ => panic!("unknown event_type: {}", event_type),
    }
}
fn se_tool(tc: StreamToolCall) -> StreamEvent {
    StreamEvent::ToolCall { call: tc }
}
fn se_finish(reason: &str) -> StreamEvent {
    StreamEvent::FinishReason {
        reason: reason.to_string(),
    }
}

struct MockClient {
    queue: Mutex<std::collections::VecDeque<Vec<StreamEvent>>>,
}
impl MockClient {
    fn new(responses: Vec<Vec<StreamEvent>>) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(responses.into()),
        })
    }
}
#[async_trait::async_trait]
impl ChatClient for MockClient {
    async fn chat(&self, _req: ChatRequest) -> Result<mpsc::Receiver<StreamEvent>, ProviderError> {
        let events = self.queue.lock().unwrap().pop_front().unwrap_or_default();
        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move {
            for ev in events {
                let _ = tx.send(ev).await;
            }
        });
        Ok(rx)
    }
    fn model(&self) -> &str {
        "mock"
    }
    fn context_window(&self) -> i32 {
        8000
    }
    fn pricing(&self) -> (f64, f64, f64) {
        (0.0, 0.0, 0.0)
    }
}

fn dummy_session() -> Session {
    Session {
        id: "s1".to_string(),
        name: String::new(),
        hash: "h".to_string(),
        named: false,
        current_turn: String::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        turn_count: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        context_tokens: 0,
        cost: 0.0,
    }
}

fn dummy_agent() -> AgentSession {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));
    let client: Arc<dyn ChatClient> = Arc::new(Client::new(
        "http://localhost",
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            ..Default::default()
        },
    ));
    let registry = Arc::new(Registry::new());
    AgentSession::new(
        store,
        dummy_session(),
        client,
        registry,
        "sys".to_string(),
        "/tmp".to_string(),
    )
}

#[test]
fn test_reload_from_syncs_session_and_accumulators() {
    let mut agent = dummy_agent();

    let mut fresh = agent.sess().clone();
    fresh.current_turn = "t1".to_string();
    fresh.prompt_tokens = 120;
    fresh.completion_tokens = 80;
    fresh.context_tokens = 200;
    fresh.cost = 1.5;

    agent.reload_from(fresh);

    let sess = agent.sess();
    assert_eq!(sess.current_turn, "t1");
    assert_eq!(sess.prompt_tokens, 120);
    assert_eq!(sess.completion_tokens, 80);
    assert_eq!(sess.context_tokens, 200);
    assert_eq!(sess.cost, 1.5);
}

fn dummy_tool(name: &str, result: &str) -> crate::tools::Tool {
    let result = result.to_string();
    crate::tools::Tool {
        name: name.to_string(),
        description: String::new(),
        parameters: BTreeMap::new(),
        execute: Arc::new(move |_| {
            let result = result.clone();
            Box::pin(async move { Ok(result) })
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_end_to_end() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let delegate_args = serde_json::json!({
        "subagent": "coder", "task": "say hello", "context": ""
    })
    .to_string();
    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: delegate_args,
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    let sub_responses = vec![vec![
        se("text_delta", "subagent result here"),
        se_finish("stop"),
    ]];
    let sub_client = MockClient::new(sub_responses);

    let sub_registry = Registry::new();

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: sub_client,
        registry: Arc::new(sub_registry),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let registry = Arc::new(Registry::new());
    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        registry,
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut delegate_result = String::new();
    let mut final_text = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::ToolResult { name, result, .. } = &ev.kind
            && name == "delegate"
        {
            delegate_result = result.clone();
        }
        if let EventKind::TextDelta(delta) = &ev.kind {
            final_text.push_str(delta);
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }

    assert_eq!(final_text, "parent-done");
    assert!(
        delegate_result.contains("subagent result here"),
        "delegate result was: {:?}",
        delegate_result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_subagent_with_tool_round() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let delegate_args = serde_json::json!({
        "subagent": "coder", "task": "read and summarize", "context": ""
    })
    .to_string();
    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: delegate_args,
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    let sub_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "sc1".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }),
            se_finish("tool_calls"),
        ],
        vec![
            se("text_delta", "summarized file contents"),
            se_finish("stop"),
        ],
    ];
    let sub_client = MockClient::new(sub_responses);

    let mut sub_registry = Registry::new();
    let _ = sub_registry.register(dummy_tool("read_file", "FILE BODY"));

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: sub_client,
        registry: Arc::new(sub_registry),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let registry = Arc::new(Registry::new());
    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        registry,
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut delegate_result = String::new();
    let mut final_text = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::ToolResult { name, result, .. } = &ev.kind
            && name == "delegate"
        {
            delegate_result = result.clone();
        }
        if let EventKind::TextDelta(delta) = &ev.kind {
            final_text.push_str(delta);
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }

    assert_eq!(final_text, "parent-done");
    assert!(
        delegate_result.contains("summarized file contents"),
        "delegate result was: {:?}",
        delegate_result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_invalid_params_is_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: r#"{"subagent":"coder"}"#.into(),
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: MockClient::new(vec![]),
        registry: Arc::new(Registry::new()),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        Arc::new(Registry::new()),
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut saw_error = false;
    let mut err_text = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::ToolError { name, error, .. } = &ev.kind
            && name == "delegate"
        {
            saw_error = true;
            err_text = error.clone();
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }
    assert!(saw_error, "expected ToolError for bad args");
    assert!(
        err_text.contains("invalid delegate params"),
        "err was: {:?}",
        err_text
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_unknown_subagent_is_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let args = serde_json::json!({
        "subagent": "nope", "task": "x"
    })
    .to_string();
    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: args,
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: MockClient::new(vec![]),
        registry: Arc::new(Registry::new()),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        Arc::new(Registry::new()),
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut saw_error = false;
    let mut err_text = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::ToolError { name, error, .. } = &ev.kind
            && name == "delegate"
        {
            saw_error = true;
            err_text = error.clone();
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }
    assert!(saw_error, "expected ToolError for unknown subagent");
    assert!(err_text.contains("not found"), "err was: {:?}", err_text);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_no_final_message_lists_tools() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let delegate_args = serde_json::json!({
        "subagent": "coder", "task": "just tool"
    })
    .to_string();
    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: delegate_args,
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    // Subagent does a tool round then stops with empty assistant content.
    let sub_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "sc1".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }),
            se_finish("tool_calls"),
        ],
        vec![se_finish("stop")],
    ];
    let mut sub_registry = Registry::new();
    let _ = sub_registry.register(dummy_tool("read_file", "body"));

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: MockClient::new(sub_responses),
        registry: Arc::new(sub_registry),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        Arc::new(Registry::new()),
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut delegate_result = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::ToolResult { name, result, .. } = &ev.kind
            && name == "delegate"
        {
            delegate_result = result.clone();
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }
    assert!(
        delegate_result.contains("tools used: read_file"),
        "delegate result was: {:?}",
        delegate_result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_ignores_pretool_chatter_without_final() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let delegate_args = serde_json::json!({
        "subagent": "coder", "task": "just tool"
    })
    .to_string();
    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: delegate_args,
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    // Pre-tool chatter, then tools, then empty final stop — must not return chatter.
    let sub_responses = vec![
        vec![
            se("text_delta", "I'll inspect the file first"),
            se_tool(StreamToolCall {
                id: "sc1".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }),
            se_finish("tool_calls"),
        ],
        vec![se_finish("stop")],
    ];
    let mut sub_registry = Registry::new();
    let _ = sub_registry.register(dummy_tool("read_file", "body"));

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: MockClient::new(sub_responses),
        registry: Arc::new(sub_registry),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        Arc::new(Registry::new()),
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut delegate_result = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::ToolResult { name, result, .. } = &ev.kind
            && name == "delegate"
        {
            delegate_result = result.clone();
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }
    assert!(
        !delegate_result.contains("I'll inspect"),
        "pre-tool chatter leaked: {:?}",
        delegate_result
    );
    assert!(
        delegate_result.contains("tools used: read_file"),
        "delegate result was: {:?}",
        delegate_result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_subagent_error_is_tool_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let delegate_args = serde_json::json!({
        "subagent": "coder", "task": "do work"
    })
    .to_string();
    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: delegate_args,
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    // Partial text + retryable error → RetryAvailable (had content, no silent retry).
    let sub_responses = vec![vec![
        se("text_delta", "partial"),
        StreamEvent::Error {
            error: ProviderError {
                status_code: 500,
                body: "boom".into(),
            },
        },
    ]];

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: MockClient::new(sub_responses),
        registry: Arc::new(Registry::new()),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        Arc::new(Registry::new()),
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut saw_error = false;
    let mut err_text = String::new();
    let mut saw_result = false;
    while let Some(ev) = rx.recv().await {
        match &ev.kind {
            EventKind::ToolError { name, error, .. } if name == "delegate" => {
                saw_error = true;
                err_text = error.clone();
            }
            EventKind::ToolResult { name, .. } if name == "delegate" => {
                saw_result = true;
            }
            EventKind::AgentDone(_) => break,
            _ => {}
        }
    }
    assert!(saw_error, "expected ToolError for subagent RetryAvailable");
    assert!(!saw_result, "must not emit ToolResult on subagent error");
    assert!(
        err_text.contains("subagent error") && err_text.contains("boom"),
        "err was: {:?}",
        err_text
    );
    assert!(
        !err_text.contains("partial"),
        "partial content must not leak into error: {:?}",
        err_text
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delegate_empty_task_is_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let args = serde_json::json!({
        "subagent": "coder", "task": "   "
    })
    .to_string();
    let parent_responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "call1".into(),
                name: "delegate".into(),
                arguments: args,
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "parent-done"), se_finish("stop")],
    ];
    let parent_client = MockClient::new(parent_responses);

    let sub_def = SubagentDef {
        id: "coder".to_string(),
        description: "coder".to_string(),
        client: MockClient::new(vec![]),
        registry: Arc::new(Registry::new()),
        system_prompt: "sub".to_string(),
        model_name: "mock".to_string(),
    };

    let mut agent = AgentSession::new(
        store,
        dummy_session(),
        parent_client,
        Arc::new(Registry::new()),
        "sys".to_string(),
        "/tmp".to_string(),
    );
    agent.set_subagents(HashMap::from([("coder".to_string(), sub_def)]));

    let mut rx = agent.prompt("please delegate");
    let mut saw_error = false;
    let mut err_text = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::ToolError { name, error, .. } = &ev.kind
            && name == "delegate"
        {
            saw_error = true;
            err_text = error.clone();
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }
    assert!(saw_error, "expected ToolError for empty task");
    assert!(
        err_text.contains("task is empty"),
        "err was: {:?}",
        err_text
    );
}

#[test]
fn test_set_subagents_replaces_delegate_def() {
    let mut agent = dummy_agent();
    let make_def = |id: &str| SubagentDef {
        id: id.to_string(),
        description: id.to_string(),
        client: MockClient::new(vec![]),
        registry: Arc::new(Registry::new()),
        system_prompt: "s".to_string(),
        model_name: "m".to_string(),
    };
    agent.set_subagents(HashMap::from([("a".to_string(), make_def("a"))]));
    agent.set_subagents(HashMap::from([
        ("a".to_string(), make_def("a")),
        ("b".to_string(), make_def("b")),
    ]));
    let defs = agent.tool_defs();
    let delegates: Vec<_> = defs
        .iter()
        .filter(|d| d.function.name == "delegate")
        .collect();
    assert_eq!(delegates.len(), 1, "delegate tool should not be duplicated");
}

#[test]
fn test_delegate_def_lists_role_tools() {
    let mut agent = dummy_agent();
    let mut reg = Registry::new();
    let _ = reg.register(dummy_tool("bash", "ok"));
    let _ = reg.register(dummy_tool("read_file", "ok"));
    let def = SubagentDef {
        id: "explorer".to_string(),
        description: "Map code. Read-only.".to_string(),
        client: MockClient::new(vec![]),
        registry: Arc::new(reg),
        system_prompt: "s".to_string(),
        model_name: "m".to_string(),
    };
    agent.set_subagents(HashMap::from([("explorer".to_string(), def)]));
    let delegate = agent
        .tool_defs()
        .into_iter()
        .find(|d| d.function.name == "delegate")
        .expect("delegate tool");
    let desc = &delegate.function.description;
    assert!(desc.contains("explorer"), "desc={desc}");
    assert!(desc.contains("Map code. Read-only."), "desc={desc}");
    assert!(desc.contains("[bash, read_file]"), "desc={desc}");
    assert!(!desc.contains("OUTPUT RULES"), "desc={desc}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_commits_partial_turn_after_each_tool_round() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "c1".into(),
                name: "echo".into(),
                arguments: "{}".into(),
            }),
            se_finish("tool_calls"),
        ],
        vec![
            se_tool(StreamToolCall {
                id: "c2".into(),
                name: "echo".into(),
                arguments: "{}".into(),
            }),
            se_finish("tool_calls"),
        ],
        vec![se("text_delta", "all done"), se_finish("stop")],
    ];
    let client = MockClient::new(responses);

    let mut registry = Registry::new();
    let _ = registry.register(dummy_tool("echo", "ok"));

    let mut agent = AgentSession::new(
        store.clone(),
        dummy_session(),
        client,
        Arc::new(registry),
        "sys".to_string(),
        "/tmp".to_string(),
    );

    let mut rx = agent.prompt("do work");
    while let Some(ev) = rx.recv().await {
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }

    let mut store = store.lock().await;
    let index = store.turn_index("s1").unwrap();
    let mains: Vec<_> = index.iter().filter(|m| m.subagent.is_empty()).collect();
    assert_eq!(
        mains.len(),
        3,
        "expected 2 tool partials + final, got {index:?}"
    );
    assert_eq!(mains[0].parent_id, "");
    assert_eq!(mains[1].parent_id, mains[0].id);
    assert_eq!(mains[2].parent_id, mains[1].id);

    let t0 = store.load_turn("s1", &mains[0].id).unwrap();
    assert!(
        t0.messages
            .iter()
            .any(|m| m.role == crate::message::Role::User)
    );
    assert!(
        t0.messages
            .iter()
            .any(|m| m.role == crate::message::Role::Tool)
    );
    assert!(t0.messages.iter().any(|m| !m.tool_calls.is_empty()));

    let t1 = store.load_turn("s1", &mains[1].id).unwrap();
    assert!(
        t1.messages
            .iter()
            .any(|m| m.role == crate::message::Role::Tool)
    );
    assert!(
        !t1.messages
            .iter()
            .any(|m| m.role == crate::message::Role::User)
    );

    let t2 = store.load_turn("s1", &mains[2].id).unwrap();
    assert!(t2.messages.iter().any(|m| m.content.contains("all done")));

    let meta = store.load("s1").unwrap();
    assert_eq!(meta.current_turn, mains[2].id);
    assert_eq!(meta.turn_count, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_partial_turn_survives_error_for_continue() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = Arc::new(TokioMutex::new(
        Store::new(&dir.path().to_string_lossy()).unwrap(),
    ));

    let responses = vec![
        vec![
            se_tool(StreamToolCall {
                id: "c1".into(),
                name: "echo".into(),
                arguments: "{}".into(),
            }),
            se_finish("tool_calls"),
        ],
        vec![
            se("text_delta", "partial"),
            StreamEvent::Error {
                error: ProviderError {
                    status_code: 500,
                    body: "boom".into(),
                },
            },
        ],
    ];
    let client = MockClient::new(responses);

    let mut registry = Registry::new();
    let _ = registry.register(dummy_tool("echo", "tool-result-1"));

    let mut agent = AgentSession::new(
        store.clone(),
        dummy_session(),
        client,
        Arc::new(registry),
        "sys".to_string(),
        "/tmp".to_string(),
    );

    let mut rx = agent.prompt("start work");
    let mut saw_retry_or_error = false;
    while let Some(ev) = rx.recv().await {
        match &ev.kind {
            EventKind::RetryAvailable(_) | EventKind::Error(_) => {
                saw_retry_or_error = true;
                break;
            }
            EventKind::AgentDone(_) => break,
            _ => {}
        }
    }
    assert!(saw_retry_or_error);

    {
        let mut store = store.lock().await;
        let index = store.turn_index("s1").unwrap();
        let mains: Vec<_> = index.iter().filter(|m| m.subagent.is_empty()).collect();
        assert_eq!(mains.len(), 1, "tool round must be committed before error");
        let t0 = store.load_turn("s1", &mains[0].id).unwrap();
        assert!(
            t0.messages
                .iter()
                .any(|m| m.content.contains("tool-result-1"))
        );
        let meta = store.load("s1").unwrap();
        assert_eq!(meta.current_turn, mains[0].id);
    }

    let cont_responses = vec![vec![se("text_delta", "continued"), se_finish("stop")]];
    let cont_client = MockClient::new(cont_responses);
    let mut registry = Registry::new();
    let _ = registry.register(dummy_tool("echo", "ok"));
    let sess = store.lock().await.load("s1").unwrap();
    let mut agent = AgentSession::new(
        store.clone(),
        sess,
        cont_client,
        Arc::new(registry),
        "sys".to_string(),
        "/tmp".to_string(),
    );

    let mut rx = agent.prompt("continue");
    let mut final_text = String::new();
    while let Some(ev) = rx.recv().await {
        if let EventKind::TextDelta(delta) = &ev.kind {
            final_text.push_str(delta);
        }
        if matches!(&ev.kind, EventKind::AgentDone(_)) {
            break;
        }
    }
    assert_eq!(final_text, "continued");

    let mut store = store.lock().await;
    let meta = store.load("s1").unwrap();
    let ancestry = store.ancestry("s1", &meta.current_turn).unwrap();
    assert_eq!(ancestry.len(), 2);
    let flat: String = ancestry
        .iter()
        .flat_map(|t| t.messages.iter())
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        flat.contains("tool-result-1"),
        "continue must see prior tool result: {flat}"
    );
    assert!(
        flat.contains("continued"),
        "continue must commit final text: {flat}"
    );
}
