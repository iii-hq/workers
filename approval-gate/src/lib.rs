//! Approval gate. Subscribes to `agent::before_tool_call` and blocks calls
//! whose `tool_call.name` appears in the run's `approval_required` list,
//! waiting for the UI to call `approval::resolve` (or for a timeout).

use iii_sdk::{FunctionRef, III};

pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const STATE_SCOPE: &str = "approvals";
pub const DEFAULT_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone)]
pub struct Config {
    pub topic: String,
    pub timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            topic: "agent::before_tool_call".into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(t) = std::env::var("APPROVAL_GATE_TIMEOUT_MS") {
            if let Ok(n) = t.parse() {
                cfg.timeout_ms = n;
            }
        }
        cfg
    }
}

pub struct Refs {
    pub resolve: FunctionRef,
    pub list_pending: FunctionRef,
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
}

pub fn register(_iii: &III, _config: Config) -> anyhow::Result<Refs> {
    anyhow::bail!("not yet implemented")
}
