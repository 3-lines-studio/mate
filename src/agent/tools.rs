use super::format::format_duration;
use super::types::{DelegateData, Event, EventKind, PendingTool, SubagentDef, SubagentTurn};
use crate::message::{Message, Role, ToolCall, ToolDef};
use crate::provider::{StreamToolCall, Usage};
use crate::session::store::Store;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::mpsc;

const DELEGATE_TOOL_NAME: &str = "delegate";

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
                had_error: false,
            })
            .collect();
        let mut deferred_errors: Vec<Event> = Vec::new();
        let mut subagent_cost = 0.0;

        while let Some(result) = set.join_next().await {
            if let Ok((i, result_str, dur, is_error, delegate_data)) = result {
                pending[i].result = result_str;
                pending[i].duration = dur;
                pending[i].had_error = is_error;
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
        let subagents = self.subagents_state.subagents.clone();
        let store = self.store.clone();
        let sess_id = self.sess.id.clone();
        let cwd = self.cwd.clone();

        set.spawn(async move {
            let start = std::time::Instant::now();
            let tc_clone = tc.clone();
            let events_clone = events.clone();
            let (result, dd) = run_delegate(tc, subagents, store, sess_id, cwd, events).await;
            let dur = format_duration(start.elapsed());
            let _ = events_clone
                .send(Event::tool_result_ev(&tc_clone, &result, &dur))
                .await;
            (i, result, dur, false, Some(dd))
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

fn run_delegate(
    tc: StreamToolCall,
    subagents: HashMap<String, SubagentDef>,
    store: Arc<TokioMutex<Store>>,
    sess_id: String,
    cwd: String,
    parent_events: mpsc::Sender<Event>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = (String, DelegateData)> + Send>> {
    Box::pin(async move {
        #[derive(Deserialize)]
        struct DelegateParams {
            subagent: String,
            task: String,
            #[serde(default)]
            context: String,
        }

        let params: DelegateParams = match serde_json::from_str(&tc.arguments) {
            Ok(p) => p,
            Err(e) => {
                return (
                    format!("invalid delegate params: {}", e),
                    DelegateData {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        cost: 0.0,
                        msgs: vec![],
                        subagent_id: String::new(),
                        tool_call_id: tc.id.clone(),
                    },
                );
            }
        };

        let def = match subagents.get(&params.subagent) {
            Some(d) => d.clone(),
            None => {
                return (
                    format!("subagent {:?} not found", params.subagent),
                    DelegateData {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        cost: 0.0,
                        msgs: vec![],
                        subagent_id: String::new(),
                        tool_call_id: tc.id.clone(),
                    },
                );
            }
        };

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

        let mut had_error = false;
        while let Some(mut ev) = event_rx.recv().await {
            match ev.kind {
                EventKind::AgentDone(_)
                | EventKind::Usage(_)
                | EventKind::TextDelta(_)
                | EventKind::ReasoningDelta(_) => {}
                EventKind::Error(_) => {
                    had_error = true;
                    ev.subagent = sub_id.clone();
                    ev.subagent_id = tc_id.clone();
                    let _ = parent_events.send(ev).await;
                }
                _ => {
                    ev.subagent = sub_id.clone();
                    ev.subagent_id = tc_id.clone();
                    let _ = parent_events.send(ev).await;
                }
            }
        }

        let (prompt_tokens, completion_tokens, cost, captured_msgs, working_msgs) = join_handle
            .await
            .unwrap_or((0, 0, 0.0, Vec::new(), Vec::new()));

        let msgs = if captured_msgs.is_empty() {
            working_msgs
        } else {
            captured_msgs
        };

        if had_error {
            return (
                "(subagent encountered an error)".into(),
                DelegateData {
                    prompt_tokens,
                    completion_tokens,
                    cost,
                    msgs,
                    subagent_id: sub_id,
                    tool_call_id: tc_id,
                },
            );
        }

        let content = extract_final_content(&msgs);
        let final_result = if content.is_empty() {
            "(subagent produced no output)".into()
        } else {
            content
        };

        (
            final_result,
            DelegateData {
                prompt_tokens,
                completion_tokens,
                cost,
                msgs,
                subagent_id: sub_id,
                tool_call_id: tc_id,
            },
        )
    })
}

fn extract_final_content(msgs: &[Message]) -> String {
    for msg in msgs.iter().rev() {
        if msg.role == Role::Assistant && !msg.content.is_empty() {
            return msg.content.clone();
        }
    }
    String::new()
}

pub(super) fn build_delegate_def(
    names: &[String],
    descriptions: &HashMap<String, String>,
) -> ToolDef {
    let mut desc = String::from("The subagent to delegate to. Available: ");
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            desc.push_str(" | ");
        }
        desc.push_str(name);
        if let Some(d) = descriptions.get(name)
            && !d.is_empty()
        {
            desc.push_str(": ");
            desc.push_str(d);
        }
    }

    let params = crate::tools::object_schema(
        &[
            (
                "subagent",
                serde_json::json!({
                    "type": "string", "enum": names, "description": desc
                }),
            ),
            (
                "task",
                serde_json::json!({
                    "type": "string",
                    "description": "The task for the subagent. Be specific and include all necessary details."
                }),
            ),
            (
                "context",
                serde_json::json!({
                    "type": "string",
                    "description": "Additional context (file contents, requirements, constraints). Optional."
                }),
            ),
        ],
        &["subagent", "task"],
    );

    ToolDef {
        def_type: "function".into(),
        function: crate::message::ToolDefFunction {
            name: DELEGATE_TOOL_NAME.into(),
            description: "Delegate a task to a subagent with its own model and tools. The subagent runs to completion and returns its result. Use for complex coding tasks or research that would benefit from a specialized model.".into(),
            parameters: params,
        },
    }
}
