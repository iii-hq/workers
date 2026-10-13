//! `computer::observe` — a screenshot and, with `include_a11y`, an app's
//! accessibility tree as text lines carrying refs for `computer::act`.

use std::sync::Arc;

use iii_sdk::errors::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::screenshot::ContentBlock;
use crate::driver::Screen;
use crate::semantic::{Code, Query, Scope, SemanticError, DEFAULT_MAX_DEPTH, MAX_DEPTH, MAX_NODES};
use crate::session::Session;

// Which tree to read and how much of it. Shared by observe and find. A plain
// comment, not a doc comment: schemars would copy it into the description of
// every request schema that flattens this struct.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TreeScope {
    /// Process id of the app to read (default: the foreground app).
    /// Exclusive with `app_name` and `root_ref`.
    #[serde(default)]
    pub app_pid: Option<u32>,
    /// Exact, case-sensitive app name as the OS reports it (macOS: the app
    /// name; Windows: the executable name without .exe, e.g. `notepad`, or the
    /// window title when the process cannot be opened). Exclusive with
    /// `app_pid` and `root_ref`.
    #[serde(default)]
    pub app_name: Option<String>,
    /// A ref from the current snapshot to read beneath (single attempt).
    /// Exclusive with `app_pid` and `app_name`.
    #[serde(default)]
    pub root_ref: Option<String>,
    /// Most elements to read, 1-256. Default 256.
    #[serde(default)]
    pub max_nodes: Option<u32>,
    /// Deepest level below the root (the root is level 0), 1-12. Default 6.
    #[serde(default)]
    pub max_depth: Option<u32>,
}

impl TreeScope {
    pub fn is_set(&self) -> bool {
        self.app_pid.is_some()
            || self.app_name.is_some()
            || self.root_ref.is_some()
            || self.max_nodes.is_some()
            || self.max_depth.is_some()
    }

    pub fn query(
        &self,
        selector: Option<xa11y_core::SelectorGroup>,
    ) -> Result<Query, SemanticError> {
        let invalid = |m: &str| SemanticError::new(Code::InvalidInput, m);
        let scoped = [
            self.app_pid.is_some(),
            self.app_name.is_some(),
            self.root_ref.is_some(),
        ];
        if scoped.iter().filter(|s| **s).count() > 1 {
            return Err(invalid(
                "app_pid, app_name and root_ref are mutually exclusive",
            ));
        }
        if self
            .app_name
            .as_deref()
            .is_some_and(|n| n.trim().is_empty())
        {
            return Err(invalid("app_name is empty"));
        }
        if self.app_name.as_deref().is_some_and(|n| n.contains('"')) {
            return Err(invalid("app_name cannot contain a double quote"));
        }
        if self.root_ref.as_deref().is_some_and(str::is_empty) {
            return Err(invalid("root_ref is empty"));
        }
        let max_nodes = self.max_nodes.unwrap_or(MAX_NODES as u32);
        if !(1..=MAX_NODES as u32).contains(&max_nodes) {
            return Err(invalid(&format!("max_nodes must be 1-{MAX_NODES}")));
        }
        let max_depth = self.max_depth.unwrap_or(DEFAULT_MAX_DEPTH);
        if !(1..=MAX_DEPTH).contains(&max_depth) {
            return Err(invalid(&format!("max_depth must be 1-{MAX_DEPTH}")));
        }
        Ok(Query {
            scope: Scope {
                app_pid: self.app_pid,
                app_name: self.app_name.clone(),
                root_ref: self.root_ref.clone(),
            },
            selector,
            max_nodes: max_nodes as usize,
            max_depth,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObserveInput {
    pub session_id: String,
    /// Capture the screen. Default true; false needs include_a11y=true.
    #[serde(default)]
    pub include_image: Option<bool>,
    /// Also read an app's accessibility tree as text lines with `[ref=…]`
    /// handles for computer::act. Native macOS/Windows sessions only.
    #[serde(default)]
    pub include_a11y: Option<bool>,
    #[serde(flatten)]
    pub scope: TreeScope,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ObserveDetails {
    pub session_id: String,
    pub screen: Screen,
    /// Image mime; absent when no image was captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// The tree walk stopped at its node budget or deadline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ObserveOutput {
    pub content: Vec<ContentBlock>,
    pub details: ObserveDetails,
}

pub(crate) async fn handle(
    session: &Arc<Session>,
    req: ObserveInput,
) -> Result<ObserveOutput, Error> {
    let include_image = req.include_image.unwrap_or(true);
    let include_a11y = req.include_a11y.unwrap_or(false);
    let invalid = |m: &str| SemanticError::new(Code::InvalidInput, m).into_iii();
    if !include_image && !include_a11y {
        return Err(invalid("include_image=false needs include_a11y=true"));
    }
    if !include_a11y && req.scope.is_set() {
        return Err(invalid(
            "app_pid, app_name, root_ref, max_nodes and max_depth need include_a11y=true",
        ));
    }
    let query = if include_a11y {
        Some(req.scope.query(None).map_err(SemanticError::into_iii)?)
    } else {
        None
    };
    session.touch();
    let shot = if include_image {
        Some(super::capture(session).await?)
    } else {
        None
    };
    let mut text = match &shot {
        Some(shot) => format!(
            "Desktop of session {} ({}x{}, {} bytes)",
            session.id,
            session.screen.width,
            session.screen.height,
            shot.byte_len()
        ),
        None => format!("Accessibility tree of session {}", session.id),
    };
    let mut truncated = None;
    if let Some(query) = query {
        let result = match &session.semantic {
            Some(layer) => layer.snapshot(query, true).await,
            None => Err(SemanticError::new(
                Code::UnsupportedSession,
                "this session drives a remote or sandboxed desktop by coordinates only",
            )),
        };
        match result {
            Ok(out) => {
                truncated = Some(out.truncated);
                text.push('\n');
                text.push_str(&out.text);
            }
            Err(e) if shot.is_some() => {
                text.push_str(&format!(
                    "\na11y tree unavailable: {}: {}",
                    e.code.as_str(),
                    e.message
                ));
            }
            Err(e) => return Err(e.into_iii()),
        }
    }
    let mut content = Vec::new();
    if let Some(shot) = &shot {
        content.push(ContentBlock::image(shot.mime.clone(), shot.to_base64()));
    }
    content.push(ContentBlock::text(text));
    Ok(ObserveOutput {
        content,
        details: ObserveDetails {
            session_id: session.id.clone(),
            screen: session.screen,
            mime: shot.map(|s| s.mime),
            truncated,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::fake::FakeTree;
    use crate::semantic::{outline_marker, SemanticSession};
    use crate::test_support::session_with;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use xa11y_core::{Error as X, Role};

    fn code(e: iii_sdk::errors::Error) -> String {
        match e {
            iii_sdk::errors::Error::Remote { code, .. } => code,
            other => format!("handler: {other}"),
        }
    }

    fn req(v: serde_json::Value) -> ObserveInput {
        serde_json::from_value(v).unwrap()
    }

    fn tree() -> Arc<FakeTree> {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::Button, "Salvar");
        Arc::new(t)
    }

    #[tokio::test]
    async fn the_old_call_still_captures_and_skips_the_tree() {
        let t = tree();
        let (s, d) = session_with(Some(SemanticSession::new(t.factory())));
        let out = handle(&s, req(serde_json::json!({"session_id":"c1"})))
            .await
            .unwrap();
        assert_eq!(out.content.len(), 2);
        assert_eq!(out.content[0].r#type, "image");
        assert_eq!(out.details.mime.as_deref(), Some("image/jpeg"));
        assert_eq!(d.shots.load(Ordering::SeqCst), 1);
        assert_eq!(t.children_calls(), 0);
    }

    #[tokio::test]
    async fn tree_only_never_captures() {
        let t = tree();
        let (s, d) = session_with(Some(SemanticSession::new(t.factory())));
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","include_image":false,"include_a11y":true})),
        )
        .await
        .unwrap();
        assert_eq!(d.shots.load(Ordering::SeqCst), 0);
        assert_eq!(out.content.len(), 1);
        let text = out.content[0].text.as_deref().unwrap();
        assert!(
            text.contains(outline_marker()) && text.contains("button \"Salvar\""),
            "{text}"
        );
        assert!(out.details.mime.is_none());
        assert_eq!(out.details.truncated, Some(false));
    }

    #[tokio::test]
    async fn invalid_combinations_are_refused_before_capture() {
        let (s, d) = session_with(None);
        for bad in [
            serde_json::json!({"session_id":"c1","include_image":false}),
            serde_json::json!({"session_id":"c1","app_name":"X"}),
            serde_json::json!({"session_id":"c1","include_a11y":true,"max_nodes":0}),
            serde_json::json!({"session_id":"c1","include_a11y":true,"max_depth":13}),
            serde_json::json!({"session_id":"c1","include_a11y":true,"app_pid":1,"app_name":"X"}),
            serde_json::json!({"session_id":"c1","include_a11y":true,"app_name":"Ação \"X\""}),
            serde_json::json!({"session_id":"c1","include_a11y":true,"app_name":""}),
            serde_json::json!({"session_id":"c1","include_a11y":true,"app_name":"  "}),
        ] {
            assert_eq!(
                code(handle(&s, req(bad)).await.unwrap_err()),
                "invalid_input"
            );
        }
        assert_eq!(d.shots.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unicode_app_names_are_accepted() {
        let t = FakeTree::new("Ação €", 1);
        t.add(0, Role::Window, "W");
        let t = Arc::new(t);
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","include_image":false,"include_a11y":true,"app_name":"Ação €"})),
        )
        .await
        .unwrap();
        assert!(out.content[0]
            .text
            .as_deref()
            .unwrap()
            .contains("window \"W\""));
    }

    #[tokio::test]
    async fn a_tree_failure_keeps_the_image() {
        let factory: crate::semantic::ProviderFactory = Arc::new(|| {
            Err(X::PermissionDenied {
                instructions: "enable Accessibility".into(),
            })
        });
        let (s, _) = session_with(Some(SemanticSession::new(factory)));
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","include_a11y":true})),
        )
        .await
        .unwrap();
        assert_eq!(out.content[0].r#type, "image");
        let text = out.content[1].text.as_deref().unwrap();
        assert!(
            text.contains("a11y tree unavailable: permission_denied"),
            "{text}"
        );
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","include_a11y":true,"include_image":false})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "permission_denied");
    }

    #[tokio::test]
    async fn coordinate_only_sessions_say_so() {
        let (s, _) = session_with(None);
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","include_a11y":true})),
        )
        .await
        .unwrap();
        assert!(out.content[1]
            .text
            .as_deref()
            .unwrap()
            .contains("a11y tree unavailable: unsupported_session"));
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","include_a11y":true,"include_image":false})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "unsupported_session");
    }

    #[tokio::test]
    async fn a_failed_capture_installs_no_snapshot() {
        let t = tree();
        let (s, d) = session_with(Some(SemanticSession::new(t.factory())));
        d.fail.store(true, Ordering::SeqCst);
        handle(
            &s,
            req(serde_json::json!({"session_id":"c1","include_a11y":true})),
        )
        .await
        .unwrap_err();
        assert_eq!(t.children_calls(), 0);
    }
}
