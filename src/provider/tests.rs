use super::*;
use crate::message::{Message, Role};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_provider_error_retryable() {
    assert!(
        ProviderError {
            status_code: 429,
            body: "rate limit".into()
        }
        .retryable()
    );
    assert!(
        ProviderError {
            status_code: 500,
            body: "internal".into()
        }
        .retryable()
    );
    assert!(
        ProviderError {
            status_code: 503,
            body: "unavailable".into()
        }
        .retryable()
    );
    assert!(
        !ProviderError {
            status_code: 400,
            body: "bad request".into()
        }
        .retryable()
    );
    assert!(
        !ProviderError {
            status_code: 404,
            body: "not found".into()
        }
        .retryable()
    );
}

#[test]
fn test_provider_error_display() {
    let err = ProviderError {
        status_code: 500,
        body: "server error".into(),
    };
    assert_eq!(format!("{}", err), "unexpected status 500: server error");
}

#[test]
fn test_client_pricing() {
    let c = Client::new(
        "http://localhost",
        "m",
        "k",
        ModelProfile {
            input_price: 3.0,
            cached_input_price: 0.3,
            output_price: 15.0,
            ..Default::default()
        },
    );
    let (input, cached, output) = c.pricing();
    assert_eq!(input, 3.0);
    assert_eq!(cached, 0.3);
    assert_eq!(output, 15.0);
}

#[test]
fn test_apply_profile_sets_cache() {
    let mut req = ChatRequest::default();
    apply_profile(
        &mut req,
        &ModelProfile {
            prompt_cache: true,
            cache_ttl: "1h".to_string(),
            open_router: false,
            reasoning_effort: "high".to_string(),
            ..Default::default()
        },
    );
    assert_eq!(req.reasoning_effort, "high");
    let cc = req.cache_control.expect("cache_control");
    assert_eq!(cc.cc_type, "ephemeral");
    assert_eq!(cc.ttl, "1h");
}

#[test]
fn test_apply_profile_openrouter_reasoning() {
    let mut req = ChatRequest::default();
    apply_profile(
        &mut req,
        &ModelProfile {
            open_router: true,
            reasoning_effort: "medium".to_string(),
            prompt_cache: false,
            ..Default::default()
        },
    );
    assert!(req.cache_control.is_none());
    assert!(req.reasoning_effort.is_empty());
    assert_eq!(req.reasoning.as_ref().unwrap().effort, "medium");
}

#[test]
fn test_client_model_and_context() {
    let c = Client::new(
        "http://localhost",
        "gpt-4",
        "k",
        ModelProfile {
            context_window: 128000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );
    assert_eq!(c.model(), "gpt-4");
    assert_eq!(c.context_window(), 128000);
}

#[tokio::test]
async fn test_chat_request_json_shape() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n"),
        )
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "test-model",
        "test-key",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![Message {
            role: Role::User,
            content: "hi".to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let rx = c.chat(req).await.unwrap();

    drop(rx);
}

#[tokio::test]
async fn test_chat_sets_stream_options() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "test-model",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 100,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let rx = c.chat(req).await.unwrap();
    drop(rx);
}

#[tokio::test]
async fn test_chat_non_200_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "test-model",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let result = c.chat(req).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().status_code, 500);
}

#[tokio::test]
async fn test_chat_openrouter_reasoning_config() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "openai/gpt-4o",
        "k",
        ModelProfile {
            context_window: 128000,
            max_output_tokens: 4096,
            reasoning_effort: "high".to_string(),
            open_router: true,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let rx = c.chat(req).await.unwrap();
    drop(rx);
}

#[tokio::test]
async fn test_chat_non_openrouter_thinking_config() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "claude-sonnet-4-20250514",
        "k",
        ModelProfile {
            context_window: 200000,
            max_output_tokens: 4096,
            reasoning_effort: "high".to_string(),
            prompt_cache: true,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let rx = c.chat(req).await.unwrap();
    drop(rx);
}

#[tokio::test]
async fn test_sse_text_delta() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n"),
        )
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got_text = false;
    while let Some(ev) = rx.recv().await {
        if matches!(ev, StreamEvent::TextDelta { delta } if delta == "hello") {
            got_text = true;
        }
    }
    assert!(got_text, "expected text_delta with 'hello'");
}

#[tokio::test]
async fn test_sse_reasoning_content_delta() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking...\"}}]}\n\n",
        ))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got = false;
    while let Some(ev) = rx.recv().await {
        if matches!(ev, StreamEvent::ReasoningDelta { delta } if delta == "thinking...") {
            got = true;
        }
    }
    assert!(got);
}

#[tokio::test]
async fn test_sse_reasoning_delta_fallback() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("data: {\"choices\":[{\"delta\":{\"reasoning\":\"think\"}}]}\n\n"),
        )
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got = false;
    while let Some(ev) = rx.recv().await {
        if matches!(ev, StreamEvent::ReasoningDelta { delta } if delta == "think") {
            got = true;
        }
    }
    assert!(got);
}

#[tokio::test]
async fn test_sse_reasoning_details_prefer_over_fallback() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"reasoning\":\"fallback\",\"reasoning_content\":\"fallback\",\"reasoning_details\":[{\"type\":\"reasoning.text\",\"index\":0,\"text\":\"detail\"}]}}]}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut deltas: Vec<String> = Vec::new();
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::ReasoningDelta { delta } = ev {
            deltas.push(delta);
        }
    }
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0], "detail");
}

#[tokio::test]
async fn test_sse_tool_call_accumulation() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"{\\\"cmd\"}}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\":\\\"ls\\\"}\"}]}}]}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got_tool_call = false;
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::ToolCall { call } = &ev
            && call.name == "bash"
        {
            got_tool_call = true;
        }
    }
    assert!(got_tool_call);
}

#[tokio::test]
async fn test_sse_tool_call_emitted_after_done_terminator() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"{\\\"cmd\"}}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\":\\\"ls\\\"}\"}}]}}]}\n\ndata: [DONE]\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got_tool_call = false;
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::ToolCall { call } = &ev
            && call.name == "bash"
            && call.arguments == "{\"cmd\":\"ls\"}"
        {
            got_tool_call = true;
        }
    }
    assert!(
        got_tool_call,
        "tool_call event dropped when stream ends with [DONE]"
    );
}

#[tokio::test]
async fn test_sse_finish_reason() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            ),
        )
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got = false;
    while let Some(ev) = rx.recv().await {
        if matches!(ev, StreamEvent::FinishReason { reason } if reason == "stop") {
            got = true;
        }
    }
    assert!(got);
}

#[tokio::test]
async fn test_sse_usage() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got = false;
    while let Some(ev) = rx.recv().await {
        if matches!(&ev, StreamEvent::Usage { usage } if usage.prompt_tokens == 10) {
            got = true;
        }
    }
    assert!(got);
}

#[tokio::test]
async fn test_sse_usage_cached_tokens_fallback() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"usage\":{\"prompt_tokens\":15,\"completion_tokens\":8,\"total_tokens\":23,\"prompt_tokens_details\":{\"cached_tokens\":9}}}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    while let Some(ev) = rx.recv().await {
        if let StreamEvent::Usage { usage: u } = ev {
            assert_eq!(u.prompt_cache_hit_tokens, 9);
            assert!(u.prompt_tokens_details.is_some());
            assert_eq!(u.prompt_tokens_details.unwrap().cached_tokens, 9);
            return;
        }
    }
    panic!("expected usage event");
}

#[tokio::test]
async fn test_sse_usage_explicit_field_wins() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15,\"prompt_cache_hit_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":7}}}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    while let Some(ev) = rx.recv().await {
        if let StreamEvent::Usage { usage } = ev {
            assert_eq!(usage.prompt_cache_hit_tokens, 3);
            return;
        }
    }
    panic!("expected usage event");
}

#[tokio::test]
async fn test_sse_usage_no_cache_field() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":16,\"total_tokens\":26}}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    while let Some(ev) = rx.recv().await {
        if let StreamEvent::Usage { usage } = ev {
            assert_eq!(usage.prompt_cache_hit_tokens, 0);
            return;
        }
    }
    panic!("expected usage event");
}

#[tokio::test]
async fn test_sse_reasoning_details_text_merge() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"reasoning_details\":[{\"type\":\"text\",\"index\":0,\"text\":\"hello \"}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"reasoning_details\":[{\"type\":\"text\",\"index\":0,\"text\":\"world\"}]}}]}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut merged: Vec<ReasoningDetail> = Vec::new();
    let mut got_delta = false;
    while let Some(ev) = rx.recv().await {
        if matches!(&ev, StreamEvent::ReasoningDelta { .. }) {
            got_delta = true;
        }
        if let StreamEvent::ReasoningDetails { details } = ev {
            merged = details;
        }
    }
    assert!(got_delta);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].detail_type, "text");
    assert_eq!(merged[0].text, "hello world");
}

#[tokio::test]
async fn test_sse_reasoning_details_encrypted() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"reasoning_details\":[{\"type\":\"reasoning.encrypted\",\"index\":0,\"data\":\"encblob\",\"id\":\"r1\",\"format\":\"anthropic-claude-v1\"}]}}]}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut merged = Vec::new();
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::ReasoningDetails { details } = ev {
            merged = details;
        }
    }
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].detail_type, "reasoning.encrypted");
    assert_eq!(merged[0].data, "encblob");
    assert_eq!(merged[0].id, "r1");
    assert_eq!(merged[0].format, "anthropic-claude-v1");
}

#[tokio::test]
async fn test_sse_reasoning_details_no_index_auto_assigns() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"reasoning_details\":[{\"type\":\"reasoning.text\",\"text\":\"a\"},{\"type\":\"reasoning.encrypted\",\"data\":\"x\"}]}}]}\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut merged = Vec::new();
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::ReasoningDetails { details } = ev {
            merged = details;
        }
    }
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].text, "a");
    assert_eq!(merged[1].data, "x");
}

#[tokio::test]
async fn test_sse_done_sentinel() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    while rx.recv().await.is_some() {}
}

#[tokio::test]
async fn test_sse_error_in_chunk() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"error\":{\"code\":500,\"message\":\"Provider disconnected\"}}\n\n",
        ))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got_error = false;
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::Error { error: err } = ev {
            got_error = true;
            assert!(err.body.contains("500"));
            assert!(err.body.contains("Provider disconnected"));
        }
    }
    assert!(got_error);
}

#[tokio::test]
async fn test_sse_tool_call_trailing_buffer_without_newline() {
    let mock_server = MockServer::start().await;

    // Final tool-call chunk has no trailing newline; must still be parsed.
    // Args arrive as two delta fragments: {"cmd"  +  ":"ls"}  =>  {"cmd":"ls"}
    let body = concat!(
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"bash","arguments":"{\"cmd"}}]}}]}"#,
        "\n\n",
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\"ls\"}"}}]}}]}"#,
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&mock_server)
        .await;

    let c = Client::new(
        &mock_server.uri(),
        "m",
        "k",
        ModelProfile {
            context_window: 8000,
            max_output_tokens: 4096,
            ..Default::default()
        },
    );

    let req = ChatRequest {
        model: String::new(),
        messages: vec![],
        tools: vec![],
        stream: false,
        max_tokens: 0,
        stream_options: None,
        reasoning_effort: String::new(),
        reasoning: None,
        cache_control: None,
    };

    let mut rx = c.chat(req).await.unwrap();

    let mut got = false;
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::ToolCall { call } = &ev
            && call.name == "bash"
            && call.arguments == "{\"cmd\":\"ls\"}"
        {
            got = true;
        }
    }
    assert!(
        got,
        "tool call args lost when final SSE line has no trailing newline"
    );
}
