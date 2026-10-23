//! `computer::find` — elements matching a selector in an app's accessibility
//! tree, optionally waiting for one to appear. One line per match, each with
//! a ref for `computer::act`; never picks a match for the caller.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::invalid;
use super::observe::TreeScope;
use crate::semantic::{
    parse_selector, Code, SemanticError, SnapshotOut, FIND_POLL, MAX_FIND_TIMEOUT_MS,
};
use crate::session::Session;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindInput {
    pub session_id: String,
    /// CSS-like selector: snake_case role, [attr='v'] filters (=, *=, ^=, $=;
    /// name, value, description, role and states such as enabled, checked,
    /// selected), descendant (space) and child (>) combinators, :nth(n), and
    /// comma groups. Example: dialog[name='Save'] button[name='OK'].
    pub selector: String,
    #[serde(flatten)]
    pub scope: TreeScope,
    /// Keep looking for up to this many ms until something matches, also
    /// waiting for the app itself to start (app_name) or come to the
    /// foreground (0-30000, default 0 = one attempt). Not with root_ref.
    #[serde(default)]
    pub timeout_ms: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FindOutput {
    /// Untrusted app text: a marker line, the `app "name" pid N` line, then
    /// one line per match (at most 50) with its ref and up to three named
    /// ancestors.
    pub tree: String,
    /// Total matches (may exceed the lines listed).
    pub matches: u32,
    /// The last walk stopped at its node budget or deadline: zero matches may
    /// be a false negative.
    pub truncated: bool,
    pub waited_ms: u64,
}

fn output(out: SnapshotOut, start: Instant) -> FindOutput {
    FindOutput {
        tree: out.text,
        matches: out.matches as u32,
        truncated: out.truncated,
        waited_ms: start.elapsed().as_millis() as u64,
    }
}

pub(crate) async fn handle(session: &Arc<Session>, req: FindInput) -> Result<FindOutput, Error> {
    let timeout_ms = req.timeout_ms.unwrap_or(0);
    if timeout_ms > MAX_FIND_TIMEOUT_MS {
        return Err(invalid(format!(
            "timeout_ms must be 0-{MAX_FIND_TIMEOUT_MS}"
        )));
    }
    if timeout_ms > 0 && req.scope.root_ref.is_some() {
        return Err(invalid(
            "root_ref takes a single attempt; drop timeout_ms, or scope with app_name and a \
             descendant selector such as dialog[name='Save'] button[name='OK']"
                .to_string(),
        ));
    }
    let group = parse_selector(&req.selector).map_err(SemanticError::into_iii)?;
    let query = req
        .scope
        .query(Some(group))
        .map_err(SemanticError::into_iii)?;
    let Some(layer) = &session.semantic else {
        return Err(SemanticError::coordinates_only().into_iii());
    };
    let start = Instant::now();
    let limit = Duration::from_millis(timeout_ms as u64);
    let mut last = None;
    let mut busy = None;
    let mut app_missing = false;
    loop {
        session.touch();
        match layer.snapshot(query.clone(), false).await {
            Ok(out) if out.matches > 0 => return Ok(output(out, start)),
            Ok(out) => last = Some(out),
            Err(e) if e.code == Code::Busy => busy = Some(e),
            // The app may still be starting: wait for it like for a match.
            Err(e) if e.app_missing && timeout_ms > 0 => app_missing = true,
            Err(e) => return Err(e.into_iii()),
        }
        // Start another attempt only if it would begin before the deadline.
        if start.elapsed() + FIND_POLL >= limit {
            break;
        }
        tokio::time::sleep(FIND_POLL).await;
    }
    let waited = start.elapsed().as_millis();
    match last {
        // The app answered at least once: no match is a result, not an error.
        Some(out) => Ok(output(out, start)),
        None if app_missing => Err(invalid(match &req.scope.app_name {
            Some(name) => {
                format!("no running application named \"{name}\" after waiting {waited} ms")
            }
            None => format!("no application came to the foreground after waiting {waited} ms"),
        })),
        // Every attempt in the window met another operation: report busy.
        None => Err(busy
            .unwrap_or_else(|| SemanticError::new(Code::Busy, "no attempt could run"))
            .into_iii()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::fake::FakeTree;
    use crate::semantic::SemanticSession;
    use crate::test_support::{code, session_with};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use xa11y_core::Role;

    fn req(v: serde_json::Value) -> FindInput {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn one_attempt_by_default_reaching_deep_matches() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        let g = t.add(w, Role::Group, "G");
        let p = t.add(g, Role::Group, "P");
        t.add(p, Role::Button, "Fundo");
        let t = Arc::new(t);
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='Fundo']"})),
        )
        .await
        .unwrap();
        assert_eq!(out.matches, 1);
        assert!(!out.truncated);
    }

    #[tokio::test]
    async fn waits_for_a_late_match() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        let t = Arc::new(t);
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let later = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            later.add(w, Role::Button, "Pronto");
        });
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='Pronto']","timeout_ms":3000})),
        )
        .await
        .unwrap();
        assert_eq!(out.matches, 1);
        assert!(out.waited_ms >= 200, "{}", out.waited_ms);
    }

    #[tokio::test]
    async fn no_match_until_the_deadline_is_an_empty_result_not_an_error() {
        let t = FakeTree::new("App", 1);
        t.add(0, Role::Window, "W");
        let t = Arc::new(t);
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let start = Instant::now();
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='Nunca']","timeout_ms":300})),
        )
        .await
        .unwrap();
        assert_eq!(out.matches, 0);
        assert!(out.tree.starts_with(crate::semantic::outline_marker()));
        assert!(out.tree.contains("no matches"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    /// Hold the executor busy with a hung read until `release` is cleared.
    async fn hold_busy(
        s: &Arc<crate::session::Session>,
        t: &Arc<FakeTree>,
    ) -> tokio::task::JoinHandle<()> {
        let before = t.children_calls();
        t.block_reads
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let bg = s.clone();
        let handle = tokio::spawn(async move {
            let q = crate::semantic::Query {
                scope: crate::semantic::Scope::default(),
                selector: None,
                max_nodes: 256,
                max_depth: 6,
            };
            let _ = bg.semantic.as_ref().unwrap().snapshot(q, true).await;
        });
        while t.children_calls() <= before {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        handle
    }

    #[tokio::test]
    async fn busy_attempts_are_skipped_and_retried() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::Button, "OK");
        let t = Arc::new(t);
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let bg = hold_busy(&s, &t).await;
        let release = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            release
                .block_reads
                .store(false, std::sync::atomic::Ordering::SeqCst);
        });
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='OK']","timeout_ms":3000})),
        )
        .await
        .unwrap();
        assert_eq!(out.matches, 1);
        bg.await.unwrap();
    }

    #[tokio::test]
    async fn a_single_busy_attempt_is_a_busy_error() {
        let t = FakeTree::new("App", 1);
        t.add(0, Role::Window, "W");
        let t = Arc::new(t);
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let bg = hold_busy(&s, &t).await;
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"window"})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "busy");
        t.block_reads
            .store(false, std::sync::atomic::Ordering::SeqCst);
        bg.await.unwrap();
    }

    #[tokio::test]
    async fn input_errors() {
        let (s, _) = session_with(Some(SemanticSession::new(
            Arc::new(FakeTree::new("A", 1)).factory(),
        )));
        assert_eq!(
            code(
                handle(
                    &s,
                    req(serde_json::json!({"session_id":"c1","selector":"buton"}))
                )
                .await
                .unwrap_err()
            ),
            "invalid_selector"
        );
        assert_eq!(
            code(handle(&s, req(serde_json::json!({"session_id":"c1","selector":"button","timeout_ms":30001}))).await.unwrap_err()),
            "invalid_input"
        );
        assert_eq!(
            code(handle(&s, req(serde_json::json!({"session_id":"c1","selector":"button","root_ref":"e1","timeout_ms":10}))).await.unwrap_err()),
            "invalid_input"
        );
        let (plain, _) = session_with(None);
        assert_eq!(
            code(
                handle(
                    &plain,
                    req(serde_json::json!({"session_id":"c1","selector":"button"}))
                )
                .await
                .unwrap_err()
            ),
            "unsupported_session"
        );
    }

    #[tokio::test]
    async fn stopping_the_session_ends_a_waiting_find() {
        let t = FakeTree::new("App", 1);
        t.add(0, Role::Window, "W");
        let t = Arc::new(t);
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let closer = s.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            closer.semantic.as_ref().unwrap().close();
        });
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='Nunca']","timeout_ms":5000})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "unsupported_session");
    }

    #[tokio::test]
    async fn a_timed_find_waits_for_the_app_to_start() {
        let t = Arc::new(FakeTree::new("App", 1));
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let later = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let doc = later.add_app("Doc", 9);
            later.add(doc, Role::Button, "OK");
        });
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='OK']","app_name":"Doc","timeout_ms":3000})),
        )
        .await
        .unwrap();
        assert_eq!(out.matches, 1, "{}", out.tree);
        assert!(out.waited_ms >= 250, "{}", out.waited_ms);
    }

    #[tokio::test]
    async fn an_app_that_never_starts_is_an_input_error_after_the_wait() {
        let t = Arc::new(FakeTree::new("App", 1));
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let start = Instant::now();
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button","app_name":"Nunca","timeout_ms":300})),
        )
        .await
        .unwrap_err();
        let iii_sdk::errors::Error::Remote { code, message, .. } = e else {
            panic!("expected Error::Remote");
        };
        assert_eq!(code, "invalid_input");
        assert!(
            message.contains("no running application named \"Nunca\" after waiting"),
            "{message}"
        );
        assert!(start.elapsed() >= Duration::from_millis(200));
    }

    #[tokio::test]
    async fn an_app_that_starts_but_never_matches_is_an_empty_result() {
        let t = Arc::new(FakeTree::new("App", 1));
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let later = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            later.add_app("Doc", 9);
        });
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='OK']","app_name":"Doc","timeout_ms":500})),
        )
        .await
        .unwrap();
        assert_eq!(out.matches, 0);
        assert!(out.tree.contains("no matches"), "{}", out.tree);
    }

    #[tokio::test]
    async fn a_missing_pid_fails_at_once_even_with_a_wait() {
        let t = Arc::new(FakeTree::new("App", 1));
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let start = Instant::now();
        let e = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button","app_pid":999,"timeout_ms":3000})),
        )
        .await
        .unwrap_err();
        assert_eq!(code(e), "invalid_input");
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn a_timed_find_waits_for_a_foreground_app() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::Button, "OK");
        let t = Arc::new(t);
        *t.foreground_error.lock().unwrap() = Some(xa11y_core::Error::selector_not_matched(
            "focused application",
        ));
        let (s, _) = session_with(Some(SemanticSession::new(t.factory())));
        let later = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            *later.foreground_error.lock().unwrap() = None;
        });
        let out = handle(
            &s,
            req(serde_json::json!({"session_id":"c1","selector":"button[name='OK']","timeout_ms":3000})),
        )
        .await
        .unwrap();
        assert_eq!(out.matches, 1);
    }
}
