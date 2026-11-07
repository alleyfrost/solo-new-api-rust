use axum::http::{HeaderMap, HeaderValue, StatusCode};
use solo_new_api_rust::relay::{SseTap, Usage, client_ip, map_model, usage_from_json};

#[test]
fn model_mapping_follows_chain_and_stops_cycles() {
    assert_eq!(map_model("a", Some(r#"{"a":"b","b":"c"}"#)), "c");
    assert_eq!(map_model("x", Some(r#"{"a":"b"}"#)), "x");
    assert_eq!(map_model("a", Some(r#"{"a":"b","b":"a"}"#)), "a");
    assert_eq!(map_model("a", None), "a");
}

#[test]
fn usage_reads_openai_and_anthropic_names() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"usage":{"prompt_tokens":3,"completion_tokens":4}}"#).unwrap();
    assert_eq!(
        usage_from_json(&v),
        Some(Usage {
            prompt_tokens: 3,
            completion_tokens: 4,
            ..Usage::default()
        })
    );
    let v: serde_json::Value =
        serde_json::from_str(r#"{"usage":{"input_tokens":5,"output_tokens":6}}"#).unwrap();
    assert_eq!(
        usage_from_json(&v),
        Some(Usage {
            prompt_tokens: 5,
            completion_tokens: 6,
            ..Usage::default()
        })
    );
    assert_eq!(usage_from_json(&serde_json::json!({"id": "x"})), None);
}

#[test]
fn usage_reads_cache_hit_fields() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":7}}}"#,
    )
    .unwrap();
    let u = usage_from_json(&v).unwrap();
    assert_eq!(u.cached_tokens, 7);

    let v: serde_json::Value = serde_json::from_str(
        r#"{"usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":8,"cache_creation_input_tokens":3}}"#,
    )
    .unwrap();
    let u = usage_from_json(&v).unwrap();
    assert_eq!(u.cached_tokens, 8);
    assert_eq!(u.cache_write_tokens, 3);
}

#[test]
fn sse_tap_handles_split_chunks_and_drops_usage_only_event() {
    let mut tap = SseTap::new(false);
    let mut out = Vec::new();
    out.extend(tap.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n"));
    out.extend(tap.feed(b"data: {\"choices\":[],\"usa"));
    out.extend(tap.feed(
        b"ge\":{\"prompt_tokens\":2,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n",
    ));
    out.extend(tap.finish());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\"content\":\"hi\""));
    assert!(
        !text.contains("prompt_tokens"),
        "客户端未要求 usage 时必须丢弃 usage-only 事件"
    );
    assert!(text.contains("[DONE]"));
    assert_eq!(
        tap.usage(),
        Usage {
            prompt_tokens: 2,
            completion_tokens: 3,
            ..Usage::default()
        }
    );
}

#[test]
fn sse_tap_keeps_usage_when_requested() {
    let mut tap = SseTap::new(true);
    let out = tap.feed(
        b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
    );
    assert!(String::from_utf8(out).unwrap().contains("prompt_tokens"));
}

#[test]
fn client_ip_only_trusts_forwarded_headers_when_enabled() {
    let mut h = HeaderMap::new();
    h.insert(
        "x-forwarded-for",
        HeaderValue::from_static("1.2.3.4, 5.6.7.8"),
    );
    assert_eq!(client_ip(&h, true), "1.2.3.4");
    assert_eq!(client_ip(&h, false), "unknown");
    let _ = StatusCode::OK;
}
