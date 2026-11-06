use solo_new_api_rust::retry::{backoff, decide, max_retry_from_options, RetryDecision};
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn retries_rate_limits_server_errors_auth_and_network() {
    for status in [
        Some(408),
        Some(429),
        Some(401),
        Some(403),
        Some(500),
        Some(503),
        None,
    ] {
        assert_eq!(
            decide(status, 0, 10),
            RetryDecision::Retry,
            "status {status:?}"
        );
    }
}

#[test]
fn stops_on_client_errors_and_budget_exhaustion() {
    for status in [Some(400), Some(404), Some(413), Some(422)] {
        assert_eq!(
            decide(status, 0, 10),
            RetryDecision::Stop,
            "status {status:?}"
        );
    }
    assert_eq!(decide(Some(503), 10, 10), RetryDecision::Stop);
}

#[test]
fn rate_limit_rotates_immediately_and_server_errors_back_off() {
    assert_eq!(backoff(Some(429), 3), Duration::ZERO);
    assert_eq!(backoff(Some(401), 3), Duration::ZERO);
    assert_eq!(backoff(Some(500), 0), Duration::from_millis(100));
    assert_eq!(backoff(Some(500), 1), Duration::from_millis(200));
    assert_eq!(backoff(Some(500), 9), Duration::from_millis(1000));
}

#[test]
fn retry_budget_reads_option_and_clamps() {
    let mut options = HashMap::new();
    assert_eq!(max_retry_from_options(&options), 3);
    options.insert("RetryTimes".into(), "10".into());
    assert_eq!(max_retry_from_options(&options), 10);
    options.insert("RetryTimes".into(), "99".into());
    assert_eq!(max_retry_from_options(&options), 10);
    options.insert("RetryTimes".into(), "0".into());
    assert_eq!(max_retry_from_options(&options), 0);
}
