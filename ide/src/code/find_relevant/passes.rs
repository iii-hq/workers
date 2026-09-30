//! File roles and read-early priority, judged beside evidence selection.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/retrieve.ts` `assessFiles` (596-622).
//!
//! Deviation: the assessment reads the discovery preview without re-reading
//! the file; its roles attach only where the evidence re-hash held (the
//! caller's check, retrieve.ts 643-650); a request over `Run::window_cap`
//! is not sent (jevgrep has no cap here).

use std::collections::HashMap;
use std::sync::Arc;

use super::navigate::Run;
use super::prompts;
use crate::code::judge::{JudgeError, Scores};

/// Roles and priority above this count (strict).
const ROLE: f64 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    /// In jevgrep's role order.
    pub roles: Vec<String>,
    pub priority: f64,
}

impl Assessment {
    fn from_scores(scores: &Scores) -> Self {
        Self {
            roles: prompts::ROLES
                .iter()
                .filter(|(role, _)| scores.get(*role).is_some_and(|p| *p > ROLE))
                .map(|(role, _)| role.to_string())
                .collect(),
            priority: scores.get("priority").copied().unwrap_or(0.0),
        }
    }
}

/// One file-assessment request per candidate, reusing its discovery
/// preview; a failed request leaves the file unassessed.
pub async fn assess_files(run: &Arc<Run>) -> HashMap<String, Assessment> {
    run.parallel(run.admitted(), |run, candidate| async move {
        let preview = run.state().previews.get(&candidate.path).cloned()?;
        let request = prompts::file_assessment(&run.query, &candidate.path, &preview);
        if prompts::request_bytes(&request) > run.window_cap {
            run.issue("request-size");
            return None;
        }
        match run.call(request).await {
            Ok(scores) => Some((candidate.path, Assessment::from_scores(&scores))),
            Err(JudgeError::TooLarge) => {
                run.issue("request-size");
                None
            }
            Err(_) => None, // recorded by `call`
        }
    })
    .await
    .into_iter()
    .flatten()
    .collect()
}
