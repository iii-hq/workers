//! `computer::act` — drive the desktop by coordinate (pointer and keyboard,
//! in the pixels of the latest screenshot) or by `ref` (an accessibility
//! action on an element from `computer::observe`/`computer::find`).

use std::sync::Arc;

use iii_sdk::errors::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::invalid;
use crate::driver::{Driver, Screen};
use crate::semantic::{SemanticError, Verb, MAX_SET_VALUE_CHARS};
use crate::session::Session;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ActInput {
    pub session_id: String,
    /// By coordinate: `click`, `right_click`, `double_click`, `move`, `drag`,
    /// `scroll`, `type`, `press`, or `hotkey`. By `ref`: `invoke`,
    /// `set_value` (with `text`), `toggle`, or `select`.
    pub action: String,
    /// Element ref from `computer::observe` (include_a11y) or
    /// `computer::find`, for invoke/set_value/toggle/select. Refs die when a
    /// newer observe/find replaces them or a coordinate action runs.
    #[serde(default)]
    pub r#ref: Option<String>,
    /// Target x (pixels). Required for click/right_click/double_click/move and
    /// the drag start; the scroll anchor defaults to screen center.
    #[serde(default)]
    pub x: Option<i64>,
    /// Target y (pixels).
    #[serde(default)]
    pub y: Option<i64>,
    /// Drag end x (with `to_y`), for `action=drag`.
    #[serde(default)]
    pub to_x: Option<i64>,
    /// Drag end y.
    #[serde(default)]
    pub to_y: Option<i64>,
    /// Mouse button for `drag` (`left`, `right`, `middle`). Default `left`.
    #[serde(default)]
    pub button: Option<String>,
    /// Text to type (`type`) or the value to set (`set_value`).
    #[serde(default)]
    pub text: Option<String>,
    /// Keys for `press`/`hotkey`: a single key (`["enter"]`) or a chord
    /// (`["ctrl", "c"]`).
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    /// Horizontal scroll amount for `action=scroll`. Default 0.
    #[serde(default)]
    pub scroll_x: Option<i64>,
    /// Vertical scroll amount for `action=scroll`; positive scrolls down.
    /// Default 3.
    #[serde(default)]
    pub scroll_y: Option<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActOutput {
    pub ok: bool,
    /// What was done, for the transcript.
    pub detail: String,
}

/// A validated coordinate action, ready to dispatch.
enum Physical {
    Click(i64, i64),
    RightClick(i64, i64),
    DoubleClick(i64, i64),
    Move(i64, i64),
    Scroll {
        x: i64,
        y: i64,
        sx: i64,
        sy: i64,
    },
    Drag {
        from: (i64, i64),
        to: (i64, i64),
        button: String,
    },
    Type(String),
    Keys(Vec<String>),
}

fn require_xy(req: &ActInput) -> Result<(i64, i64), Error> {
    match (req.x, req.y) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(Error::Handler(format!(
            "action '{}' needs x and y",
            req.action
        ))),
    }
}

/// The actions `plan` dispatches by coordinate.
fn is_coordinate(action: &str) -> bool {
    matches!(
        action,
        "click"
            | "left_click"
            | "right_click"
            | "double_click"
            | "move"
            | "move_cursor"
            | "scroll"
            | "drag"
            | "type"
            | "press"
            | "hotkey"
            | "key"
    )
}

fn plan(req: &ActInput, screen: Screen) -> Result<Physical, Error> {
    Ok(match req.action.as_str() {
        "click" | "left_click" => {
            let (x, y) = require_xy(req)?;
            Physical::Click(x, y)
        }
        "right_click" => {
            let (x, y) = require_xy(req)?;
            Physical::RightClick(x, y)
        }
        "double_click" => {
            let (x, y) = require_xy(req)?;
            Physical::DoubleClick(x, y)
        }
        "move" | "move_cursor" => {
            let (x, y) = require_xy(req)?;
            Physical::Move(x, y)
        }
        "scroll" => Physical::Scroll {
            x: req.x.unwrap_or(screen.width as i64 / 2),
            y: req.y.unwrap_or(screen.height as i64 / 2),
            sx: req.scroll_x.unwrap_or(0),
            sy: req.scroll_y.unwrap_or(3),
        },
        "drag" => {
            let from = require_xy(req)?;
            let to = match (req.to_x, req.to_y) {
                (Some(tx), Some(ty)) => (tx, ty),
                _ => return Err(Error::Handler("action 'drag' needs to_x and to_y".to_string())),
            };
            Physical::Drag { from, to, button: req.button.clone().unwrap_or_else(|| "left".into()) }
        }
        "type" => Physical::Type(
            req.text
                .clone()
                .ok_or_else(|| Error::Handler("action 'type' needs text".to_string()))?,
        ),
        "press" | "hotkey" | "key" => Physical::Keys(
            req.keys.clone().filter(|k| !k.is_empty()).ok_or_else(|| {
                Error::Handler(format!("action '{}' needs a non-empty keys array", req.action))
            })?,
        ),
        other => {
            return Err(Error::Handler(format!(
                "unknown action '{other}' (by coordinate: click, right_click, double_click, \
                 move, drag, scroll, type, press, hotkey; by ref: invoke, set_value, toggle, select)"
            )))
        }
    })
}

async fn dispatch(driver: &Arc<dyn Driver>, p: Physical) -> Result<String, Error> {
    Ok(match p {
        Physical::Click(x, y) => {
            driver.left_click(x, y).await.map_err(Error::Handler)?;
            format!("left click at ({x}, {y})")
        }
        Physical::RightClick(x, y) => {
            driver.right_click(x, y).await.map_err(Error::Handler)?;
            format!("right click at ({x}, {y})")
        }
        Physical::DoubleClick(x, y) => {
            driver.double_click(x, y).await.map_err(Error::Handler)?;
            format!("double click at ({x}, {y})")
        }
        Physical::Move(x, y) => {
            driver.move_cursor(x, y).await.map_err(Error::Handler)?;
            format!("move to ({x}, {y})")
        }
        Physical::Scroll { x, y, sx, sy } => {
            driver.scroll(x, y, sx, sy).await.map_err(Error::Handler)?;
            format!("scroll ({sx}, {sy}) at ({x}, {y})")
        }
        Physical::Drag { from, to, button } => {
            driver
                .drag(from, to, &button)
                .await
                .map_err(Error::Handler)?;
            format!("drag ({}, {}) -> ({}, {})", from.0, from.1, to.0, to.1)
        }
        Physical::Type(text) => {
            driver.type_text(&text).await.map_err(Error::Handler)?;
            format!("type {} chars", text.chars().count())
        }
        Physical::Keys(keys) => {
            driver.keypress(&keys).await.map_err(Error::Handler)?;
            format!("press {}", keys.join("+"))
        }
    })
}

pub(crate) async fn handle(session: &Arc<Session>, req: ActInput) -> Result<ActOutput, Error> {
    if let Some(verb) = Verb::parse(&req.action) {
        return by_ref(session, &req, verb).await;
    }
    if req.r#ref.is_some() {
        return Err(invalid(if is_coordinate(&req.action) {
            format!(
                "action '{}' acts by coordinates; `ref` is only for invoke, set_value, toggle and select",
                req.action
            )
        } else {
            format!(
                "unknown action '{}'; with `ref` use invoke, set_value, toggle or select",
                req.action
            )
        }));
    }
    let planned = plan(&req, session.screen)?;
    let _guard = session.act_lock.lock().await;
    session.touch();
    // Before the input too, so refs taken earlier die even if this handler
    // is cancelled mid-dispatch.
    if let Some(semantic) = &session.semantic {
        semantic.invalidate();
    }
    let result = dispatch(session.driver(), planned).await;
    // Input may have reached the desktop even when the driver reported an
    // error, and a walk that ran during it is stale too.
    if let Some(semantic) = &session.semantic {
        semantic.invalidate();
    }
    Ok(ActOutput {
        ok: true,
        detail: result?,
    })
}

async fn by_ref(session: &Arc<Session>, req: &ActInput, verb: Verb) -> Result<ActOutput, Error> {
    let stray: Vec<&str> = [
        ("x", req.x.is_some()),
        ("y", req.y.is_some()),
        ("to_x", req.to_x.is_some()),
        ("to_y", req.to_y.is_some()),
        ("button", req.button.is_some()),
        ("keys", req.keys.is_some()),
        ("scroll_x", req.scroll_x.is_some()),
        ("scroll_y", req.scroll_y.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, set)| set.then_some(name))
    .collect();
    if !stray.is_empty() {
        return Err(invalid(format!(
            "{} acts by ref; drop {}",
            verb.as_str(),
            stray.join(", ")
        )));
    }
    let r = req
        .r#ref
        .as_deref()
        .filter(|r| !r.is_empty())
        .ok_or_else(|| {
            invalid(format!(
                "{} needs `ref` from computer::observe or computer::find",
                verb.as_str()
            ))
        })?;
    let text = match (verb, req.text.as_deref()) {
        (Verb::SetValue, None) => return Err(invalid("set_value needs `text`")),
        (Verb::SetValue, Some(t)) if t.chars().count() > MAX_SET_VALUE_CHARS => {
            return Err(invalid(format!(
                "text is longer than {MAX_SET_VALUE_CHARS} characters"
            )))
        }
        (Verb::SetValue, Some(t)) => Some(t.to_string()),
        (_, Some(_)) => return Err(invalid(format!("{} does not take `text`", verb.as_str()))),
        (_, None) => None,
    };
    let Some(layer) = &session.semantic else {
        return Err(SemanticError::coordinates_only().into_iii());
    };
    let _guard = session.act_lock.lock().await;
    session.touch();
    let detail = layer
        .act(r.to_string(), verb, text)
        .await
        .map_err(SemanticError::into_iii)?;
    Ok(ActOutput { ok: true, detail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::fake::FakeTree;
    use crate::semantic::{Query, Scope, SemanticSession};
    use crate::test_support::{code, session_with};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use xa11y_core::Role;

    fn req(v: serde_json::Value) -> ActInput {
        serde_json::from_value(v).unwrap()
    }

    async fn semantic_session() -> (
        Arc<crate::session::Session>,
        Arc<crate::test_support::FakeDesktop>,
        Arc<FakeTree>,
        String,
    ) {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::Button, "Salvar");
        let t = Arc::new(t);
        let (s, d) = session_with(Some(SemanticSession::new(t.factory())));
        let q = Query {
            scope: Scope::default(),
            selector: None,
            max_nodes: 256,
            max_depth: 6,
        };
        let out = s
            .semantic
            .as_ref()
            .unwrap()
            .snapshot(q, true)
            .await
            .unwrap();
        let line = out.text.lines().find(|l| l.contains("Salvar")).unwrap();
        let r = line
            .trim_start()
            .strip_prefix("[ref=")
            .unwrap()
            .split(']')
            .next()
            .unwrap()
            .to_string();
        (s, d, t, r)
    }

    #[tokio::test]
    async fn physical_click_hits_the_driver_once() {
        let (s, d) = session_with(None);
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"click","x":42,"y":17})),
        )
        .await
        .unwrap();
        assert_eq!(out.detail, "left click at (42, 17)");
        assert_eq!(d.inputs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn semantic_invoke_never_touches_the_driver() {
        let (s, d, t, r) = semantic_session().await;
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"invoke","ref":r})),
        )
        .await
        .unwrap();
        assert!(out.ok && out.detail.starts_with("dispatched invoke on"));
        assert_eq!(t.actions(), vec!["press:Salvar"]);
        assert_eq!(d.inputs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn mixed_payloads_are_refused_without_any_effect() {
        let (s, d, t, r) = semantic_session().await;
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"invoke","ref":r,"x":42,"y":17})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "invalid_input");
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"click","ref":r,"x":1,"y":1})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "invalid_input");
        assert!(t.actions().is_empty());
        assert_eq!(d.inputs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn text_rules_per_verb() {
        let (s, _, t, r) = semantic_session().await;
        for bad in [
            serde_json::json!({"session_id":"c1","action":"set_value","ref":r}),
            serde_json::json!({"session_id":"c1","action":"invoke","ref":r,"text":"x"}),
            serde_json::json!({"session_id":"c1","action":"set_value","ref":r,"text":"a".repeat(16_385)}),
            serde_json::json!({"session_id":"c1","action":"toggle"}),
        ] {
            assert_eq!(
                code(handle(&s, req(bad)).await.unwrap_err()),
                "invalid_input"
            );
        }
        assert!(t.actions().is_empty());
    }

    #[tokio::test]
    async fn semantic_actions_need_a_semantic_session() {
        let (s, _) = session_with(None);
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"invoke","ref":"e1"})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "unsupported_session");
    }

    #[tokio::test]
    async fn a_coordinate_action_invalidates_refs() {
        let (s, d, t, r) = semantic_session().await;
        handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"click","x":1,"y":1})),
        )
        .await
        .unwrap();
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"invoke","ref":r})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "stale_ref");
        assert!(t.actions().is_empty());
        assert_eq!(d.inputs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_click_during_an_observe_walk_invalidates_its_refs() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::Button, "Salvar");
        let t = Arc::new(t);
        let (s, d) = session_with(Some(SemanticSession::new(t.factory())));
        t.block_reads.store(true, Ordering::SeqCst);
        let bg = s.clone();
        let walking = tokio::spawn(async move {
            crate::functions::observe::handle(
                &bg,
                serde_json::from_value(serde_json::json!({
                    "session_id":"c1","include_image":false,"include_a11y":true
                }))
                .unwrap(),
            )
            .await
            .unwrap()
        });
        while t.children_calls() == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        // A console click lands while the tree is being read.
        handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"click","x":5,"y":5})),
        )
        .await
        .unwrap();
        t.block_reads.store(false, Ordering::SeqCst);
        let out = walking.await.unwrap();
        let text = out.content[0].text.clone().unwrap();
        let line = text.lines().find(|l| l.contains("Salvar")).unwrap();
        let r = line
            .trim_start()
            .strip_prefix("[ref=")
            .unwrap()
            .split(']')
            .next()
            .unwrap()
            .to_string();
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"invoke","ref":r})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "stale_ref");
        assert!(t.actions().is_empty());
        assert_eq!(d.inputs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_coordinate_action_cancelled_mid_dispatch_still_invalidates() {
        let (s, d, t, r) = semantic_session().await;
        d.hold.store(true, Ordering::SeqCst);
        // The caller gives up while the click is inside the driver.
        tokio::select! {
            res = handle(
                &s,
                req(serde_json::json!({"session_id":"c1","action":"click","x":1,"y":1})),
            ) => panic!("the click must still be pending: {:?}", res.map(|o| o.detail)),
            () = async {
                while d.inputs.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            } => {}
        }
        d.hold.store(false, Ordering::SeqCst);
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"invoke","ref":r})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "stale_ref");
        assert!(t.actions().is_empty());
    }

    #[tokio::test]
    async fn a_refused_coordinate_action_does_not_invalidate() {
        let (s, _, t, r) = semantic_session().await;
        // `click` without coordinates is refused before any input.
        handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"click"})),
        )
        .await
        .unwrap_err();
        handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"invoke","ref":r})),
        )
        .await
        .unwrap();
        assert_eq!(t.actions().len(), 1);
    }

    #[tokio::test]
    async fn a_misspelled_ref_action_is_named_unknown() {
        let (s, d, t, r) = semantic_session().await;
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","action":"clik","ref":r})),
        )
        .await
        .unwrap_err();
        let iii_sdk::errors::Error::Remote { code, message, .. } = e else {
            panic!("expected Error::Remote");
        };
        assert_eq!(code, "invalid_input");
        assert!(
            message.contains("unknown action 'clik'")
                && message.contains("invoke, set_value, toggle or select"),
            "{message}"
        );
        assert!(t.actions().is_empty());
        assert_eq!(d.inputs.load(Ordering::SeqCst), 0);
    }
}
