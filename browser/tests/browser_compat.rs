#![cfg(feature = "scrapling-compat")]

use std::path::PathBuf;
use std::time::Duration;

use browser::config::{SecurityMode, WorkerConfig};
use browser::scrapling::raw_browser::{RawBrowser, RawBrowserOptions};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn origin() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = [0; 8192];
                let Ok(size) = socket.read(&mut request).await else {
                    return;
                };
                let Some(path) = std::str::from_utf8(&request[..size])
                    .ok()
                    .and_then(|request| request.split_whitespace().nth(1))
                else {
                    return;
                };
                let (status, headers, body) = match path {
                    "/plain" => (
                        "206 Partial Content",
                        "Content-Type: text/plain; charset=iso-8859-1\r\nX-Test: plain\r\n",
                        b"caf\xe9".to_vec(),
                    ),
                    "/visual" => (
                        "200 OK",
                        "Content-Type: text/html; charset=utf-8\r\n",
                        include_bytes!("corpus/browser_visual.html").to_vec(),
                    ),
                    _ => (
                        "200 OK",
                        "Content-Type: text/html; charset=utf-8\r\nX-Test: one\r\nX-Test: two\r\nSet-Cookie: sid=abc; Path=/\r\n",
                        br#"<!doctype html><html><head><title>T</title></head><body><h1>initial</h1><div id="fp"></div><script>document.querySelector('h1').textContent='rendered';document.querySelector('#fp').textContent=[navigator.language,Intl.DateTimeFormat().resolvedOptions().timeZone,devicePixelRatio,innerWidth,innerHeight].join('|')</script></body></html>"#.to_vec(),
                    ),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\n{headers}Date: Wed, 12 Aug 2026 16:00:00 GMT\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if socket.write_all(response.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(&body).await;
                }
            });
        }
    });
    (format!("http://{address}"), task)
}

fn assert_image_close(label: &str, actual: &[u8], expected: &[u8]) {
    let actual = image::load_from_memory(actual).unwrap().to_rgba8();
    let expected = image::load_from_memory(expected).unwrap().to_rgba8();
    assert_eq!(actual.dimensions(), expected.dimensions());
    let mut total_error = 0u64;
    let mut within_three = 0usize;
    let mut maximum = 0u8;
    for (actual, expected) in actual.as_raw().iter().zip(expected.as_raw()) {
        let error = actual.abs_diff(*expected);
        total_error += u64::from(error);
        within_three += usize::from(error <= 3);
        maximum = maximum.max(error);
    }
    let channels = actual.as_raw().len();
    let mean = total_error as f64 / channels as f64;
    assert!(mean <= 0.5, "{label}: mean error {mean}");
    assert!(
        within_three * 1_000 >= channels * 999,
        "{label}: {} of {channels} channels within 3",
        within_three
    );
    assert!(maximum <= 12, "{label}: maximum error {maximum}");
}

fn config() -> WorkerConfig {
    let mut config = WorkerConfig::default();
    config.scrapling.security_mode = SecurityMode::Compat;
    config.scrapling.chromium_executable = PathBuf::from(
        std::env::var_os("SCRAPLING_CHROMIUM_EXECUTABLE")
            .expect("SCRAPLING_CHROMIUM_EXECUTABLE must name frozen Chrome 148"),
    )
    .display()
    .to_string();
    config
}

#[tokio::test]
async fn browser_response_matches_frozen_dynamic_and_stealth_contracts() {
    let (origin, server) = origin().await;
    for (stealth, viewport) in [(false, "1280|720"), (true, "1920|1080")] {
        let options = RawBrowserOptions::from_payload(&json!({
            "retries": 1,
            "timeout": 5_000,
            "locale": "fr-FR",
            "timezone_id": "Europe/Paris"
        }))
        .unwrap();
        let browser = RawBrowser::start(&config(), &options, stealth, false)
            .await
            .unwrap();
        let page = browser
            .fetch(&format!("{origin}/page"), &options, stealth)
            .await
            .unwrap();
        assert_eq!(page.status, Some(200));
        assert_eq!(page.headers["x-test"], "one, two");
        assert_eq!(page.cookies, serde_json::Map::new());
        assert_eq!(page.encoding.as_deref(), Some("utf-8"));
        assert!(page.html.contains("<h1>rendered</h1>"), "{}", page.html);
        assert!(
            page.html
                .contains(&format!("fr-FR|Europe/Paris|2|{viewport}")),
            "{}",
            page.html
        );

        let plain = browser
            .fetch(&format!("{origin}/plain"), &options, stealth)
            .await
            .unwrap();
        assert_eq!(plain.status, Some(206));
        assert_eq!(plain.encoding.as_deref(), Some("iso-8859-1"));
        assert_eq!(plain.html, "<html><body>cafÃ©</body></html>");
    }
    server.abort();
}

#[tokio::test]
async fn browser_waits_match_the_frozen_timing_contract() {
    let (origin, server) = origin().await;
    let options = RawBrowserOptions::from_payload(&json!({
        "retries": 1,
        "timeout": 5_000,
        "wait": 25
    }))
    .unwrap();
    let browser = RawBrowser::start(&config(), &options, false, false)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    browser
        .fetch(&format!("{origin}/page"), &options, false)
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(25));
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn five_browser_processes_run_within_the_release_envelope() {
    let (origin, server) = origin().await;
    let jobs = (0..5).map(|_| {
        let origin = origin.clone();
        async move {
            let options = RawBrowserOptions::from_payload(&json!({
                "retries": 1,
                "timeout": 10_000
            }))
            .unwrap();
            let browser = RawBrowser::start(&config(), &options, false, false)
                .await
                .unwrap();
            let page = browser
                .fetch(&format!("{origin}/page"), &options, false)
                .await
                .unwrap();
            browser.shutdown().await;
            page
        }
    });
    for page in futures::future::join_all(jobs).await {
        assert_eq!(page.status, Some(200));
        assert!(page.html.contains("<h1>rendered</h1>"));
    }
    server.abort();
}

#[tokio::test]
async fn screenshots_match_the_frozen_wrapper_metrics_and_pixels() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("golden/browser/manifest.json")).unwrap();
    let (origin, server) = origin().await;
    for case in manifest["cases"].as_array().unwrap() {
        let request = &case["request"];
        let stealth = request["fetcher"] == "stealthy";
        let options = RawBrowserOptions::from_payload(request).unwrap();
        let browser = RawBrowser::start(&config(), &options, stealth, false)
            .await
            .unwrap();
        let (content, mime, final_url) = browser
            .screenshot(
                &format!("{origin}/visual"),
                &options,
                stealth,
                request["full_page"].as_bool().unwrap(),
                request["format"].as_str().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mime, case["response"]["mime"]);
        assert_eq!(final_url, format!("{origin}/visual"));
        let expected = case["response"]["content"].as_array().unwrap();
        assert_eq!(content.len(), expected.len());
        for (actual, expected) in content.iter().zip(expected) {
            assert_eq!(actual["type"], expected["type"]);
            if actual["type"] == "image" {
                assert_eq!(actual["mime"], expected["mime"]);
                let actual = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    actual["data"].as_str().unwrap(),
                )
                .unwrap();
                let filename = expected["file"].as_str().unwrap();
                let expected = match filename {
                    "dynamic-viewport-png-1.png" => {
                        include_bytes!("golden/browser/dynamic-viewport-png-1.png").as_slice()
                    }
                    "dynamic-full-png-1.png" => {
                        include_bytes!("golden/browser/dynamic-full-png-1.png").as_slice()
                    }
                    "stealthy-viewport-png-1.png" => {
                        include_bytes!("golden/browser/stealthy-viewport-png-1.png").as_slice()
                    }
                    "stealthy-full-jpeg-1.jpg" => {
                        include_bytes!("golden/browser/stealthy-full-jpeg-1.jpg").as_slice()
                    }
                    name => panic!("unexpected browser fixture {name}"),
                };
                assert_image_close(filename, &actual, expected);
            } else {
                assert_eq!(
                    actual["text"],
                    expected["text"]
                        .as_str()
                        .unwrap()
                        .replace("{origin}", &origin)
                );
            }
        }
    }
    server.abort();
}
