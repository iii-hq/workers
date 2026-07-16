//! Scenario fixture loading and load-time validation. Every defect caught
//! here is a `runner_error` before the stack starts (spec § Script schema):
//! duplicate ordinals, invalid matcher/normalizer, missing or multiple
//! terminal frames, and response/frame disagreement.
//!
//! One deviation from the spec's repository sketch: `recorder.json` is folded
//! into `scenario.yaml` — `ConformanceScenarioV1` already embeds
//! `RecorderConfigV1`, so a second file would be a divergent copy.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::types::scenario::ConformanceScenarioV1;
use crate::types::script::{JsonMatcherV1, JsonNormalizerV1, NormalizerOperation, RouterScriptV1};

#[derive(Debug, Clone)]
pub struct ScenarioFixture {
    pub dir: PathBuf,
    pub scenario: ConformanceScenarioV1,
    pub script: RouterScriptV1,
    /// Template text of `expected/system-prompt.txt` (placeholders intact).
    pub system_prompt_template: String,
}

impl ScenarioFixture {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let scenario_path = dir.join("scenario.yaml");
        let scenario: ConformanceScenarioV1 = serde_yaml::from_str(
            &std::fs::read_to_string(&scenario_path)
                .with_context(|| format!("reading {}", scenario_path.display()))?,
        )
        .with_context(|| format!("parsing {}", scenario_path.display()))?;

        let script_path = dir.join(&scenario.router_script);
        let script: RouterScriptV1 = serde_json::from_str(
            &std::fs::read_to_string(&script_path)
                .with_context(|| format!("reading {}", script_path.display()))?,
        )
        .with_context(|| format!("parsing {}", script_path.display()))?;

        let prompt_path = dir.join("expected").join("system-prompt.txt");
        let raw_template = std::fs::read_to_string(&prompt_path)
            .with_context(|| format!("reading {}", prompt_path.display()))?;
        // The assembled prompt ends without a newline (the policy aid is the
        // final line); tolerate the single trailing newline editors and git
        // hooks like to append to text files.
        let system_prompt_template = raw_template
            .strip_suffix('\n')
            .unwrap_or(&raw_template)
            .to_string();

        let fixture = ScenarioFixture {
            dir: dir.to_path_buf(),
            scenario,
            script,
            system_prompt_template,
        };
        fixture.validate()?;
        Ok(fixture)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.script.scenario_id != self.scenario.id {
            anyhow::bail!(
                "script scenario_id {:?} does not match scenario id {:?}",
                self.script.scenario_id,
                self.scenario.id
            );
        }
        for declared in std::iter::once(&self.scenario.recorder.target)
            .chain(self.scenario.recorder.extra_functions.iter())
        {
            if !declared.function_id.starts_with("{{run_id}}::") {
                anyhow::bail!(
                    "recorder function {:?} must be scoped by the {{{{run_id}}}}:: prefix",
                    declared.function_id
                );
            }
        }
        for binding in &self.scenario.bindings {
            if !binding.function_id.starts_with("{{run_id}}::") {
                anyhow::bail!(
                    "scenario binding {:?} must bind a run-scoped function",
                    binding.function_id
                );
            }
        }
        validate_script(&self.script)
            .with_context(|| format!("router script for {}", self.scenario.id))?;
        Ok(())
    }
}

pub fn validate_script(script: &RouterScriptV1) -> anyhow::Result<()> {
    if script.generations.is_empty() {
        anyhow::bail!("script has no generations");
    }
    let mut seen = std::collections::BTreeSet::new();
    for generation in &script.generations {
        if !seen.insert(generation.ordinal) {
            anyhow::bail!("duplicate generation ordinal {}", generation.ordinal);
        }
        for (field, matcher) in generation.match_.fields() {
            validate_matcher(matcher)
                .with_context(|| format!("generation {} field {field}", generation.ordinal))?;
        }
        let terminal_positions: Vec<usize> = generation
            .frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.is_terminal())
            .map(|(i, _)| i)
            .collect();
        match terminal_positions.as_slice() {
            [] => anyhow::bail!("generation {} has no terminal frame", generation.ordinal),
            [last] if *last == generation.frames.len() - 1 => {}
            [_] => anyhow::bail!(
                "generation {}: terminal frame is not the last frame",
                generation.ordinal
            ),
            _ => anyhow::bail!(
                "generation {} has multiple terminal frames",
                generation.ordinal
            ),
        }
        validate_response_agreement(generation)?;
        for barrier in generation.barriers.as_deref().unwrap_or_default() {
            if barrier.before_frame >= generation.frames.len() {
                anyhow::bail!(
                    "generation {}: barrier {:?} references frame {} of {}",
                    generation.ordinal,
                    barrier.id,
                    barrier.before_frame,
                    generation.frames.len()
                );
            }
        }
    }
    Ok(())
}

fn validate_matcher(matcher: &JsonMatcherV1) -> anyhow::Result<()> {
    match matcher {
        JsonMatcherV1::Regex { pattern } => {
            regex::Regex::new(pattern)
                .map_err(|e| anyhow::anyhow!("invalid regex {pattern:?}: {e}"))?;
        }
        JsonMatcherV1::Sha256 { expected } => {
            let is_placeholder = expected.contains("{{");
            let is_hex = expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit());
            if !is_placeholder && !is_hex {
                anyhow::bail!("sha256 expected value must be 64 hex chars, got {expected:?}");
            }
        }
        JsonMatcherV1::Exact { normalize, .. } | JsonMatcherV1::Subset { normalize, .. } => {
            for n in normalize.as_deref().unwrap_or_default() {
                validate_normalizer(n)?;
            }
        }
        JsonMatcherV1::Absent | JsonMatcherV1::Present => {}
    }
    Ok(())
}

fn validate_normalizer(n: &JsonNormalizerV1) -> anyhow::Result<()> {
    crate::matcher::validate_pointer(&n.pointer)?;
    match n.operation {
        NormalizerOperation::Replace if n.replacement.is_none() => {
            anyhow::bail!(
                "replace normalizer at {:?} requires `replacement`",
                n.pointer
            )
        }
        NormalizerOperation::Delete if n.replacement.is_some() => {
            anyhow::bail!("delete normalizer at {:?} forbids `replacement`", n.pointer)
        }
        NormalizerOperation::Delete if n.pointer.is_empty() => {
            anyhow::bail!("delete normalizer cannot target the document root")
        }
        _ => Ok(()),
    }
}

/// Terminal frame and scripted response must agree (spec: "response/frame
/// disagreement" is rejected at load): `done` requires `ok:true` and a
/// matching `stop_reason`; `error` requires `ok:false` with an error shape.
fn validate_response_agreement(
    generation: &crate::types::script::ScriptedGenerationV1,
) -> anyhow::Result<()> {
    use crate::types::frames::AssistantMessageEvent as Frame;
    let terminal = generation.frames.last().expect("validated non-empty");
    match terminal {
        Frame::Done { message } => {
            if !generation.response.ok {
                anyhow::bail!(
                    "generation {}: done frame with ok:false response",
                    generation.ordinal
                );
            }
            if let Some(stop) = generation.response.stop_reason {
                if stop != message.stop_reason {
                    anyhow::bail!(
                        "generation {}: response stop_reason disagrees with done message",
                        generation.ordinal
                    );
                }
            }
        }
        Frame::Error { .. } => {
            if generation.response.ok || generation.response.error.is_none() {
                anyhow::bail!(
                    "generation {}: error frame requires ok:false and an error shape",
                    generation.ordinal
                );
            }
        }
        _ => unreachable!("terminal position validated"),
    }
    Ok(())
}

/// Resolve `--scenario <id|all>` against the checked-in scenarios directory.
pub fn scenario_dirs(scenarios_root: &Path, selector: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(scenarios_root)
        .with_context(|| format!("reading {}", scenarios_root.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.join("scenario.yaml").is_file())
        .collect();
    dirs.sort();
    if selector == "all" {
        // Quarantined scenarios (known-open defect reproductions) run only
        // by explicit id.
        let mut selected = Vec::new();
        for dir in dirs {
            let fixture = ScenarioFixture::load(&dir)?;
            if !fixture.scenario.quarantine {
                selected.push(dir);
            }
        }
        return Ok(selected);
    }
    for dir in dirs {
        let fixture = ScenarioFixture::load(&dir)?;
        if fixture.scenario.id == selector
            || dir.file_name().and_then(|n| n.to_str()) == Some(selector)
        {
            return Ok(vec![dir]);
        }
    }
    anyhow::bail!("no scenario matches selector {selector:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_script(mutate: impl FnOnce(&mut serde_json::Value)) -> serde_json::Value {
        let matcher_absent = json!({ "mode": "absent" });
        let mut match_ = serde_json::Map::new();
        for field in crate::types::script::MATCH_FIELDS {
            match_.insert(field.to_string(), matcher_absent.clone());
        }
        let mut script = json!({
            "schema_version": "1",
            "scenario_id": "T",
            "model": {
                "id": "m", "provider": "p",
                "context_window": 1000, "max_output_tokens": 100
            },
            "generations": [{
                "ordinal": 1,
                "match": match_,
                "frames": [{
                    "type": "done",
                    "message": {
                        "role": "assistant", "content": [], "stop_reason": "end",
                        "model": "m", "provider": "p", "timestamp": 1
                    }
                }],
                "response": { "ok": true, "provider": "p", "model": "m", "stop_reason": "end" }
            }]
        });
        mutate(&mut script);
        script
    }

    fn validate(value: serde_json::Value) -> anyhow::Result<()> {
        let script: RouterScriptV1 = serde_json::from_value(value)?;
        validate_script(&script)
    }

    /// Full anyhow chain (`{:#}`): plain Display shows only the outermost
    /// context, hiding the root-cause text the assertions look for.
    fn error_chain(value: serde_json::Value) -> String {
        format!("{:#}", validate(value).unwrap_err())
    }

    #[test]
    fn a_wellformed_script_validates() {
        validate(minimal_script(|_| {})).unwrap();
    }

    #[test]
    fn duplicate_ordinals_are_rejected() {
        let script = minimal_script(|s| {
            let g = s["generations"][0].clone();
            s["generations"].as_array_mut().unwrap().push(g);
        });
        assert!(error_chain(script).contains("duplicate"));
    }

    #[test]
    fn missing_terminal_frame_is_rejected() {
        let script = minimal_script(|s| {
            s["generations"][0]["frames"] = json!([{ "type": "ping" }]);
        });
        assert!(error_chain(script).contains("no terminal"));
    }

    #[test]
    fn terminal_frame_must_be_last() {
        let script = minimal_script(|s| {
            let done = s["generations"][0]["frames"][0].clone();
            s["generations"][0]["frames"] = json!([done, { "type": "ping" }]);
        });
        assert!(error_chain(script).contains("not the last"));
    }

    #[test]
    fn response_frame_disagreement_is_rejected() {
        let script = minimal_script(|s| {
            s["generations"][0]["response"]["ok"] = json!(false);
        });
        assert!(error_chain(script).contains("ok:false"));

        let script = minimal_script(|s| {
            s["generations"][0]["response"]["stop_reason"] = json!("length");
        });
        assert!(error_chain(script).contains("disagrees"));
    }

    #[test]
    fn invalid_matchers_and_normalizers_are_rejected() {
        let script = minimal_script(|s| {
            s["generations"][0]["match"]["model"] = json!({ "mode": "regex", "pattern": "(" });
        });
        assert!(validate(script).is_err());

        let script = minimal_script(|s| {
            s["generations"][0]["match"]["messages"] = json!({
                "mode": "exact", "expected": [],
                "normalize": [{ "pointer": "/0/x", "operation": "replace" }]
            });
        });
        assert!(error_chain(script).contains("replacement"));

        let script = minimal_script(|s| {
            s["generations"][0]["match"]["messages"] = json!({
                "mode": "exact", "expected": [],
                "normalize": [{ "pointer": "no-slash", "operation": "delete" }]
            });
        });
        assert!(validate(script).is_err());
    }

    #[test]
    fn unknown_fields_and_bad_sha256_are_rejected_by_schema() {
        let script = minimal_script(|s| {
            s["generations"][0]["surprise"] = json!(true);
        });
        assert!(validate(script).is_err());

        let script = minimal_script(|s| {
            s["generations"][0]["match"]["system_prompt"] =
                json!({ "mode": "sha256", "expected": "nothex" });
        });
        assert!(error_chain(script).contains("64 hex"));
    }

    #[test]
    fn barrier_out_of_range_is_rejected() {
        let script = minimal_script(|s| {
            s["generations"][0]["barriers"] =
                json!([{ "before_frame": 5, "id": "b", "timeout_ms": 100 }]);
        });
        assert!(error_chain(script).contains("barrier"));
    }
}
