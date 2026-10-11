//! `context-clm::status` — one session's projection, budget and controller
//! counters, read from its persisted state (spec §5.1).

use iii_sdk::errors::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::assemble::{state_error, window, Deps};
use crate::controller::raw_count;
use crate::store::Stats;

pub const STATUS_ID: &str = "context-clm::status";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusRequest {
    pub session_id: String,
}

/// `found: false` (and zero counters) for a session that never ran a CLM step.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusResponse {
    pub session_id: String,
    pub found: bool,
    /// Messages in the editable projection (after the task and the ledger).
    pub projection_messages: usize,
    /// Calibrated size of task + ledger + projection (no system prompt, no tools).
    pub projection_tokens: u64,
    /// Budget enforced on the session's last step: its `context_budget_tokens`
    /// or the worker default, lowered to the model's usable room.
    pub budget_tokens: u64,
    /// The worker-wide default (`budget_tokens` in the configuration).
    pub configured_budget_tokens: u64,
    /// Provider prompt tokens per raw ruler token.
    pub calibration_ratio: f64,
    /// The last step ended the run (`applied.strategy_stop`).
    pub stopped: bool,
    /// Cumulative counters: steps, edits applied/rejected, rollbacks, stops,
    /// peak calibrated size (`peak_tc`) and stripped thinking blocks.
    pub stats: Stats,
}

// NOTE: no session lock: `Store::save` renames into place, so a read sees the
// state before or after a step in flight. An unreadable state file is an
// error here (it is not quarantined; the next step does that).
pub async fn status<D>(deps: &Deps<D>, req: StatusRequest) -> Result<StatusResponse, Error> {
    let configured_budget_tokens = deps.config.read().await.budget_tokens;
    let state = deps
        .store
        .load(&req.session_id)
        .map_err(|e| state_error(&req.session_id, &e))?;
    let Some(st) = state else {
        return Ok(StatusResponse {
            session_id: req.session_id,
            configured_budget_tokens,
            ..StatusResponse::default()
        });
    };
    let size = raw_count("", &window(&st, &st.proj, &[]));
    Ok(StatusResponse {
        session_id: req.session_id,
        found: true,
        projection_messages: st.proj.len(),
        projection_tokens: (size as f64 * st.ratio) as u64,
        budget_tokens: st.budget,
        configured_budget_tokens,
        calibration_ratio: st.ratio,
        stopped: st.stopped,
        stats: st.stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::Locks;
    use crate::config::ClmConfig;
    use crate::store::{SessionState, Store};
    use crate::types::Msg;
    use serde_json::json;

    fn deps(dir: &std::path::Path) -> Deps<()> {
        Deps {
            delegate: (),
            store: Store::new(dir.to_path_buf()),
            config: crate::configuration::new_cell(ClmConfig::default()),
            locks: Locks::default(),
        }
    }

    #[tokio::test]
    async fn an_unknown_session_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let req = StatusRequest {
            session_id: "s".into(),
        };
        let resp = status(&deps(dir.path()), req).await.unwrap();
        let expected = StatusResponse {
            session_id: "s".into(),
            configured_budget_tokens: 65_536,
            ..StatusResponse::default()
        };
        assert_eq!(resp, expected);
    }

    #[tokio::test]
    async fn an_unreadable_state_is_an_error_and_is_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        let path = dir
            .path()
            .join(format!("{}.json", crate::mirror::safe_id("s")));
        std::fs::write(&path, b"{ torn").unwrap();
        let req = StatusRequest {
            session_id: "s".into(),
        };
        let err = status(&d, req).await.unwrap_err().to_string();
        assert!(err.contains("session state for s"), "{err}");
        // Status only reads: the next step quarantines it.
        assert_eq!(std::fs::read(&path).unwrap(), b"{ torn");
    }

    #[tokio::test]
    async fn reports_counters_and_the_calibrated_size() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        let msg: Msg = serde_json::from_value(json!({
            "role": "user", "content": [{ "type": "text", "text": "x".repeat(400) }], "timestamp": 1
        }))
        .unwrap();
        let mut st = SessionState {
            raw0: Some(msg.clone()),
            proj: vec![msg.clone()],
            ratio: 1.5,
            stopped: true,
            budget: 32_768,
            ..SessionState::default()
        };
        st.stats.steps = 7;
        st.stats.edits_applied = 2;
        st.stats.rollbacks = 1;
        st.stats.peak_tc = 900;
        st.stats.stripped = 3;
        d.store.save("s", &st).unwrap();

        let req = StatusRequest {
            session_id: "s".into(),
        };
        let resp = status(&d, req).await.unwrap();
        assert!(resp.found && resp.stopped);
        assert_eq!(resp.projection_messages, 1);
        let size = raw_count("", &[msg.clone(), msg]);
        assert_eq!(resp.projection_tokens, (size as f64 * 1.5) as u64);
        assert_eq!(resp.calibration_ratio, 1.5);
        assert_eq!(
            (resp.budget_tokens, resp.configured_budget_tokens),
            (32_768, 65_536)
        );
        assert_eq!(
            (
                resp.stats.steps,
                resp.stats.edits_applied,
                resp.stats.rollbacks,
                resp.stats.peak_tc,
                resp.stats.stripped
            ),
            (7, 2, 1, 900, 3)
        );
        // The bench reads the counters under `stats` (`stats.get("stats", stats)`).
        let wire = serde_json::to_value(&resp).unwrap();
        assert_eq!(wire["stats"]["peak_tc"], 900);
        assert_eq!(wire["stats"]["edits_applied"], 2);
        assert_eq!(wire["budget_tokens"], 32_768);
    }
}
