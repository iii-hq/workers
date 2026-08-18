//! The nine outbound `browser::*` functions: the fetch tiers,
//! screenshot, persistent sessions and crawl.
//!
//! Unlike the ten parse ops (sync, stateless, `fn(&Value) -> Result<Value>`),
//! these are async and need state — an HTTP session registry, the browser
//! session registry, config and the bus — so they carry a `Ctx` and register
//! through their own path in `super::register_all`.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::{json, Value};

use crate::config::{SecurityMode, SharedConfig, WorkerConfig};
use crate::scrapling::crawl::{self, CrawlOpts};
use crate::scrapling::fetch::{self, HttpMode, HttpOptions};
use crate::scrapling::page::{self, PageData};
use crate::scrapling::raw_browser::{RawBrowser, RawBrowserOptions};
use crate::scrapling::sessions::{parse_type, uses_compat_only_options, Registry};
use crate::session::Sessions;
use crate::ssrf::SsrfPolicy;

pub struct Ctx {
    pub http: Registry,
    pub config: SharedConfig,
    pub iii: Arc<iii_sdk::IIIClient>,
}

impl Ctx {
    pub fn new(sessions: Arc<Sessions>, iii: Arc<iii_sdk::IIIClient>) -> Self {
        let config = sessions.config.clone();
        let startup = config.load().scrapling.startup_snapshot();
        Self {
            http: Registry::new(startup.max_sessions, startup.session_idle_timeout_s),
            config,
            iii,
        }
    }

    pub fn policy(&self) -> SsrfPolicy {
        SsrfPolicy {
            allow_loopback: self.config.load().scrapling.allow_loopback,
        }
    }
}

/// Run one fetch per target URL, single or bulk, isolating per-URL failures.
async fn fetch_targets<F, Fut>(
    payload: &Value,
    concurrency: usize,
    fetch_one: F,
) -> Result<Value, String>
where
    F: Fn(String) -> Fut + Sync,
    Fut: std::future::Future<Output = Result<PageData, String>>,
{
    let (urls, bulk) = page::targets(payload)?;
    let include_html = page::include_html(payload);
    if !bulk {
        let p = fetch_one(urls[0].clone()).await?;
        return page::serialize_page(&p, payload, include_html);
    }
    let out = futures::stream::iter(urls.into_iter().map(|url| {
        let fetch_one = &fetch_one;
        async move {
            match fetch_one(url.clone()).await {
                Ok(page) => page::serialize_page(&page, payload, include_html)
                    .map_err(|error| (url.clone(), error)),
                Err(error) => Err((url, error)),
            }
        }
    }))
    .buffered(concurrency)
    .collect::<Vec<_>>()
    .await;
    Ok(page::bulk_results(out))
}

fn one_shot_payload(config: &WorkerConfig, payload: &Value, include_html_default: bool) -> Value {
    let mut request = payload.as_object().cloned().unwrap_or_default();
    let defaults = &config.scrapling.defaults;
    // Safe mode refuses every proxy, so injecting the config default would
    // fail every outbound call with an error blaming a "caller proxy" the
    // caller never sent. The knob is compat-only; safe mode leaves it out and
    // a caller-passed proxy still gets the explicit refusal.
    let inject_proxy = config.scrapling.security_mode == SecurityMode::Compat;
    for (key, value) in [
        ("impersonate", json!(defaults.impersonate)),
        ("headless", json!(defaults.headless)),
        ("network_idle", json!(defaults.network_idle)),
        ("proxy", json!(defaults.proxy)),
    ] {
        if key == "proxy" && !inject_proxy {
            continue;
        }
        if request.get(key).is_none_or(Value::is_null) && value != json!("") {
            request.insert(key.to_string(), value);
        }
    }
    if include_html_default && request.get("include_html").is_none_or(Value::is_null) {
        request.insert("include_html".to_string(), json!(defaults.include_html));
    }
    Value::Object(request)
}

fn http_mode(ctx: &Ctx) -> HttpMode {
    match ctx.config.load().scrapling.security_mode {
        SecurityMode::Safe => HttpMode::Safe,
        SecurityMode::Compat => HttpMode::Compat,
    }
}

async fn raw_browser_targets(ctx: &Ctx, payload: &Value, stealth: bool) -> Result<Value, String> {
    let config = ctx.config.load_full();
    let request = one_shot_payload(&config, payload, true);
    let mut options = RawBrowserOptions::from_payload(&request)?;
    options.clamp_durations(config.max_timeout_ms);
    options.validate_policy(config.scrapling.security_mode)?;
    let concurrency = usize::try_from(config.scrapling.max_bulk_concurrency)
        .unwrap_or(usize::MAX)
        .max(1);
    let (urls, bulk) = page::targets(&request)?;
    let include_html = page::include_html(&request);
    if !bulk {
        let browser = RawBrowser::start(&config, &options, stealth, false).await?;
        let fetched = browser.fetch(&urls[0], &options, stealth).await?;
        return page::serialize_page(&fetched, &request, include_html);
    }
    let output = futures::stream::iter(urls.into_iter().map(|url| {
        let config = config.clone();
        let options = options.clone();
        let request = request.clone();
        async move {
            let fetched = async {
                let browser = RawBrowser::start(&config, &options, stealth, false).await?;
                browser.fetch(&url, &options, stealth).await
            }
            .await;
            match fetched {
                Ok(page) => page::serialize_page(&page, &request, include_html)
                    .map_err(|error| (url.clone(), error)),
                Err(error) => Err((url, error)),
            }
        }
    }))
    .buffered(concurrency)
    .collect::<Vec<_>>()
    .await;
    Ok(page::bulk_results(output))
}

// ---- the nine handlers ---------------------------------------------------

pub async fn op_fetch(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    let config = ctx.config.load_full();
    let request = one_shot_payload(&config, payload, true);
    let opts = HttpOptions::from_payload_for_mode(&request, http_mode(ctx))?;
    let policy = ctx.policy();
    let concurrency = usize::try_from(config.scrapling.max_bulk_concurrency)
        .unwrap_or(usize::MAX)
        .max(1);
    fetch_targets(&request, concurrency, |url| {
        let opts = &opts;
        let policy = &policy;
        async move { fetch::fetch_page(&url, opts, policy).await }
    })
    .await
}

pub async fn op_dynamic_fetch(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    raw_browser_targets(ctx, payload, false).await
}

pub async fn op_stealthy_fetch(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    raw_browser_targets(ctx, payload, true).await
}

pub async fn op_screenshot(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    let cfg = ctx.config.load_full();
    let request = one_shot_payload(&cfg, payload, false);
    let url = request
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .ok_or("provide `url`")?;
    let fetcher = request
        .get("fetcher")
        .and_then(Value::as_str)
        .unwrap_or("dynamic");
    let format = request
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("png");
    let full_page = request
        .get("full_page")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut options = RawBrowserOptions::from_payload(&request)?;
    options.clamp_durations(cfg.max_timeout_ms);
    let browser = RawBrowser::start(&cfg, &options, fetcher == "stealthy", false).await?;
    let (content, mime, final_url) = browser
        .screenshot(url, &options, fetcher == "stealthy", full_page, format)
        .await?;
    Ok(json!({"content": content, "mime": mime, "url": final_url}))
}

pub async fn op_session_open(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    let stype = parse_type(payload)?;
    let mode = ctx.config.load().scrapling.security_mode;
    let compat_only = uses_compat_only_options(payload) || mode == SecurityMode::Compat;
    if compat_only && mode == SecurityMode::Safe {
        return Err(
            "session options require browser.scrapling.security_mode=compat; remove them or switch modes"
                .to_string(),
        );
    }
    if stype == crate::scrapling::sessions::SessionType::Http {
        ctx.http
            .open_http(payload, compat_only, http_mode(ctx))
            .await
    } else {
        ctx.http
            .open_browser(stype, payload, compat_only, ctx.config.load_full())
            .await
    }
}

pub async fn op_session_fetch(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    let sid = payload
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or("provide `session_id`")?;
    let url = payload
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .ok_or("provide `url`")?
        .to_string();
    validate_session_mode(
        ctx.http.uses_compat_only_options(sid)?,
        ctx.config.load().scrapling.security_mode,
    )?;
    match ctx.http.session_type(sid)? {
        crate::scrapling::sessions::SessionType::Http => {
            let backend = ctx.http.http_backend(sid)?;
            let request = backend.request(payload);
            let include_html = page::include_html(&request);
            let policy = ctx.policy();
            ctx.http
                .run(
                    sid,
                    Box::pin(async move {
                        let mut options =
                            HttpOptions::from_payload_for_mode(&request, backend.mode)?;
                        options.jar = Some(backend.jar.clone());
                        #[cfg(feature = "scrapling-compat")]
                        {
                            options.compat_session = backend.compat.clone();
                        }
                        let fetched = fetch::fetch_page(&url, &options, &policy).await?;
                        page::serialize_page(&fetched, &request, include_html)
                    }),
                )
                .await
        }
        crate::scrapling::sessions::SessionType::Dynamic
        | crate::scrapling::sessions::SessionType::Stealthy => {
            let backend = ctx.http.browser_backend(sid)?;
            let request = backend.request(payload);
            let proxy_override = payload
                .get("proxy")
                .filter(|value| !value.is_null())
                .is_some_and(|value| value.as_str() != Some(""));
            let include_html = page::include_html(&request);
            let security_mode = backend.security_mode;
            let max_timeout_ms = ctx.config.load().max_timeout_ms;
            ctx.http
                .run(
                    sid,
                    Box::pin(async move {
                        let mut options = RawBrowserOptions::from_payload(&request)?;
                        options.clamp_durations(max_timeout_ms);
                        options.validate_policy(security_mode)?;
                        let fetched = backend
                            .browser
                            .fetch_session(&url, &options, backend.stealth, proxy_override)
                            .await?;
                        page::serialize_page(&fetched, &request, include_html)
                    }),
                )
                .await
        }
    }
}

fn validate_session_mode(compat_only: bool, mode: SecurityMode) -> Result<(), String> {
    if compat_only && mode == SecurityMode::Safe {
        return Err(
            "this session uses compat-only options and safe mode is now active; close and reopen the session"
                .to_string(),
        );
    }
    Ok(())
}

pub async fn op_session_close(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    let sid = payload
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or("provide `session_id`")?;
    Ok(ctx.http.close(sid).await)
}

pub async fn op_session_list(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    Ok(ctx.http.list(payload.get("type")))
}

pub async fn op_crawl(ctx: &Ctx, payload: &Value) -> Result<Value, String> {
    let cfg = ctx.config.load_full();
    let opts = CrawlOpts::from_payload_for_mode(
        payload,
        cfg.scrapling.max_bulk_concurrency as usize,
        cfg.scrapling.security_mode,
    )?;
    let (group_id, group_id_text) = crawl_group_id(payload);

    let http = opts.fetcher == "http";
    let policy = ctx.policy();
    let mode = ctx.config.load().scrapling.security_mode;
    // Atomic, not Cell: this future is handed to the SDK, which requires Send
    // + Sync.
    let seq = std::sync::atomic::AtomicUsize::new(0);

    let outcome = crawl::run(
        &opts,
        payload,
        |url| {
            let cfg = cfg.clone();
            let policy = &policy;
            let stealth = opts.fetcher == "stealthy";
            let request = one_shot_payload(&cfg, &crawl::fetch_payload(payload, &url), false);
            async move {
                if http {
                    let options = HttpOptions::from_payload_for_mode(
                        &request,
                        match mode {
                            SecurityMode::Safe => HttpMode::Safe,
                            SecurityMode::Compat => HttpMode::Compat,
                        },
                    )?;
                    fetch::fetch_page(&url, &options, policy).await
                } else {
                    let mut options = RawBrowserOptions::from_payload(&request)?;
                    options.clamp_durations(cfg.max_timeout_ms);
                    options.validate_policy(cfg.scrapling.security_mode)?;
                    let browser = RawBrowser::start(&cfg, &options, stealth, false).await?;
                    browser.fetch(&url, &options, stealth).await
                }
            }
        },
        |item| {
            let n = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let iii = ctx.iii.clone();
            let payload = json!({
                "stream_name": opts.stream_name,
                "group_id": group_id,
                "item_id": format!("{group_id_text}-{n:06}"),
                "data": item,
            });
            Box::pin(async move {
                let res = iii
                    .trigger(iii_sdk::protocol::TriggerRequest {
                        function_id: "stream::set".to_string(),
                        payload,
                        action: None,
                        timeout_ms: Some(5_000),
                    })
                    .await;
                if let Err(e) = res {
                    tracing::debug!(error = %e, "crawl stream::set failed; continuing");
                }
            })
        },
    )
    .await;

    Ok(json!({
        "stats": {
            "crawled": outcome.crawled,
            "items": outcome.item_count,
            "errors": outcome.errors,
            "stopped": outcome.stopped,
        },
        "items": outcome.items,
        "stream": {"name": opts.stream_name, "group_id": group_id},
    }))
}

fn crawl_group_id(payload: &Value) -> (Value, String) {
    if let Some(value) = payload
        .get("group_id")
        .filter(|value| crawl::json_truthy(value))
    {
        let text = match value {
            Value::String(value) => value.clone(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) | Value::Null => unreachable!("falsy values were filtered"),
            other => crawl::python_repr(other),
        };
        return (value.clone(), text);
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    (json!(id), id)
}

/// Route one of the nine async function ids to its handler. The ten sync
/// parse ops go through `super::op_for` instead; `super::register_all` picks
/// the path per catalog entry.
pub async fn dispatch(ctx: &Ctx, function_id: &str, payload: &Value) -> Result<Value, String> {
    match function_id {
        "browser::fetch" => op_fetch(ctx, payload).await,
        "browser::dynamic-fetch" => op_dynamic_fetch(ctx, payload).await,
        "browser::stealthy-fetch" => op_stealthy_fetch(ctx, payload).await,
        "browser::screenshot-url" => op_screenshot(ctx, payload).await,
        "browser::session-open" => op_session_open(ctx, payload).await,
        "browser::session-fetch" => op_session_fetch(ctx, payload).await,
        "browser::session-close" => op_session_close(ctx, payload).await,
        "browser::session-list" => op_session_list(ctx, payload).await,
        "browser::crawl" => op_crawl(ctx, payload).await,
        other => Err(format!("not implemented: {other}")),
    }
}

/// Every id `dispatch` handles, in catalog order. `super::register_all`
/// asserts this partitions the catalog exactly with `op_for`.
pub const NET_IDS: &[&str] = &[
    "browser::fetch",
    "browser::stealthy-fetch",
    "browser::dynamic-fetch",
    "browser::screenshot-url",
    "browser::session-open",
    "browser::session-fetch",
    "browser::session-close",
    "browser::session-list",
    "browser::crawl",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_net_id_dispatches_somewhere() {
        // A typo'd arm in `dispatch` would silently become "not implemented"
        // at runtime; this pins the two lists together instead.
        for id in NET_IDS {
            assert!(
                !matches!(id, &"") && crate::scrapling::op_for(id).is_none(),
                "{id} must not also be a sync parse op"
            );
        }
    }

    #[test]
    fn one_shot_defaults_apply_to_null_fields_but_crawl_html_stays_request_only() {
        let mut config = WorkerConfig::default();
        config.scrapling.defaults.impersonate = "firefox".to_string();
        config.scrapling.defaults.headless = false;
        config.scrapling.defaults.network_idle = true;
        config.scrapling.defaults.proxy = "http://configured".to_string();
        config.scrapling.defaults.include_html = true;
        let request = one_shot_payload(
            &config,
            &json!({"impersonate": null, "headless": true}),
            true,
        );
        assert_eq!(request["impersonate"], "firefox");
        assert_eq!(request["headless"], true);
        assert_eq!(request["network_idle"], true);
        assert_eq!(request["include_html"], true);
        // Safe mode (the default) refuses every proxy, so the config default
        // must NOT be injected — otherwise every fetch fails blaming a caller
        // proxy nobody sent. Compat mode injects it.
        assert!(request.get("proxy").is_none());
        config.scrapling.security_mode = SecurityMode::Compat;
        let compat = one_shot_payload(&config, &json!({}), true);
        assert_eq!(compat["proxy"], "http://configured");
        assert!(one_shot_payload(&config, &json!({}), false)
            .get("include_html")
            .is_none());
    }

    #[test]
    fn crawl_group_id_honors_nonempty_input_or_generates_uuid4_hex() {
        assert_eq!(
            crawl_group_id(&json!({"group_id": "caller-group"})),
            (json!("caller-group"), "caller-group".into())
        );
        assert_eq!(
            crawl_group_id(&json!({"group_id": 3})),
            (json!(3), "3".into())
        );
        for payload in [json!({}), json!({"group_id": ""})] {
            let (value, generated) = crawl_group_id(&payload);
            assert_eq!(value, generated);
            assert_eq!(generated.len(), 32);
            assert!(generated
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
            assert_eq!(&generated[12..13], "4");
        }
    }

    #[test]
    fn safe_mode_invalidates_a_compat_only_session() {
        assert_eq!(
            validate_session_mode(true, SecurityMode::Safe).unwrap_err(),
            "this session uses compat-only options and safe mode is now active; close and reopen the session"
        );
        assert!(validate_session_mode(true, SecurityMode::Compat).is_ok());
        assert!(validate_session_mode(false, SecurityMode::Safe).is_ok());
    }
}
