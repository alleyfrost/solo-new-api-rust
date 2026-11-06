use axum::http::{HeaderMap, HeaderValue};

#[test]
fn builds_url_with_and_without_v1_suffix() {
    assert_eq!(
        solo_new_api_rust::upstream::build_url("https://a.cn", "/v1/chat/completions", 1),
        "https://a.cn/v1/chat/completions"
    );
    assert_eq!(
        solo_new_api_rust::upstream::build_url("https://a.cn/", "/v1/messages", 14),
        "https://a.cn/v1/messages"
    );
    assert_eq!(
        solo_new_api_rust::upstream::build_url("https://a.cn/v1", "/v1/chat/completions", 1),
        "https://a.cn/v1/chat/completions"
    );
}

#[test]
fn forwards_all_headers_and_sets_auth() {
    let mut client = HeaderMap::new();
    client.insert("x-trace-id", HeaderValue::from_static("t1"));
    client.insert("x-drop", HeaderValue::from_static("d"));
    let overrides = r#"{"*":true,"re:^X-Trace-.*$":true,"X-Drop":null,"X-Foo":"v-{client_header:x-trace-id}","Authorization":"Bearer {api_key}"}"#;
    let out = solo_new_api_rust::upstream::rewrite_headers(&client, "sk-up", Some(overrides));
    assert_eq!(out.get("x-trace-id").unwrap(), "t1");
    assert!(out.get("x-drop").is_none());
    assert_eq!(out.get("x-foo").unwrap(), "v-t1");
    assert_eq!(out.get("authorization").unwrap(), "Bearer sk-up");
    assert_eq!(out.get("accept-encoding").unwrap(), "identity");
}

#[test]
fn regex_only_forwards_matching_when_no_wildcard() {
    let mut client = HeaderMap::new();
    client.insert("x-trace-id", HeaderValue::from_static("t1"));
    client.insert("x-other", HeaderValue::from_static("o"));
    let out = solo_new_api_rust::upstream::rewrite_headers(
        &client,
        "sk-up",
        Some(r#"{"re:^X-Trace-.*$":true}"#),
    );
    assert!(out.get("x-trace-id").is_some());
    assert!(out.get("x-other").is_none());
    assert_eq!(out.get("authorization").unwrap(), "Bearer sk-up");
}
