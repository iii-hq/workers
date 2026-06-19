use serde_json::json;
use web::config::WebConfig;
use web::fetch::execute_fetch;
use web::schemas::FetchPayload;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn payload(j: serde_json::Value) -> FetchPayload {
    serde_json::from_value(j).unwrap()
}

#[tokio::test]
async fn basic_get_returns_body_and_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/hi"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/hi", server.uri()) })), &WebConfig::default()).await;
    assert_eq!(out["ok"], true);
    assert_eq!(out["status"], 200);
    assert_eq!(out["body"], "hello");
}

#[tokio::test]
async fn json_mode_autoparses_when_not_truncated() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/j"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"a\":1}"))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/j", server.uri()), "response_format": "json" })), &WebConfig::default()).await;
    assert_eq!(out["json"]["a"], 1);
    assert_eq!(out["body"], "{\"a\":1}"); // raw text still present
}

#[tokio::test]
async fn json_mode_sets_parse_error_on_bad_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/bad"))
        .respond_with(ResponseTemplate::new(200).set_body_string("nope"))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/bad", server.uri()), "response_format": "json" })), &WebConfig::default()).await;
    assert!(out.get("json").is_none());
    assert!(out["parse_error"].is_string());
}

#[tokio::test]
async fn byte_cap_truncates() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(1000)))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/big", server.uri()), "max_bytes": 10 })), &WebConfig::default()).await;
    assert_eq!(out["bytes_truncated"], true);
    assert_eq!(out["body"].as_str().unwrap().len(), 10);
}

#[tokio::test]
async fn redirect_followed_and_chain_recorded() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/from"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/to"))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/to"))
        .respond_with(ResponseTemplate::new(200).set_body_string("arrived"))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/from", server.uri()) })), &WebConfig::default()).await;
    assert_eq!(out["body"], "arrived");
    assert!(out["redirect_chain"].as_array().unwrap().len() == 1);
}

#[tokio::test]
async fn too_many_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/loop"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
        .mount(&server).await;

    let cfg = WebConfig { max_redirects: 2, ..WebConfig::default() };
    let out = execute_fetch(payload(json!({ "url": format!("{}/loop", server.uri()) })), &cfg).await;
    assert_eq!(out["error"], "too_many_redirects");
}

#[tokio::test]
async fn three_xx_without_location_returns_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/nolctn"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/nolctn", server.uri()) })), &WebConfig::default()).await;
    assert_eq!(out["ok"], true);
    assert_eq!(out["status"], 304);
}

#[tokio::test]
async fn page_mode_markdown_transforms_html() {
    let server = MockServer::start().await;
    // Use set_body_raw so wiremock sends the correct Content-Type header.
    // set_body_string always sets mime to "text/plain" and that field wins
    // over any insert_header("content-type") call in generate_response().
    Mock::given(method("GET")).and(path("/page"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_raw("<h1>Title</h1><p>Body</p>", "text/html; charset=utf-8"))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/page", server.uri()), "format": "markdown" })), &WebConfig::default()).await;
    assert_eq!(out["transformed"], "markdown");
    assert!(out["body"].as_str().unwrap().contains("Title"));
    assert_eq!(out["content_type"], "text/html");
}

#[tokio::test]
async fn image_envelope_deserializes_to_content_blocks() {
    // 1x1 transparent PNG
    let png: &[u8] = &[
        0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x00,0x00,0x0D,0x49,0x48,0x44,0x52,
        0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x01,0x08,0x06,0x00,0x00,0x00,0x1F,0x15,0xC4,
        0x89,0x00,0x00,0x00,0x0A,0x49,0x44,0x41,0x54,0x78,0x9C,0x63,0x00,0x01,0x00,0x00,
        0x05,0x00,0x01,0x0D,0x0A,0x2D,0xB4,0x00,0x00,0x00,0x00,0x49,0x45,0x4E,0x44,0xAE,0x42,0x60,0x82,
    ];
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/img"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(png.to_vec()))
        .mount(&server).await;

    let out = execute_fetch(payload(json!({ "url": format!("{}/img", server.uri()), "format": "markdown" })), &WebConfig::default()).await;
    // shape: { content: [image, text], details }
    let content = out["content"].as_array().expect("content array");
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["mime"], "image/png");
    assert_eq!(content[1]["type"], "text");
    assert_eq!(out["details"]["ok"], true);
}

#[tokio::test]
async fn blocked_host_for_metadata_ip() {
    let out = execute_fetch(payload(json!({ "url": "http://169.254.169.254/latest/meta-data/" })), &WebConfig::default()).await;
    assert_eq!(out["error"], "blocked_host");
}
