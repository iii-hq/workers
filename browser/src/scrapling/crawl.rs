//! `browser::crawl` — breadth-first crawl from one or more start
//! URLs, extracting per page and streaming the results (crawl.py:73-190).
//!
//! The frontier walk is parameterised over the fetch step, so the whole
//! algorithm — ordering, dedup, the depth and page caps, per-page error
//! isolation — is testable against a canned link graph with no network and no
//! browser. Only the closure passed in at registration does I/O.

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use serde_json::{json, Value};

use crate::config::SecurityMode;
use crate::scrapling::dom;
use crate::scrapling::page::{serialize_page, PageData};

/// The RPC response carries only a sample; the full set goes to the stream.
const SAMPLE_MAX: usize = 10;
/// Ceiling on the per-page politeness delay (see the note where it is read).
const MAX_DOWNLOAD_DELAY_SECS: f64 = 300.0;

#[derive(Debug)]
pub struct CrawlOpts {
    pub start_urls: Vec<String>,
    pub fetcher: String,
    pub allowed_domains: Vec<String>,
    pub same_domain: bool,
    pub max_pages: i64,
    pub max_depth: i64,
    pub concurrency: usize,
    pub download_delay: Duration,
    pub stream_name: Value,
}

impl CrawlOpts {
    /// Defaults are crawl.py's, which are hardcoded there rather than
    /// configurable: 20 pages, depth 2, same-domain on, no delay.
    pub fn from_payload(payload: &Value, max_concurrency: usize) -> Result<Self, String> {
        Self::from_payload_for_mode(payload, max_concurrency, SecurityMode::Safe)
    }

    pub fn from_payload_for_mode(
        payload: &Value,
        max_concurrency: usize,
        mode: SecurityMode,
    ) -> Result<Self, String> {
        let mut start_urls = crawl_strings(payload.get("start_urls"), "decode")?;
        if start_urls.is_empty() {
            if let Some(value) = payload.get("url").filter(|value| json_truthy(value)) {
                if let Some(url) = value.as_str() {
                    start_urls.push(url.to_string());
                } else {
                    return Err(format!(
                        "'{}' object has no attribute 'decode'",
                        python_type(value)
                    ));
                }
            }
        }
        if start_urls.is_empty() {
            return Err("provide `start_urls`".to_string());
        }
        let fetcher = match payload.get("fetcher") {
            None => "http".to_string(),
            Some(Value::String(value)) => value.clone(),
            Some(value) => {
                return Err(format!(
                    "unknown fetcher: {} (use http|stealthy|dynamic)",
                    python_repr(value)
                ))
            }
        };
        if !matches!(fetcher.as_str(), "http" | "stealthy" | "dynamic") {
            return Err(format!(
                "unknown fetcher: {fetcher} (use http|stealthy|dynamic)"
            ));
        }
        let concurrency_ceiling = max_concurrency.max(1).min(i64::MAX as usize) as i64;
        Ok(Self {
            start_urls,
            fetcher,
            allowed_domains: crawl_strings(payload.get("allowed_domains"), "lower")?
                .into_iter()
                // Normalize to the same punycode form host_of/normalize_link
                // produce, so an IDN allow-entry matches IDN candidate hosts.
                .map(|value| ascii_host(&value))
                .collect(),
            same_domain: payload.get("same_domain").is_none_or(json_truthy),
            max_pages: python_int(payload, "max_pages", 20)?,
            max_depth: python_int(payload, "max_depth", 2)?,
            // The caller may ask for less, never for more than the server cap.
            concurrency: python_int(payload, "concurrency", concurrency_ceiling)?
                .clamp(1, concurrency_ceiling) as usize,
            // Clamped for the same reason the fetch timeouts are:
            // `from_secs_f64` panics on an unrepresentable value, and a panic
            // in a detached handler drops the invocation and hangs the caller.
            download_delay: crawl_delay(payload.get("download_delay"), mode)?,
            stream_name: payload
                .get("stream_name")
                .filter(|value| json_truthy(value))
                .cloned()
                .unwrap_or_else(|| json!("browser::crawl")),
        })
    }
}

fn python_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(value) if value.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

pub(crate) fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(value) => format!("'{value}'"),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("'{key}': {}", python_repr(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn crawl_strings(value: Option<&Value>, method: &str) -> Result<Vec<String>, String> {
    let Some(value) = value.filter(|value| json_truthy(value)) else {
        return Ok(Vec::new());
    };
    let values = match value {
        Value::String(value) => return Ok(value.chars().map(String::from).collect()),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value.as_str().map(str::to_string).ok_or_else(|| {
                    format!(
                        "'{}' object has no attribute '{method}'",
                        python_type(value)
                    )
                })
            })
            .collect(),
        Value::Object(values) => Ok(values.keys().cloned().collect()),
        _ => Err(format!("'{}' object is not iterable", python_type(value))),
    }?;
    Ok(values)
}

fn python_int(payload: &Value, key: &str, default: i64) -> Result<i64, String> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(i64::from(*value)),
        Some(Value::Number(value)) => Ok(value
            .as_i64()
            .or_else(|| {
                value
                    .as_u64()
                    .map(|value| value.min(i64::MAX as u64) as i64)
            })
            .or_else(|| value.as_f64().map(|value| value as i64))
            .unwrap_or(default)),
        Some(Value::String(value)) => value
            .trim()
            .parse()
            .map_err(|_| format!("invalid literal for int() with base 10: '{}'", value)),
        Some(value) => Err(format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            python_type(value)
        )),
    }
}

fn crawl_delay(value: Option<&Value>, mode: SecurityMode) -> Result<Duration, String> {
    let value = match value {
        None | Some(Value::Null | Value::Bool(false)) => 0.0,
        Some(Value::Bool(true)) => 1.0,
        Some(Value::Number(value)) => value.as_f64().unwrap_or_default(),
        Some(Value::String(value)) if value.is_empty() => 0.0,
        Some(Value::String(value)) => value
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("could not convert string to float: '{value}'"))?,
        Some(Value::Array(value)) if value.is_empty() => 0.0,
        Some(Value::Object(value)) if value.is_empty() => 0.0,
        Some(Value::Array(_)) => {
            return Err("float() argument must be a string or a real number, not 'list'".into())
        }
        Some(Value::Object(_)) => {
            return Err("float() argument must be a string or a real number, not 'dict'".into())
        }
    };
    let value = if value <= 0.0 {
        0.0
    } else if mode == SecurityMode::Safe {
        value.min(MAX_DOWNLOAD_DELAY_SECS)
    } else {
        value
    };
    Duration::try_from_secs_f64(value).map_err(|_| "timestamp too large to convert".to_string())
}

/// Host with a leading `www.` folded away, so `example.com` and
/// `www.example.com` count as one site (crawl.py `_same_site`).
fn fold_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// Same site if either host is the other, or a subdomain of it, after folding
/// `www.` — the relationship holds in both directions, as in the reference.
pub fn same_site(a: &str, b: &str) -> bool {
    let (a, b) = (fold_www(a), fold_www(b));
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

/// Normalize a bare domain to its lowercase ASCII/punycode form, matching
/// what `host_of` yields for a full URL. Falls back to the lowercased input
/// for anything url can't parse as a host.
fn ascii_host(domain: &str) -> String {
    url::Url::parse(&format!("http://{domain}"))
        .ok()
        .and_then(|url| url.host_str().map(str::to_lowercase))
        .unwrap_or_else(|| domain.to_lowercase())
}

pub fn host_of(raw: &str) -> Option<String> {
    // Use url::Url's host, not a raw netloc scan: extracted links are
    // re-serialized through url::Url (punycode), so an IDN seed scanned raw
    // (`münchen.example`) would never match a candidate (`xn--mnchen-3ya…`)
    // and the crawl would follow zero links. Normalizing both sides the same
    // way keeps IDN crawls working. The non-default port is kept, matching
    // urllib's netloc (so `e.com:8443` and `e.com:9443` stay distinct sites).
    let url = url::Url::parse(raw).ok()?;
    let host = url.host_str()?.to_lowercase();
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// Should we follow this link? (crawl.py `_domain_ok`)
pub fn domain_ok(candidate: &str, opts: &CrawlOpts, seed_hosts: &[String]) -> bool {
    let Some(host) = host_of(candidate) else {
        return false;
    };
    if !opts.allowed_domains.is_empty() {
        return opts
            .allowed_domains
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")));
    }
    if opts.same_domain {
        return seed_hosts.iter().any(|s| same_site(&host, s));
    }
    true
}

/// Absolutise against the page URL and drop the `#fragment`, so `p#a` and
/// `p#b` collapse onto one already-seen URL (crawl.py uses `urldefrag`).
pub fn normalize_link(base: &str, href: &str) -> Option<String> {
    let base = url::Url::parse(base).ok()?;
    let mut joined = base.join(href).ok()?;
    joined.set_fragment(None);
    if !matches!(joined.scheme(), "http" | "https") {
        return None;
    }
    Some(joined.to_string())
}

/// Every `<a href>` on the page, absolutised and fragment-stripped.
pub fn extract_links(html: &str, base: &str) -> Vec<String> {
    let doc = dom::parse(html);
    dom::descendant_elements(doc.root())
        .into_iter()
        .filter(|element| element.name() == "a")
        .filter_map(|element| element.attr("href"))
        .filter_map(|href| normalize_link(base, href))
        .collect()
}

pub fn fetch_payload(payload: &Value, url: &str) -> Value {
    const FETCH_KEYS: &[&str] = &[
        "impersonate",
        "proxy",
        "headless",
        "network_idle",
        "solve_cloudflare",
        "real_chrome",
        "wait_selector",
        "timeout",
        "useragent",
    ];
    let mut request = serde_json::Map::new();
    for key in FETCH_KEYS {
        if let Some(value) = payload.get(*key) {
            request.insert((*key).into(), value.clone());
        }
    }
    request.insert("url".into(), json!(url));
    if let Some(value) = payload.get("selectors").filter(|value| json_truthy(value)) {
        request.insert("selectors".into(), value.clone());
    }
    if let Some(value) = payload.get("format").filter(|value| json_truthy(value)) {
        request.insert("format".into(), value.clone());
        for key in ["main_content_only", "css_selector"] {
            if let Some(value) = payload.get(key).filter(|value| !value.is_null()) {
                request.insert(key.into(), value.clone());
            }
        }
    }
    Value::Object(request)
}

pub struct CrawlOutcome {
    /// A bounded SAMPLE for the RPC response — never the full set. Use
    /// `item_count` for the real total; `items.len()` caps at `SAMPLE_MAX`.
    pub items: Vec<Value>,
    /// Pages that produced a result, matching crawl.py's `stats["items"]`.
    pub item_count: usize,
    pub crawled: usize,
    pub errors: usize,
    pub stopped: &'static str,
}

/// Walk the frontier. `fetch` does the I/O; `emit` receives every item (in
/// completion order) for streaming. Neither is allowed to abort the crawl:
/// a failing page becomes an `{url, error}` item, exactly as in the reference
/// where `visit()` never raises.
pub async fn run<F, Fut, E>(
    opts: &CrawlOpts,
    payload: &Value,
    body_budget: Option<usize>,
    fetch: F,
    mut emit: E,
) -> CrawlOutcome
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<PageData, String>>,
    E: for<'a> FnMut(
        &'a Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>,
{
    let seed_hosts: Vec<String> = opts.start_urls.iter().filter_map(|u| host_of(u)).collect();
    let include_html = crate::scrapling::page::include_html(payload);

    let mut frontier: VecDeque<(String, i64)> =
        opts.start_urls.iter().map(|u| (u.clone(), 0)).collect();
    let mut seen: HashSet<String> = opts.start_urls.iter().cloned().collect();
    let mut items = Vec::new();
    let (mut crawled, mut errors, mut started, mut item_count) = (0usize, 0usize, 0usize, 0usize);

    let mut pending = FuturesUnordered::new();
    loop {
        while pending.len() < opts.concurrency && (started as i64) < opts.max_pages {
            let Some((url, depth)) = frontier.pop_front() else {
                break;
            };
            started += 1;
            let fut = fetch(url.clone());
            pending.push(async move { (url, depth, fut.await) });
        }
        let Some(first) = pending.next().await else {
            break;
        };

        // `asyncio.wait(..., FIRST_COMPLETED)` returns every task that is
        // already done, not just the one that woke the scheduler. Process that
        // whole batch before refilling the pool or applying the delay.
        let mut done = vec![first];
        while let Some(Some(completed)) = pending.next().now_or_never() {
            done.push(completed);
        }
        for (url, depth, result) in done {
            let item = match result {
                Ok(page) => match serialize_page(&page, payload, include_html, body_budget) {
                    Ok(serialized) => {
                        if depth < opts.max_depth {
                            for link in extract_links(&page.html, &page.url) {
                                if seen.contains(&link) || !domain_ok(&link, opts, &seed_hosts) {
                                    continue;
                                }
                                seen.insert(link.clone());
                                frontier.push_back((link, depth + 1));
                            }
                        }
                        reduce_page(&url, serialized, include_html)
                    }
                    Err(e) => {
                        errors += 1;
                        json!({"url": url, "error": e})
                    }
                },
                Err(e) => {
                    errors += 1;
                    json!({"url": url, "error": e})
                }
            };
            crawled += 1;
            if item.get("error").is_none() {
                item_count += 1;
            }
            emit(&item).await;
            if items.len() < SAMPLE_MAX {
                items.push(item);
            }
        }
        if !opts.download_delay.is_zero() {
            tokio::time::sleep(opts.download_delay).await;
        }
    }

    if let Some(body_budget) = body_budget {
        let per_sample = body_budget / items.len().max(1);
        for item in &mut items {
            if let Some(item) = item.as_object_mut() {
                crate::scrapling::page::budget_page_derived_fields(item, per_sample);
            }
        }
    }

    CrawlOutcome {
        items,
        item_count,
        crawled,
        errors,
        // Anything left in the frontier means the page cap, not exhaustion,
        // ended the crawl.
        stopped: if frontier.is_empty() {
            "done"
        } else {
            "max_pages"
        },
    }
}

fn reduce_page(url: &str, page: Value, include_html: bool) -> Value {
    let mut item = serde_json::Map::new();
    item.insert("url".into(), json!(url));
    item.insert(
        "status".into(),
        page.get("status").cloned().unwrap_or(Value::Null),
    );
    if page.get("extracted").is_some_and(json_truthy) {
        item.insert("extracted".into(), page["extracted"].clone());
    }
    if page.get("content").is_some_and(|value| !value.is_null()) {
        item.insert("content".into(), page["content"].clone());
    }
    if include_html && page.get("html").is_some_and(|value| !value.is_null()) {
        item.insert("html".into(), page["html"].clone());
    }
    Value::Object(item)
}

pub(crate) fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64() != Some(0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn opts(payload: Value) -> CrawlOpts {
        CrawlOpts::from_payload(&payload, 5).unwrap()
    }

    fn page(url: &str, html: &str) -> PageData {
        PageData {
            status: Some(200),
            url: url.to_string(),
            html: html.to_string(),
            ..Default::default()
        }
    }

    /// A canned site: url -> html. Anything not in the map is a fetch error.
    async fn run_on(o: &CrawlOpts, site: &[(&str, &str)], payload: &Value) -> CrawlOutcome {
        let order = RefCell::new(Vec::new());
        let out = run(
            o,
            payload,
            None,
            |url| {
                order.borrow_mut().push(url.clone());
                let found = site
                    .iter()
                    .find(|(u, _)| *u == url)
                    .map(|(u, h)| page(u, h));
                async move { found.ok_or_else(|| "404 not found".to_string()) }
            },
            |_| Box::pin(async {}),
        )
        .await;
        out
    }

    #[test]
    fn same_site_folds_www_both_directions() {
        assert!(same_site("example.com", "www.example.com"));
        assert!(same_site("blog.example.com", "example.com"));
        assert!(same_site("example.com", "blog.example.com"));
        assert!(!same_site("example.com", "example.org"));
        assert!(!same_site("notexample.com", "example.com"));
    }

    #[test]
    fn links_are_absolutised_and_fragment_stripped() {
        let links = extract_links(
            r#"<a href="/a">1</a><a href="/a#x">2</a><a href="https://o.com/b">3</a>
               <a href="mailto:x@y.z">4</a>"#,
            "https://e.com/start",
        );
        assert_eq!(
            links,
            vec![
                "https://e.com/a",
                "https://e.com/a", // #x stripped -> same url, deduped by `seen`
                "https://o.com/b",
            ]
        );
    }

    #[test]
    fn allowed_domains_overrides_same_domain() {
        let o = opts(json!({"url": "https://e.com/", "allowed_domains": ["o.com"]}));
        let seeds = vec!["e.com".to_string()];
        assert!(domain_ok("https://o.com/x", &o, &seeds));
        assert!(domain_ok("https://sub.o.com/x", &o, &seeds));
        assert!(!domain_ok("https://e.com/x", &o, &seeds));
    }

    #[tokio::test]
    async fn breadth_first_order_and_fragment_dedup() {
        let o = opts(json!({"url": "https://e.com/", "max_depth": 2, "concurrency": 1}));
        let site = [
            (
                "https://e.com/",
                r#"<a href="/a">a</a><a href="/b">b</a><a href="/a#dup">a</a>"#,
            ),
            ("https://e.com/a", r#"<a href="/c">c</a>"#),
            ("https://e.com/b", ""),
            ("https://e.com/c", ""),
        ];
        let out = run_on(&o, &site, &json!({})).await;
        // seed, then depth 1 (a, b), then depth 2 (c) — and /a only once
        assert_eq!(out.crawled, 4);
        assert_eq!(out.errors, 0);
        assert_eq!(out.stopped, "done");
    }

    #[tokio::test]
    async fn max_depth_zero_visits_only_the_seeds() {
        let o = opts(json!({"url": "https://e.com/", "max_depth": 0}));
        let site = [
            ("https://e.com/", r#"<a href="/a">a</a>"#),
            ("https://e.com/a", ""),
        ];
        let out = run_on(&o, &site, &json!({})).await;
        assert_eq!(out.crawled, 1);
        assert_eq!(out.stopped, "done");
    }

    #[tokio::test]
    async fn page_cap_stops_and_reports_max_pages() {
        let o = opts(json!({"url": "https://e.com/", "max_pages": 2, "max_depth": 3}));
        let site = [
            ("https://e.com/", r#"<a href="/a">a</a><a href="/b">b</a>"#),
            ("https://e.com/a", ""),
            ("https://e.com/b", ""),
        ];
        let out = run_on(&o, &site, &json!({})).await;
        assert_eq!(out.crawled, 2);
        assert_eq!(out.stopped, "max_pages");
    }

    #[tokio::test]
    async fn a_failing_page_never_sinks_the_crawl() {
        let o = opts(json!({"url": "https://e.com/", "max_depth": 1}));
        // /missing is not in the canned site -> fetch error
        let site = [
            (
                "https://e.com/",
                r#"<a href="/missing">x</a><a href="/ok">y</a>"#,
            ),
            ("https://e.com/ok", ""),
        ];
        let out = run_on(&o, &site, &json!({})).await;
        assert_eq!(out.crawled, 3);
        assert_eq!(out.errors, 1);
        let err_item = out
            .items
            .iter()
            .find(|i| i.get("error").is_some())
            .expect("the failed page is still reported");
        assert_eq!(err_item["url"], json!("https://e.com/missing"));
        assert_eq!(err_item["error"], json!("404 not found"));
    }

    #[tokio::test]
    async fn off_site_links_are_not_followed_by_default() {
        let o = opts(json!({"url": "https://e.com/", "max_depth": 2}));
        let site = [
            ("https://e.com/", r#"<a href="https://other.com/x">x</a>"#),
            ("https://other.com/x", ""),
        ];
        let out = run_on(&o, &site, &json!({})).await;
        assert_eq!(out.crawled, 1, "off-site link must not be crawled");
    }

    #[tokio::test]
    async fn response_items_are_sampled_but_every_item_is_emitted() {
        let o = opts(json!({"url": "https://e.com/p0", "max_pages": 30, "max_depth": 1}));
        // one seed linking to 20 pages
        let links: String = (1..=20)
            .map(|i| format!(r#"<a href="/p{i}">p</a>"#))
            .collect();
        let mut site: Vec<(String, String)> = vec![("https://e.com/p0".into(), links)];
        for i in 1..=20 {
            site.push((format!("https://e.com/p{i}"), String::new()));
        }
        let refs: Vec<(&str, &str)> = site.iter().map(|(u, h)| (u.as_str(), h.as_str())).collect();

        let emitted = RefCell::new(0usize);
        let out = run(
            &o,
            &json!({}),
            None,
            |url| {
                let found = refs
                    .iter()
                    .find(|(u, _)| *u == url)
                    .map(|(u, h)| page(u, h));
                async move { found.ok_or_else(|| "missing".to_string()) }
            },
            |_| {
                *emitted.borrow_mut() += 1;
                Box::pin(async {})
            },
        )
        .await;

        assert_eq!(out.crawled, 21);
        assert_eq!(*emitted.borrow(), 21, "every page is streamed");
        assert_eq!(out.items.len(), SAMPLE_MAX, "the response only samples");
        // The reported count is the real one, not the sample size — reporting
        // items=10 for a 21-page crawl would silently cap forever.
        assert_eq!(out.item_count, 21);
    }

    #[tokio::test]
    async fn item_count_counts_successes_only_and_errors_count_separately() {
        let o = opts(json!({"url": "https://e.com/", "max_depth": 1}));
        let site = [
            (
                "https://e.com/",
                r#"<a href="/gone">x</a><a href="/ok">y</a>"#,
            ),
            ("https://e.com/ok", ""),
        ];
        let out = run_on(&o, &site, &json!({})).await;
        assert_eq!(out.crawled, 3, "every visit counts as crawled");
        assert_eq!(out.errors, 1);
        assert_eq!(out.item_count, 2, "the failed page is not an item");
    }

    #[test]
    fn concurrency_is_clamped_to_the_server_cap() {
        let o = CrawlOpts::from_payload(&json!({"url": "https://e.com/"}), 5).unwrap();
        assert_eq!(
            o.concurrency, 5,
            "the oracle defaults to the configured ceiling"
        );
        let o = CrawlOpts::from_payload(&json!({"url": "https://e.com/", "concurrency": 99}), 5)
            .unwrap();
        assert_eq!(o.concurrency, 5);
        let o = CrawlOpts::from_payload(&json!({"url": "https://e.com/", "concurrency": 0}), 5)
            .unwrap();
        assert_eq!(o.concurrency, 1);
    }

    #[test]
    fn domain_keys_include_the_port_like_urllib_netloc() {
        assert_eq!(
            host_of("https://e.com:8443/a").as_deref(),
            Some("e.com:8443")
        );
        assert!(!same_site("e.com:8443", "e.com:9443"));
    }

    #[tokio::test]
    async fn negative_page_cap_starts_nothing() {
        let o = opts(json!({"url": "https://e.com/", "max_pages": -1}));
        let out = run_on(&o, &[("https://e.com/", "")], &json!({})).await;
        assert_eq!(out.crawled, 0);
        assert_eq!(out.stopped, "max_pages");
    }

    #[tokio::test]
    async fn crawl_sample_has_the_reduced_wrapper_shape() {
        let o = opts(json!({"url": "https://e.com/"}));
        let out = run_on(
            &o,
            &[("https://e.com/", "<h1>Hi</h1>")],
            &json!({"selectors": [{"name": "h", "css": "h1"}]}),
        )
        .await;
        assert_eq!(
            out.items,
            vec![json!({
                "url": "https://e.com/",
                "status": 200,
                "extracted": {"h": "Hi"},
            })]
        );
    }

    #[tokio::test]
    async fn safe_crawl_divides_body_budget_across_returned_samples_with_errors_in_place() {
        let o = opts(json!({
            "start_urls": ["https://e.com/a", "https://e.com/b", "https://e.com/c"],
            "concurrency": 1,
            "max_depth": 0,
        }));
        let payload = json!({"format": "text"});
        let out = run(
            &o,
            &payload,
            Some(crate::scrapling::page::SAFE_BODY_BUDGET),
            |url| async move {
                if url.ends_with("/b") {
                    Err("boom".to_string())
                } else {
                    Ok(page(&url, &format!("<p>{}</p>", "x".repeat(30_000))))
                }
            },
            |_| Box::pin(async {}),
        )
        .await;
        let per_sample = crate::scrapling::page::SAFE_BODY_BUDGET / 3;

        assert_eq!(out.items[0]["url"], "https://e.com/a");
        assert_eq!(out.items[0]["content"].as_str().unwrap().len(), per_sample);
        assert_eq!(
            out.items[1],
            json!({"url": "https://e.com/b", "error": "boom"})
        );
        assert_eq!(out.items[2]["url"], "https://e.com/c");
        assert_eq!(out.items[2]["content"].as_str().unwrap().len(), per_sample);
    }

    #[tokio::test]
    async fn a_completion_batch_is_recorded_before_refilling_the_pool() {
        let o = opts(json!({
            "start_urls": ["https://e.com/1", "https://e.com/2", "https://e.com/3"],
            "concurrency": 2,
            "max_depth": 0,
        }));
        let events = RefCell::new(Vec::new());
        let out = run(
            &o,
            &json!({}),
            None,
            |url| {
                events.borrow_mut().push(format!("start:{url}"));
                async move { Ok(page(&url, "")) }
            },
            |item| {
                events
                    .borrow_mut()
                    .push(format!("emit:{}", item["url"].as_str().unwrap()));
                Box::pin(async {})
            },
        )
        .await;
        assert_eq!(out.crawled, 3);

        let events = events.into_inner();
        let third_start = events
            .iter()
            .position(|event| event == "start:https://e.com/3")
            .unwrap();
        assert_eq!(
            events[..third_start]
                .iter()
                .filter(|event| event.starts_with("emit:"))
                .count(),
            2,
            "the oracle processes every task returned by FIRST_COMPLETED before refilling: {events:?}"
        );
    }

    #[test]
    fn missing_start_urls_and_bad_fetcher_are_rejected() {
        assert_eq!(
            CrawlOpts::from_payload(&json!({}), 5).unwrap_err(),
            "provide `start_urls`"
        );
        assert!(
            CrawlOpts::from_payload(&json!({"url": "u", "fetcher": "carrier"}), 5)
                .unwrap_err()
                .contains("unknown fetcher")
        );
    }

    #[test]
    fn wrapper_coercions_and_errors_match_python() {
        assert_eq!(
            CrawlOpts::from_payload(&json!({"url": "https://e.com/", "max_pages": [1]}), 5)
                .unwrap_err(),
            "int() argument must be a string, a bytes-like object or a real number, not 'list'"
        );
        assert_eq!(
            CrawlOpts::from_payload(&json!({"url": "https://e.com/", "max_depth": "x"}), 5)
                .unwrap_err(),
            "invalid literal for int() with base 10: 'x'"
        );
        assert_eq!(
            CrawlOpts::from_payload(&json!({"url": "https://e.com/", "fetcher": null}), 5)
                .unwrap_err(),
            "unknown fetcher: None (use http|stealthy|dynamic)"
        );
        assert_eq!(
            CrawlOpts::from_payload(&json!({"start_urls": [1]}), 5).unwrap_err(),
            "'int' object has no attribute 'decode'"
        );

        let options = opts(json!({
            "start_urls": "ab",
            "allowed_domains": {"EXAMPLE.COM": true},
            "same_domain": 0,
            "max_pages": " 2 "
        }));
        assert_eq!(options.start_urls, ["a", "b"]);
        assert_eq!(options.allowed_domains, ["example.com"]);
        assert!(!options.same_domain);
        assert_eq!(options.max_pages, 2);
    }

    #[test]
    fn per_page_payload_only_forwards_the_wrapper_allowlist() {
        let request = fetch_payload(
            &json!({
                "url": "https://old.invalid",
                "headers": {"x": "silently excluded by the oracle crawl wrapper"},
                "impersonate": "chrome",
                "timeout": 9,
                "selectors": [{"name": "h", "css": "h1"}],
                "format": "text",
                "main_content_only": false,
                "css_selector": "main",
                "include_html": true,
            }),
            "https://e.com/page",
        );
        assert_eq!(
            request,
            json!({
                "impersonate": "chrome",
                "timeout": 9,
                "url": "https://e.com/page",
                "selectors": [{"name": "h", "css": "h1"}],
                "format": "text",
                "main_content_only": false,
                "css_selector": "main",
            })
        );
    }

    #[test]
    fn an_absurd_download_delay_is_clamped_not_panicked_on() {
        for v in [1e20, f64::MAX] {
            let o = opts(json!({"url": "https://e.com/", "download_delay": v}));
            assert!(o.download_delay <= Duration::from_secs_f64(MAX_DOWNLOAD_DELAY_SECS));
        }
        assert_eq!(
            opts(json!({"url": "https://e.com/", "download_delay": f64::NAN})).download_delay,
            Duration::ZERO
        );
        assert_eq!(
            CrawlOpts::from_payload_for_mode(
                &json!({"url": "https://e.com/", "download_delay": 301}),
                5,
                SecurityMode::Compat,
            )
            .unwrap()
            .download_delay,
            Duration::from_secs(301)
        );
        assert_eq!(
            CrawlOpts::from_payload_for_mode(
                &json!({"url": "https://e.com/", "download_delay": -1}),
                5,
                SecurityMode::Compat,
            )
            .unwrap()
            .download_delay,
            Duration::ZERO
        );
        assert_eq!(
            CrawlOpts::from_payload(&json!({"url": "https://e.com/", "download_delay": [1]}), 5,)
                .unwrap_err(),
            "float() argument must be a string or a real number, not 'list'"
        );
    }

    #[test]
    fn stream_name_defaults_to_our_namespace() {
        assert_eq!(opts(json!({"url": "u"})).stream_name, "browser::crawl");
        assert_eq!(
            opts(json!({"url": "u", "stream_name": 3})).stream_name,
            json!(3)
        );
    }
}
