//! Screenshots and DOM/React trees through the `browser` worker: open a
//! headless incognito tab on the console's `/ui-files` copy of the story,
//! wait for the runtime's ready mark, capture the tree, take the shot and
//! crop it to the story content. Results are cached under
//! `renders/<hash>/` where the hash covers everything that influences the
//! pixels: component version, state, args, globals, viewport, mode.

use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use iii_sdk::protocol::TriggerRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::builder::{Ctx, now};
use crate::config::Viewport;
use crate::model::{Component, sha256_hex};

/// Bump when runtime.js changes what a capture or a render looks like.
pub const RUNTIME_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderMeta {
    pub hash: String,
    pub workspace: String,
    pub line: String,
    pub project: String,
    pub component: String,
    pub version: String,
    pub state: String,
    pub args: Value,
    pub globals: Value,
    pub viewport: Viewport,
    pub deterministic: bool,
    /// content | viewport
    pub clip: String,
    /// Story content box in CSS pixels, relative to the viewport.
    #[serde(rename = "box")]
    pub content_box: Value,
    pub width: u32,
    pub height: u32,
    pub rendered_at: String,
    pub url: String,
}

pub struct RenderRequest<'a> {
    pub workspace: &'a str,
    pub line_key: &'a str,
    pub component: &'a Component,
    pub state_id: String,
    pub args: Value,
    pub globals: Value,
    pub viewport: Viewport,
    pub deterministic: bool,
    pub clip: String,
}

#[derive(Debug, Clone)]
pub struct Rendered {
    pub meta: RenderMeta,
    pub dir: PathBuf,
    pub shot: PathBuf,
    pub tree: Value,
    pub cached: bool,
}

pub fn render_hash(req: &RenderRequest<'_>) -> String {
    let key = json!({
        "runtime": RUNTIME_VERSION,
        "version": req.component.version,
        "state": req.state_id,
        "args": req.args,
        "globals": req.globals,
        "viewport": req.viewport,
        "deterministic": req.deterministic,
        "clip": req.clip,
    });
    sha256_hex(serde_json::to_string(&key).unwrap_or_default().as_bytes())
}

async fn browser(
    ctx: &Ctx,
    function: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    ctx.iii
        .trigger(TriggerRequest {
            function_id: function.to_string(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
        .map_err(|e| format!("{function}: {e} (is the browser worker running?)"))
}

fn b64url(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_string(value).unwrap_or_else(|_| "{}".into()))
}

#[allow(clippy::too_many_arguments)]
pub fn story_url(
    console_url: &str,
    workspace: &str,
    line_key: &str,
    html: &str,
    state: &str,
    args: &Value,
    globals: &Value,
    deterministic: bool,
) -> String {
    format!(
        "{}/ui-files/stories/{}/{}/{}?story={}&args={}&globals={}&deterministic={}",
        console_url.trim_end_matches('/'),
        workspace,
        line_key,
        html,
        state,
        b64url(args),
        b64url(globals),
        if deterministic { 1 } else { 0 }
    )
}

pub async fn render(ctx: &Ctx, req: RenderRequest<'_>) -> Result<Rendered, String> {
    let hash = render_hash(&req);
    let dir = ctx.store.renders_dir(req.workspace).join(&hash);
    let shot = dir.join("shot.png");
    if let (Ok(meta), Ok(tree)) = (
        std::fs::read(dir.join("meta.json")),
        std::fs::read(dir.join("tree.json")),
    ) && shot.is_file()
        && let (Ok(meta), Ok(tree)) = (
            serde_json::from_slice::<RenderMeta>(&meta),
            serde_json::from_slice::<Value>(&tree),
        )
    {
        return Ok(Rendered {
            meta,
            dir,
            shot,
            tree,
            cached: true,
        });
    }
    let html = req.component.html.clone().ok_or_else(|| {
        format!(
            "component {} has no built html (build error: {})",
            req.component.id,
            req.component.error.clone().unwrap_or_default()
        )
    })?;
    let url = story_url(
        &ctx.config().console_url,
        req.workspace,
        req.line_key,
        &html,
        &req.state_id,
        &req.args,
        &req.globals,
        req.deterministic,
    );

    let started = browser(
        ctx,
        "browser::sessions::start",
        json!({ "headful": false, "incognito": true, "ttl_ms": 180_000 }),
        60_000,
    )
    .await?;
    let session_id = started
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or("browser::sessions::start returned no session_id")?
        .to_string();
    let result = drive(ctx, &session_id, &url, &req).await;
    let _ = browser(
        ctx,
        "browser::sessions::stop",
        json!({ "session_id": session_id }),
        15_000,
    )
    .await;
    let (tree, png, content_box) = result?;

    let mut image = image::load_from_memory(&png)
        .map_err(|e| format!("decoding screenshot: {e}"))?
        .to_rgba8();
    let dpr = req.viewport.dpr.max(1) as f64;
    if req.clip == "content" {
        let get = |k: &str| content_box.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let (x, y, w, h) = (
            (get("x") * dpr).floor(),
            (get("y") * dpr).floor(),
            (get("w") * dpr).ceil(),
            (get("h") * dpr).ceil(),
        );
        let x = (x.max(0.0) as u32).min(image.width().saturating_sub(1));
        let y = (y.max(0.0) as u32).min(image.height().saturating_sub(1));
        let w = (w.max(1.0) as u32).min(image.width() - x);
        let h = (h.max(1.0) as u32).min(image.height() - y);
        image = image::imageops::crop_imm(&image, x, y, w, h).to_image();
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    image
        .save(&shot)
        .map_err(|e| format!("saving screenshot: {e}"))?;
    let meta = RenderMeta {
        hash: hash.clone(),
        workspace: req.workspace.to_string(),
        line: req.line_key.to_string(),
        project: req.component.project.clone(),
        component: req.component.id.clone(),
        version: req.component.version.clone(),
        state: req.state_id.clone(),
        args: req.args.clone(),
        globals: req.globals.clone(),
        viewport: req.viewport,
        deterministic: req.deterministic,
        clip: req.clip.clone(),
        content_box,
        width: image.width(),
        height: image.height(),
        rendered_at: now(),
        url,
    };
    std::fs::write(
        dir.join("tree.json"),
        serde_json::to_vec(&tree).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(Rendered {
        meta,
        dir,
        shot,
        tree,
        cached: false,
    })
}

async fn drive(
    ctx: &Ctx,
    session_id: &str,
    url: &str,
    req: &RenderRequest<'_>,
) -> Result<(Value, Vec<u8>, Value), String> {
    browser(
        ctx,
        "browser::resize",
        json!({ "session_id": session_id, "width": req.viewport.width, "height": req.viewport.height, "device_scale_factor": req.viewport.dpr as f64 }),
        30_000,
    )
    .await?;
    let navigated = browser(
        ctx,
        "browser::navigate",
        json!({ "session_id": session_id, "url": url, "timeout_ms": 45_000 }),
        60_000,
    )
    .await?;
    if navigated.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(format!(
            "navigation failed: {}",
            navigated
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    let probe = "document.documentElement.dataset.storiesReady === '1' ? 'ready' : (document.getElementById('story-error') ? 'error:' + document.getElementById('story-error').textContent : 'pending')";
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(40);
    loop {
        let value = browser(
            ctx,
            "browser::evaluate",
            json!({ "session_id": session_id, "expression": probe, "timeout_ms": 5_000 }),
            10_000,
        )
        .await?;
        match value.get("value").and_then(Value::as_str) {
            Some("ready") => break,
            Some(text) if text.starts_with("error:") => {
                return Err(format!("story failed to render: {}", &text[6..]));
            }
            _ => {}
        }
        if tokio::time::Instant::now() > deadline {
            return Err("story never signalled ready (40s)".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let captured = browser(
        ctx,
        "browser::evaluate",
        json!({ "session_id": session_id, "expression": "JSON.stringify(window.__stories.capture())", "timeout_ms": 20_000 }),
        30_000,
    )
    .await?;
    let mut tree: Value = match captured.get("value") {
        Some(Value::String(text)) => {
            serde_json::from_str(text).map_err(|e| format!("bad capture: {e}"))?
        }
        Some(other) => other.clone(),
        None => {
            return Err(format!(
                "capture failed: {}",
                captured
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("no value")
            ));
        }
    };
    if let Ok(snapshot) = browser(
        ctx,
        "browser::snapshot",
        json!({ "session_id": session_id }),
        20_000,
    )
    .await
        && let Some(a11y) = snapshot.get("tree").cloned()
    {
        tree["a11y"] = a11y;
    }
    let shot = browser(
        ctx,
        "browser::screenshot",
        json!({ "session_id": session_id, "full_page": false, "format": "png" }),
        30_000,
    )
    .await?;
    let data = shot
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks
                .iter()
                .find_map(|b| b.get("data").and_then(Value::as_str))
        })
        .ok_or("browser::screenshot returned no image data")?;
    let png = STANDARD
        .decode(data.trim())
        .map_err(|e| format!("decoding screenshot base64: {e}"))?;
    let content_box = tree
        .get("box")
        .cloned()
        .unwrap_or(json!({ "x": 0, "y": 0, "w": req.viewport.width, "h": req.viewport.height }));
    Ok((tree, png, content_box))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Component, State};

    fn component() -> Component {
        Component {
            id: "ui-button".into(),
            project: "app".into(),
            file: "src/Button.stories.tsx".into(),
            path: "app/src/Button.stories.tsx".into(),
            title: "UI/Button".into(),
            group: "UI".into(),
            tags: vec![],
            component: None,
            version: "v1".into(),
            html: Some("app/node_modules/.stories/src-button.html".into()),
            states: vec![State {
                id: "ui-button--primary".into(),
                name: "Primary".into(),
                export_name: "Primary".into(),
                tags: vec![],
                args: Value::Null,
                arg_types: Value::Null,
                parameters: Value::Null,
                controls: vec![],
                has_render: false,
            }],
            inputs: vec![],
            globals: Value::Null,
            global_types: Value::Null,
            error: None,
        }
    }

    #[test]
    fn hash_depends_on_args_and_viewport_but_not_on_the_line() {
        let c = component();
        let base = RenderRequest {
            workspace: "ws",
            line_key: "worktree",
            component: &c,
            state_id: "ui-button--primary".into(),
            args: json!({}),
            globals: json!({}),
            viewport: Viewport::default(),
            deterministic: true,
            clip: "content".into(),
        };
        let same_other_line = RenderRequest {
            line_key: "abc",
            ..RenderRequest {
                workspace: "ws",
                line_key: "x",
                component: &c,
                state_id: "ui-button--primary".into(),
                args: json!({}),
                globals: json!({}),
                viewport: Viewport::default(),
                deterministic: true,
                clip: "content".into(),
            }
        };
        assert_eq!(render_hash(&base), render_hash(&same_other_line));
        let other_args = RenderRequest {
            args: json!({ "size": "lg" }),
            ..RenderRequest {
                workspace: "ws",
                line_key: "worktree",
                component: &c,
                state_id: "ui-button--primary".into(),
                args: json!({}),
                globals: json!({}),
                viewport: Viewport::default(),
                deterministic: true,
                clip: "content".into(),
            }
        };
        assert_ne!(render_hash(&base), render_hash(&other_args));
    }

    #[test]
    fn story_urls_encode_args_as_base64url() {
        let url = story_url(
            "http://127.0.0.1:3113/",
            "ws",
            "worktree",
            "app/x.html",
            "ui-button--primary",
            &json!({ "a": 1 }),
            &json!({}),
            true,
        );
        assert!(url.starts_with("http://127.0.0.1:3113/ui-files/stories/ws/worktree/app/x.html?story=ui-button--primary&args="));
        assert!(url.ends_with("&deterministic=1"));
        let args = url
            .split("&args=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        assert!(
            !args.is_empty() && !args.contains('='),
            "base64url without padding: {args}"
        );
    }
}
