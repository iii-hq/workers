//! The worker-wide `context-clm` configuration entry. Every field except
//! `data_dir` is read per assemble call from the live snapshot, so it
//! hot-reloads; `data_dir` is read once at boot.
use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONFIG_ID: &str = "context-clm";
pub const CONFIG_NAME: &str = "Context CLM";
pub const CONFIG_DESCRIPTION: &str =
    "Model-managed context: token budget and reserve, nudge thresholds, rollback limits, the edit gate, observation caps, thinking strip, step log and the state folder.";
pub const DEFAULT_DATA_DIR: &str = "data/context-clm";

/// When the urgent "compact now" nudge fires.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PersistentNudgeWire", into = "PersistentNudgeWire")]
pub enum PersistentNudge {
    /// When the room left can no longer hold the next observation.
    Adaptive,
    /// When the context reaches this fraction (0 < r <= 1) of the limit.
    Ratio(f64),
    Off,
}

/// Serde and schema proxy for [`PersistentNudge`]: `"adaptive"`, `"off"` or a number.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PersistentNudgeWire {
    Mode(NudgeMode),
    Ratio(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NudgeMode {
    Adaptive,
    Off,
}

impl TryFrom<PersistentNudgeWire> for PersistentNudge {
    type Error = String;
    fn try_from(wire: PersistentNudgeWire) -> Result<Self, String> {
        match wire {
            PersistentNudgeWire::Mode(NudgeMode::Adaptive) => Ok(Self::Adaptive),
            PersistentNudgeWire::Mode(NudgeMode::Off) => Ok(Self::Off),
            PersistentNudgeWire::Ratio(r) if r > 0.0 && r <= 1.0 => Ok(Self::Ratio(r)),
            PersistentNudgeWire::Ratio(r) => {
                Err(format!("persistent_nudge ratio must be in (0, 1], got {r}"))
            }
        }
    }
}

impl From<PersistentNudge> for PersistentNudgeWire {
    fn from(nudge: PersistentNudge) -> Self {
        match nudge {
            PersistentNudge::Adaptive => Self::Mode(NudgeMode::Adaptive),
            PersistentNudge::Off => Self::Mode(NudgeMode::Off),
            PersistentNudge::Ratio(r) => Self::Ratio(r),
        }
    }
}

impl JsonSchema for PersistentNudge {
    fn schema_name() -> String {
        "PersistentNudge".into()
    }
    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        PersistentNudgeWire::json_schema(gen)
    }
}

/// What the edit gate rejects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EditGate {
    /// Reject an edit whose result is over the limit and bigger than before.
    Fit,
    /// Reject any edit that grows the context.
    Shrink,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ClmConfig {
    /// Context budget in tokens. A session's `context_budget_tokens`
    /// metadata overrides it.
    pub budget_tokens: u64,
    /// Tokens kept free under the budget; the limit is budget - reserve.
    pub reserve_tokens: u64,
    /// Fractions of the budget (each in (0, 1)) that trigger a one-time nudge.
    pub nudge_ratios: Vec<f64>,
    /// "adaptive", "off", or a fraction of the limit in (0, 1].
    pub persistent_nudge: PersistentNudge,
    /// Rollbacks allowed before the run gets its final turn.
    pub max_rollbacks: u32,
    /// Tokens a rollback frees below the limit.
    pub rollback_margin_tokens: u64,
    pub edit_gate: EditGate,
    /// Head/tail cap, in characters, on each tool output in the projection.
    pub observation_max_chars: usize,
    /// Strip thinking from the first changed message on (preserved-thinking safety).
    pub strip_thinking: bool,
    /// Append one JSON line per step to `<data_dir>/log/<session>.jsonl`.
    pub step_log: bool,
    /// Folder for per-session state and step logs. Relative paths resolve
    /// against the Compose project directory (III_COMPOSE_DIR). Read once at
    /// boot; a change takes effect after a restart.
    pub data_dir: String,
}

impl Default for ClmConfig {
    fn default() -> Self {
        Self {
            budget_tokens: 65_536,
            reserve_tokens: 2_048,
            nudge_ratios: vec![0.25, 0.5, 0.75],
            persistent_nudge: PersistentNudge::Adaptive,
            max_rollbacks: 50,
            rollback_margin_tokens: 2_048,
            edit_gate: EditGate::Fit,
            observation_max_chars: 10_000,
            strip_thinking: true,
            step_log: true,
            data_dir: iii_worker_paths::default_path(DEFAULT_DATA_DIR),
        }
    }
}

impl ClmConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.budget_tokens <= self.reserve_tokens {
            return Err("budget_tokens must be greater than reserve_tokens".into());
        }
        if let Some(r) = self.nudge_ratios.iter().find(|r| !(**r > 0.0 && **r < 1.0)) {
            return Err(format!("nudge_ratios entries must be in (0, 1), got {r}"));
        }
        if self.observation_max_chars == 0 {
            return Err("observation_max_chars must be at least 1".into());
        }
        Ok(())
    }

    /// Parse and validate a stored value. A blank `data_dir` falls back to
    /// the default; any other bad value is an error (the previous snapshot
    /// stays live).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let mut config: Self = serde_json::from_value(value.clone())
            .map_err(|e| format!("context-clm configuration does not match its schema: {e}"))?;
        if config.data_dir.trim().is_empty() {
            config.data_dir = Self::default().data_dir;
        }
        config.validate()?;
        Ok(config)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("context-clm config serializes")
    }

    /// The entry schema, with the shipped defaults attached as `example`.
    pub fn json_schema() -> Value {
        let mut schema = serde_json::to_value(schemars::schema_for!(Self))
            .expect("context-clm schema serializes");
        schema["example"] = Self::default().to_json();
        schema
    }
}
