use serde_json::json;
use web::config::WebConfig;
use web::fetch::execute_fetch;
use web::schemas::FetchPayload;
use web::ssrf::{check_target, parse_target, SsrfPolicy};

fn payload(j: serde_json::Value) -> FetchPayload {
    serde_json::from_value(j).unwrap()
}

#[tokio::test]
async fn refuses_when_resolution_is_private() {
    // A hostname that resolves to loopback; with allow_loopback=false it must be refused.
    let t = parse_target("http://localhost/").unwrap();
    let rej = check_target(&t, &SsrfPolicy { allow_loopback: false }).await.unwrap_err();
    assert_eq!(rej.code, "blocked_host");
}

#[tokio::test]
async fn private_literal_blocked_end_to_end() {
    for url in [
        "http://10.0.0.1/",
        "http://192.168.1.1/",
        "http://[::1]/",
        "http://[::ffff:a9fe:a9fe]/",
    ] {
        let out = execute_fetch(payload(json!({ "url": url })), &WebConfig { allow_loopback: false, ..WebConfig::default() }).await;
        assert_eq!(out["error"], "blocked_host", "{url} must be blocked");
    }
}

#[tokio::test]
async fn pinning_dials_validated_ip() {
    // wiremock binds to 127.0.0.1; with allow_loopback=true the request must
    // succeed AND be served by the pinned loopback address (not re-resolved).
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pinned"))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/ok", server.uri()) })), &WebConfig::default()).await;
    assert_eq!(out["body"], "pinned");
}
