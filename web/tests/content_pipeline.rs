use serde_json::json;
use web::config::WebConfig;
use web::fetch::execute_fetch;
use web::schemas::FetchPayload;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn payload(j: serde_json::Value) -> FetchPayload {
    serde_json::from_value(j).unwrap()
}

const PAGE: &str = r#"<html><head><title>T</title></head><body>
<nav class="nav"><a href="/home">Home</a></nav>
<article><p>The quick brown fox jumps over the lazy dog in a long real paragraph of content.</p>
<img src="/pic.png" alt="pic"></article>
<footer class="footer">footer junk</footer></body></html>"#;

async fn serve(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/p"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(PAGE, "text/html; charset=utf-8"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn content_filter_prunes_boilerplate_into_body() {
    let server = MockServer::start().await;
    serve(&server).await;
    let out = execute_fetch(
        payload(json!({
            "url": format!("{}/p", server.uri()),
            "format": "markdown",
            "content_filter": { "type": "pruning" }
        })),
        &WebConfig::default(),
    )
    .await;
    let body = out["body"].as_str().unwrap();
    assert!(body.contains("quick brown fox"));
    assert!(!body.contains("Home"));
    assert!(!body.contains("footer junk"));
    assert_eq!(out["transformed"], "markdown");
}

#[tokio::test]
async fn include_links_and_media_populate_envelope() {
    let server = MockServer::start().await;
    serve(&server).await;
    let base = server.uri();
    let out = execute_fetch(
        payload(json!({
            "url": format!("{}/p", base),
            "format": "markdown",
            "include_links": true,
            "include_media": true
        })),
        &WebConfig::default(),
    )
    .await;
    assert_eq!(
        out["links"]["internal"][0]["href"],
        format!("{}/home", base)
    );
    assert_eq!(
        out["media"]["images"][0]["src"],
        format!("{}/pic.png", base)
    );
    assert_eq!(out["media"]["images"][0]["alt"], "pic");
}

#[tokio::test]
async fn excluded_tags_and_target_elements_scope_body() {
    let server = MockServer::start().await;
    serve(&server).await;
    let out = execute_fetch(
        payload(json!({
            "url": format!("{}/p", server.uri()),
            "format": "markdown",
            "target_elements": ["article"],
            "excluded_tags": ["nav", "footer"]
        })),
        &WebConfig::default(),
    )
    .await;
    let body = out["body"].as_str().unwrap();
    assert!(body.contains("quick brown fox"));
    assert!(!body.contains("Home"));
    assert!(!body.contains("footer junk"));
}

#[tokio::test]
async fn target_elements_alone_scopes_body() {
    let server = MockServer::start().await;
    serve(&server).await;
    let out = execute_fetch(
        payload(json!({
            "url": format!("{}/p", server.uri()),
            "format": "markdown",
            "target_elements": ["article"]
        })),
        &WebConfig::default(),
    )
    .await;
    let body = out["body"].as_str().unwrap();
    assert!(body.contains("quick brown fox"));
    assert!(!body.contains("Home")); // nav is outside <article>
    assert!(!body.contains("footer junk")); // footer is outside <article>
    assert!(out.get("links").is_none()); // not requested
}

#[tokio::test]
async fn excluded_tags_alone_drops_boilerplate() {
    let server = MockServer::start().await;
    serve(&server).await;
    let out = execute_fetch(
        payload(json!({
            "url": format!("{}/p", server.uri()),
            "format": "markdown",
            "excluded_tags": ["nav", "footer"]
        })),
        &WebConfig::default(),
    )
    .await;
    let body = out["body"].as_str().unwrap();
    assert!(body.contains("quick brown fox"));
    assert!(!body.contains("Home"));
    assert!(!body.contains("footer junk"));
}

#[tokio::test]
async fn backward_compat_no_new_fields_unchanged() {
    let server = MockServer::start().await;
    serve(&server).await;
    let out = execute_fetch(
        payload(json!({ "url": format!("{}/p", server.uri()), "format": "markdown" })),
        &WebConfig::default(),
    )
    .await;
    let body = out["body"].as_str().unwrap();
    // full page rendered (boilerplate retained), no links/media keys
    assert!(body.contains("quick brown fox"));
    assert!(body.contains("Home"));
    assert!(out.get("links").is_none());
    assert!(out.get("media").is_none());
    assert_eq!(out["transformed"], "markdown");
}
