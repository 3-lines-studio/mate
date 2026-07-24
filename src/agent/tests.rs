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
        compacted_summary: String::new(),
        compacted_up_to: String::new(),
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
    fresh.compacted_summary = "sum".to_string();
    fresh.compacted_up_to = "t0".to_string();

    agent.reload_from(fresh);

    let sess = agent.sess();
    assert_eq!(sess.current_turn, "t1");
    assert_eq!(sess.prompt_tokens, 120);
    assert_eq!(sess.completion_tokens, 80);
    assert_eq!(sess.context_tokens, 200);
    assert_eq!(sess.cost, 1.5);
    assert_eq!(sess.compacted_summary, "sum");
    assert_eq!(sess.compacted_up_to, "t0");
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
