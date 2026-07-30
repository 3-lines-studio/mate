use super::format::format_duration;
use super::types::{DelegateData, Event, EventKind, PendingTool, SubagentDef, SubagentTurn};
use crate::message::{Message, Role, ToolCall, ToolDef};
use crate::provider::{StreamToolCall, Usage};
use crate::session::store::Store;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

const DELEGATE_TOOL_NAME: &str = "delegate";

struct AbortOnDrop {
    handle: Option<AbortHandle>,
}

impl AbortOnDrop {
    fn new(handle: AbortHandle) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    fn disarm(&mut self) {
        self.handle.take();
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

#[derive(Deserialize)]
struct DelegateParams {
    subagent: String,
    task: String,
    #[serde(default)]
    context: String,
}

impl super::AgentSession {
    pub(super) async fn execute_tools(
        &mut self,
        tool_calls: &[StreamToolCall],
        events: &mpsc::Sender<Event>,
    ) -> Vec<PendingTool> {
        let n = tool_calls.len();

        for tc in tool_calls {
            let _ = events.send(Event::tool_call_start(tc)).await;
        }

        let mut set = tokio::task::JoinSet::new();

        for (i, tc) in tool_calls.iter().enumerate() {
            let tc = tc.clone();
            let events = events.clone();
            if tc.name == DELEGATE_TOOL_NAME && !self.subagents_state.subagents.is_empty() {
                self.spawn_delegate_task(&mut set, i, tc, events);
                continue;
            }
            self.spawn_registry_task(&mut set, i, tc, events);
        }

        let mut pending: Vec<PendingTool> = (0..n)
            .map(|i| PendingTool {
                call: tool_calls[i].clone(),
                result: String::new(),
                duration: String::new(),
            })
            .collect();
        let mut outstanding: HashSet<usize> = (0..n).collect();
        let mut deferred_errors: Vec<Event> = Vec::new();
        let mut subagent_cost = 0.0;

        while let Some(result) = set.join_next().await {
            match result {
                Ok((i, result_str, dur, is_error, delegate_data)) => {
                    outstanding.remove(&i);
                    pending[i].result = result_str;
                    pending[i].duration = dur;
                    if is_error {
                        deferred_errors.push(Event::tool_error_ev(
                            &pending[i].call,
                            &pending[i].result,
                            &pending[i].duration,
                        ));
                    }
                    if let Some(dd) = delegate_data {
                        self.sess.prompt_tokens += dd.prompt_tokens;
                        self.sess.completion_tokens += dd.completion_tokens;
                        self.sess.cost += dd.cost;
                        subagent_cost += dd.cost;
                        self.subagents_state.subagent_turns.push(SubagentTurn {
                            msgs: dd.msgs,
                            subagent: dd.subagent_id,
                            tool_call_id: dd.tool_call_id,
                        });
                    }
                }
                Err(e) => {
                    log::warn!("tool task join error: {}", e);
                }
            }
        }

        for i in outstanding {
            pending[i].result = "tool task failed".into();
            deferred_errors.push(Event::tool_error_ev(
                &pending[i].call,
                &pending[i].result,
                &pending[i].duration,
            ));
        }

        if subagent_cost > 0.0 {
            let _ = events
                .send(Event::usage_ev(Usage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    prompt_cache_hit_tokens: 0,
                    prompt_tokens_details: None,
                    completion_tokens_details: None,
                    cost: subagent_cost,
                }))
                .await;
        }

        for ev in deferred_errors {
            let _ = events.send(ev).await;
        }

        pending
    }

    fn spawn_delegate_task(
        &mut self,
        set: &mut tokio::task::JoinSet<(usize, String, String, bool, Option<DelegateData>)>,
        i: usize,
        tc: StreamToolCall,
        events: mpsc::Sender<Event>,
    ) {
        let params: DelegateParams = match serde_json::from_str(&tc.arguments) {
            Ok(p) => p,
            Err(e) => {
                set.spawn(async move {
                    let msg = format!("invalid delegate params: {}", e);
                    (i, msg, String::new(), true, None)
                });
                return;
            }
        };

        if params.task.trim().is_empty() {
            set.spawn(async move {
                (
                    i,
                    "invalid delegate params: task is empty".into(),
                    String::new(),
                    true,
                    None,
                )
            });
            return;
        }

        let def = match self.subagents_state.subagents.get(&params.subagent) {
            Some(d) => d.clone(),
            None => {
                let name = params.subagent;
                set.spawn(async move {
                    let msg = format!("subagent {:?} not found", name);
                    (i, msg, String::new(), true, None)
                });
                return;
            }
        };

        let store = self.store.clone();
        let sess_id = self.sess.id.clone();
        let cwd = self.cwd.clone();

        set.spawn(async move {
            let start = std::time::Instant::now();
            let tc_clone = tc.clone();
            let events_clone = events.clone();
            let (result, is_error, dd) = run_delegate(DelegateRun {
                tc,
                params,
                def,
                store,
                sess_id,
                cwd,
                parent_events: events,
            })
            .await;
            let dur = format_duration(start.elapsed());
            if !is_error {
                let _ = events_clone
                    .send(Event::tool_result_ev(&tc_clone, &result, &dur))
                    .await;
            }
            (i, result, dur, is_error, dd)
        });
    }

    fn spawn_registry_task(
        &self,
        set: &mut tokio::task::JoinSet<(usize, String, String, bool, Option<DelegateData>)>,
        i: usize,
        tc: StreamToolCall,
        events: mpsc::Sender<Event>,
    ) {
        let tools = self.tools.clone();

        set.spawn(async move {
            let start = std::time::Instant::now();
            match tools.get(&tc.name) {
                None => {
                    let msg = format!("Tool {} not found", tc.name);
                    (i, msg, String::new(), true, None)
                }
                Some(tool) => {
                    let args: serde_json::Value = match serde_json::from_str(&tc.arguments) {
                        Ok(v) => v,
                        Err(e) => {
                            let msg = format!("Tool {} invalid args: {}", tc.name, e);
                            let dur = format_duration(start.elapsed());
                            return (i, msg, dur, true, None);
                        }
                    };
                    let tool_result = tokio::time::timeout(
                        std::time::Duration::from_secs(super::TOOL_TIMEOUT_SECS),
                        (tool.execute)(args),
                    )
                    .await;
                    let dur = format_duration(start.elapsed());
                    match tool_result {
                        Ok(Ok(result)) => {
                            let final_result = result;
                            let _ = events
                                .send(Event::tool_result_ev(&tc, &final_result, &dur))
                                .await;
                            (i, final_result, dur, false, None)
                        }
                        Ok(Err(e)) => {
                            let msg = format!("Tool {} error: {}", tc.name, e);
                            (i, msg, dur, true, None)
                        }
                        Err(_) => {
                            let msg = format!(
                                "Tool {} timed out after {}s",
                                tc.name,
                                super::TOOL_TIMEOUT_SECS
                            );
                            (i, msg, dur, true, None)
                        }
                    }
                }
            }
        });
    }

    pub(super) fn append_tool_messages(
        &mut self,
        pending: &[PendingTool],
        reasoning: &str,
        details: &[crate::message::ReasoningDetail],
    ) {
        let assistant_tool_calls: Vec<ToolCall> =
            pending.iter().map(|pt| pt.call.clone().into()).collect();

        self.working_messages.push(Message {
            role: Role::Assistant,
            content: String::new(),
            reasoning_content: reasoning.into(),
            reasoning_details: details.to_vec(),
            tool_calls: assistant_tool_calls,
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        });

        for pt in pending {
            let content = if pt.result.is_empty() {
                "(no output)".into()
            } else {
                pt.result.clone()
            };
            self.working_messages.push(Message {
                role: Role::Tool,
                content,
                reasoning_content: String::new(),
                reasoning_details: vec![],
                tool_calls: vec![],
                tool_call_id: pt.call.id.clone(),
                name: pt.call.name.clone(),
                tool_duration: pt.duration.clone(),
            });
        }
    }
}

struct DelegateRun {
    tc: StreamToolCall,
    params: DelegateParams,
    def: SubagentDef,
    store: Arc<TokioMutex<Store>>,
    sess_id: String,
    cwd: String,
    parent_events: mpsc::Sender<Event>,
}

async fn run_delegate(run: DelegateRun) -> (String, bool, Option<DelegateData>) {
    let DelegateRun {
        tc,
        params,
        def,
        store,
        sess_id,
        cwd,
        parent_events,
    } = run;

    let mut task_text = params.task;
    if !params.context.is_empty() {
        task_text.push_str("\n\n");
        task_text.push_str(&params.context);
    }

    let mut sub = super::AgentSession::new_subagent(store, sess_id, &def, cwd, &tc.id);

    let (event_tx, mut event_rx) = mpsc::channel(100);
    let sub_id = def.id.clone();
    let tc_id = tc.id.clone();

    let join_handle = tokio::spawn(async move {
        sub.run_loop(&task_text, &event_tx).await;
        (
            sub.sess.prompt_tokens,
            sub.sess.completion_tokens,
            sub.sess.cost,
            sub.captured_msgs,
            sub.working_messages,
        )
    });
    let mut abort_guard = AbortOnDrop::new(join_handle.abort_handle());

    let mut had_error = false;
    let mut error_msg = String::new();
    while let Some(mut ev) = event_rx.recv().await {
        match &ev.kind {
            EventKind::AgentDone(_)
            | EventKind::Usage(_)
            | EventKind::TextDelta(_)
            | EventKind::ReasoningDelta(_)
            | EventKind::Retry(_) => {}
            EventKind::Error(msg) | EventKind::RetryAvailable(msg) => {
                had_error = true;
                error_msg = msg.clone();
            }
            _ => {
                ev.subagent = sub_id.clone();
                ev.subagent_id = tc_id.clone();
                let _ = parent_events.send(ev).await;
            }
        }
    }

    let joined = join_handle.await;
    abort_guard.disarm();

    let (prompt_tokens, completion_tokens, cost, captured_msgs, working_msgs) = match joined {
        Ok(v) => v,
        Err(e) => {
            if e.is_cancelled() {
                return (
                    "(subagent aborted)".into(),
                    true,
                    Some(DelegateData {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        cost: 0.0,
                        msgs: vec![],
                        subagent_id: sub_id,
                        tool_call_id: tc_id,
                    }),
                );
            }
            return (
                format!("subagent task panicked: {}", e),
                true,
                Some(DelegateData {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cost: 0.0,
                    msgs: vec![],
                    subagent_id: sub_id,
                    tool_call_id: tc_id,
                }),
            );
        }
    };

    let msgs = if captured_msgs.is_empty() {
        working_msgs
    } else {
        captured_msgs
    };

    let (final_result, is_error) = if had_error {
        let result = if error_msg.is_empty() {
            "(subagent encountered an error)".into()
        } else {
            format!("(subagent error: {})", error_msg)
        };
        (result, true)
    } else {
        (extract_parent_result(&msgs), false)
    };

    (
        final_result,
        is_error,
        Some(DelegateData {
            prompt_tokens,
            completion_tokens,
            cost,
            msgs,
            subagent_id: sub_id,
            tool_call_id: tc_id,
        }),
    )
}

fn extract_parent_result(msgs: &[Message]) -> String {
    for (i, msg) in msgs.iter().enumerate().rev() {
        if msg.role != Role::Assistant || msg.content.is_empty() || !msg.tool_calls.is_empty() {
            continue;
        }
        let tools_after = msgs[i + 1..].iter().any(|m| {
            m.role == Role::Tool || (m.role == Role::Assistant && !m.tool_calls.is_empty())
        });
        if !tools_after {
            return msg.content.clone();
        }
    }

    let mut tools: Vec<&str> = Vec::new();
    for msg in msgs {
        if msg.role == Role::Tool && !msg.name.is_empty() && !tools.contains(&msg.name.as_str()) {
            tools.push(&msg.name);
        }
    }
    if tools.is_empty() {
        "(subagent produced no output)".into()
    } else {
        format!(
            "(subagent finished without a final message; tools used: {})",
            tools.join(", ")
        )
    }
}

pub(super) struct DelegateRole {
    pub id: String,
    pub description: String,
    pub tools: Vec<String>,
}

pub(super) fn build_delegate_def(roles: &[DelegateRole]) -> ToolDef {
    let names: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();

    let mut tool_desc = String::from(
        "Hand off one scoped job to a subagent. Isolated history; no nested delegate. \
Only the final message returns. Fan out independent calls. Not for one trivial tool call.\n\nRoles:",
    );
    for r in roles {
        tool_desc.push_str("\n- ");
        tool_desc.push_str(&r.id);
        if !r.description.is_empty() {
            tool_desc.push_str(" — ");
            tool_desc.push_str(&r.description);
        }
        if !r.tools.is_empty() {
            tool_desc.push_str(" [");
            tool_desc.push_str(&r.tools.join(", "));
            tool_desc.push(']');
        }
    }

    let params = crate::tools::object_schema(
        &[
            (
                "subagent",
                serde_json::json!({
                    "type": "string",
                    "enum": names,
                    "description": "Role id"
                }),
            ),
            (
                "task",
                serde_json::json!({
                    "type": "string",
                    "description": "One goal, success check, paths, constraints. Prefer paths over pastes."
                }),
            ),
            (
                "context",
                serde_json::json!({
                    "type": "string",
                    "description": "Optional bulk (snippets, logs). Prefer paths when disk is readable."
                }),
            ),
        ],
        &["subagent", "task"],
    );

    ToolDef {
        def_type: "function".into(),
        function: crate::message::ToolDefFunction {
            name: DELEGATE_TOOL_NAME.into(),
            description: tool_desc,
            parameters: params,
        },
    }
}
