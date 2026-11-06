use serde_json::json;
use switchyard_translation::{StreamTranslationState, TranslationEngine, TranslationPolicy, WireFormat};

fn engine() -> TranslationEngine {
    TranslationEngine::default()
}

#[test]
fn anthropic_request_translates_to_chat() {
    let request = json!({
        "model": "m1",
        "max_tokens": 100,
        "messages": [{"role": "user", "content": "hi"}]
    });
    let out = engine()
        .translate_request(
            WireFormat::AnthropicMessages,
            WireFormat::OpenAiChat,
            &request,
            &TranslationPolicy::default(),
        )
        .unwrap();
    assert_eq!(out.body["model"], "m1");
    assert_eq!(out.body["messages"][0]["role"], "user");
    assert_eq!(out.body["messages"][0]["content"], "hi");
}

#[test]
fn responses_request_translates_to_chat() {
    let request = json!({"model": "m1", "input": "hi"});
    let out = engine()
        .translate_request(
            WireFormat::OpenAiResponses,
            WireFormat::OpenAiChat,
            &request,
            &TranslationPolicy::default(),
        )
        .unwrap();
    assert_eq!(out.body["model"], "m1");
    assert_eq!(out.body["messages"][0]["content"], "hi");
}

#[test]
fn chat_response_translates_to_anthropic() {
    let response = json!({
        "id": "x",
        "object": "chat.completion",
        "model": "m1",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hi"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 4}
    });
    let out = engine()
        .translate_response(
            WireFormat::OpenAiChat,
            WireFormat::AnthropicMessages,
            &response,
            &TranslationPolicy::default(),
        )
        .unwrap();
    assert_eq!(out.body["type"], "message");
    assert_eq!(out.body["role"], "assistant");
    assert_eq!(out.body["content"][0]["type"], "text");
    assert_eq!(out.body["content"][0]["text"], "hi");
    assert_eq!(out.body["usage"]["input_tokens"], 3);
    assert_eq!(out.body["usage"]["output_tokens"], 4);
}

#[test]
fn chat_stream_translates_to_anthropic_events() {
    let e = engine();
    let mut state = StreamTranslationState::new(WireFormat::OpenAiChat, WireFormat::AnthropicMessages);
    let mut events = Vec::new();
    for chunk in [
        json!({"id":"x","model":"m1","choices":[{"index":0,"delta":{"role":"assistant","content":"hi"},"finish_reason":null}]}),
        json!({"id":"x","model":"m1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2}}),
    ] {
        events.extend(
            e.translate_event(
                &mut state,
                WireFormat::OpenAiChat,
                WireFormat::AnthropicMessages,
                &chunk,
            )
            .unwrap(),
        );
    }
    events.extend(
        e.finish_stream(&mut state, WireFormat::AnthropicMessages)
            .unwrap(),
    );
    let types: Vec<&str> = events
        .iter()
        .filter_map(|v| v.get("type").and_then(|t| t.as_str()))
        .collect();
    assert!(types.contains(&"message_start"));
    assert!(types.contains(&"content_block_delta"));
    assert_eq!(types.last().copied(), Some("message_stop"));
    let text: String = events
        .iter()
        .filter(|v| v["type"] == "content_block_delta")
        .filter_map(|v| v["delta"]["text"].as_str())
        .collect();
    assert_eq!(text, "hi");
}

#[test]
fn chat_response_translates_to_responses() {
    let response = json!({
        "id": "x",
        "object": "chat.completion",
        "model": "m1",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hi"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 4}
    });
    let out = engine()
        .translate_response(
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            &response,
            &TranslationPolicy::default(),
        )
        .unwrap();
    let text: String = out.body["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"].as_array())
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect();
    assert_eq!(text, "hi");
    assert_eq!(out.body["usage"]["input_tokens"], 3);
    assert_eq!(out.body["usage"]["output_tokens"], 4);
}

#[test]
fn chat_request_translates_to_anthropic() {
    let request = json!({
        "model": "m1",
        "messages": [{"role": "user", "content": "hi"}]
    });
    let out = engine()
        .translate_request(
            WireFormat::OpenAiChat,
            WireFormat::AnthropicMessages,
            &request,
            &TranslationPolicy::default(),
        )
        .unwrap();
    assert_eq!(out.body["model"], "m1");
    assert_eq!(out.body["messages"][0]["content"], "hi");
    assert!(out.body["max_tokens"].as_i64().unwrap_or(0) > 0);
}
