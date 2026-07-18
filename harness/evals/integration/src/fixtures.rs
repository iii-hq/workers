//! Single-file scenario loading, compilation, selection, and load-time
//! validation. Every authoring defect is reported before the stack starts.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::expand::{compile_scenario, CompiledFixtureV1};
use crate::types::scenario::{CompiledScenarioV1, IntegrationScenarioV1};
use crate::types::script::{JsonMatcherV1, JsonNormalizerV1, NormalizerOperation, RouterScriptV1};

#[derive(Debug, Clone)]
pub struct ScenarioFixture {
    pub dir: PathBuf,
    pub authored: IntegrationScenarioV1,
    pub scenario: CompiledScenarioV1,
    pub script: RouterScriptV1,
    /// Compiled shared golden plus inferred session/policy aid.
    pub system_prompt_template: String,
}

impl ScenarioFixture {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let scenario_path = dir.join("scenario.yaml");
        let scenario: IntegrationScenarioV1 = serde_yaml::from_str(
            &std::fs::read_to_string(&scenario_path)
                .with_context(|| format!("reading {}", scenario_path.display()))?,
        )
        .with_context(|| format!("parsing {}", scenario_path.display()))?;

        let scenarios_root = dir.parent().with_context(|| {
            format!("scenario directory {} has no scenarios root", dir.display())
        })?;
        let prompt_path = scenarios_root.join("system-prompt.txt");
        let system_prompt_base = std::fs::read_to_string(&prompt_path)
            .with_context(|| format!("reading {}", prompt_path.display()))?;
        let CompiledFixtureV1 {
            scenario: compiled,
            script,
            system_prompt_template,
        } = compile_scenario(&scenario, &system_prompt_base)
            .with_context(|| format!("compiling {}", scenario_path.display()))?;

        let fixture = ScenarioFixture {
            dir: dir.to_path_buf(),
            authored: scenario,
            scenario: compiled,
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

    pub fn compiled(&self) -> CompiledFixtureV1 {
        CompiledFixtureV1 {
            scenario: self.scenario.clone(),
            script: self.script.clone(),
            system_prompt_template: self.system_prompt_template.clone(),
        }
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

/// Resolve `--scenario <id|slug|all>` and return already-loaded fixtures.
///
/// `include_quarantined` is intended for validation. Explicit id/slug
/// selection always includes the requested fixture; for `all`, normal runs
/// exclude quarantines while validation includes them.
pub fn scenario_fixtures(
    scenarios_root: &Path,
    selector: &str,
    include_quarantined: bool,
) -> anyhow::Result<Vec<ScenarioFixture>> {
    let dirs = scenario_directories(scenarios_root)?;
    if selector == "all" {
        let mut selected = Vec::new();
        let mut ids = std::collections::BTreeMap::new();
        for dir in dirs {
            let fixture = ScenarioFixture::load(&dir)?;
            if let Some(previous) = ids.insert(fixture.scenario.id.clone(), dir.clone()) {
                anyhow::bail!(
                    "duplicate scenario id {:?} in {} and {}",
                    fixture.scenario.id,
                    previous.display(),
                    dir.display()
                );
            }
            if include_quarantined || !fixture.scenario.quarantine {
                selected.push(fixture);
            }
        }
        if selected.is_empty() {
            let qualifier = if include_quarantined {
                ""
            } else {
                " non-quarantined"
            };
            anyhow::bail!(
                "selector \"all\" matched no{qualifier} scenario under {}",
                scenarios_root.display()
            );
        }
        return Ok(selected);
    }

    // A slug is an exact directory lookup and must not be blocked by an
    // unrelated invalid fixture.
    if let Some(dir) = dirs
        .iter()
        .find(|dir| dir.file_name().and_then(|name| name.to_str()) == Some(selector))
    {
        let fixture = ScenarioFixture::load(dir)?;
        reject_duplicate_selected_id(&dirs, dir, &fixture.scenario.id)?;
        return Ok(vec![fixture]);
    }

    // ID lookup reads only identities first, then compiles the one selected
    // fixture. Malformed unrelated fixtures remain the responsibility of
    // `validate --scenario all`.
    let matching: Vec<&PathBuf> = dirs
        .iter()
        .filter(|dir| {
            read_scenario_id(dir)
                .map(|id| id == selector)
                .unwrap_or(false)
        })
        .collect();
    match matching.as_slice() {
        [dir] => return Ok(vec![ScenarioFixture::load(dir)?]),
        [] => {}
        duplicates => {
            anyhow::bail!(
                "scenario id {selector:?} is duplicated in {}",
                duplicates
                    .iter()
                    .map(|dir| dir.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    anyhow::bail!("no scenario matches selector {selector:?}")
}

/// Refuse `init` when an authored scenario already owns the requested id.
pub fn ensure_scenario_id_available(
    scenarios_root: &Path,
    scenario_id: &str,
) -> anyhow::Result<()> {
    crate::types::scenario::validate_scenario_id(scenario_id)?;
    for dir in scenario_directories(scenarios_root)? {
        if read_scenario_id(&dir)? == scenario_id {
            anyhow::bail!(
                "scenario id {scenario_id:?} already exists in {}",
                dir.display()
            );
        }
    }
    Ok(())
}

fn scenario_directories(scenarios_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(scenarios_root)
        .with_context(|| format!("reading {}", scenarios_root.display()))?
    {
        let entry = entry
            .with_context(|| format!("reading an entry under {}", scenarios_root.display()))?;
        let path = entry.path();
        if path.join("scenario.yaml").is_file() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

#[derive(serde::Deserialize)]
struct ScenarioIdentity {
    id: String,
}

fn read_scenario_id(dir: &Path) -> anyhow::Result<String> {
    let path = dir.join("scenario.yaml");
    let source = std::fs::read_to_string(&path)
        .with_context(|| format!("reading scenario identity from {}", path.display()))?;
    let identity: ScenarioIdentity = serde_yaml::from_str(&source)
        .with_context(|| format!("parsing scenario identity from {}", path.display()))?;
    Ok(identity.id)
}

fn reject_duplicate_selected_id(
    dirs: &[PathBuf],
    selected_dir: &Path,
    selected_id: &str,
) -> anyhow::Result<()> {
    let duplicates = dirs
        .iter()
        .filter(|dir| dir.as_path() != selected_dir)
        .filter(|dir| {
            read_scenario_id(dir)
                .map(|id| id == selected_id)
                .unwrap_or(false)
        })
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>();
    if !duplicates.is_empty() {
        anyhow::bail!(
            "scenario id {selected_id:?} from {} is duplicated in {}",
            selected_dir.display(),
            duplicates.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_script(mutate: impl FnOnce(&mut serde_json::Value)) -> serde_json::Value {
        let match_ = serde_json::to_value(crate::types::script::GenerationMatchV1::uniform(
            JsonMatcherV1::Absent,
        ))
        .unwrap();
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
    fn removed_barriers_are_rejected_as_unknown_fields() {
        let script = minimal_script(|s| {
            s["generations"][0]["barriers"] =
                json!([{ "before_frame": 5, "id": "b", "timeout_ms": 100 }]);
        });
        assert!(error_chain(script).contains("barrier"));
    }

    /// Both delta wire forms are valid fixtures: the slim delta (no
    /// `partial`) the router emits today, and the legacy fat delta carrying
    /// a full snapshot.
    #[test]
    fn slim_and_fat_deltas_both_validate() {
        let slim = minimal_script(|s| {
            let done = s["generations"][0]["frames"][0].clone();
            s["generations"][0]["frames"] = json!([{ "type": "text_delta", "delta": "x" }, done]);
        });
        validate(slim).unwrap();

        let fat = minimal_script(|s| {
            let done = s["generations"][0]["frames"][0].clone();
            let partial = done["message"].clone();
            s["generations"][0]["frames"] =
                json!([{ "type": "text_delta", "partial": partial, "delta": "x" }, done]);
        });
        validate(fat).unwrap();
    }

    #[test]
    fn all_selector_with_no_runnable_scenario_is_an_error() {
        let empty = tempfile::tempdir().unwrap();
        let err = scenario_fixtures(empty.path(), "all", false).unwrap_err();
        assert!(format!("{err:#}").contains("no non-quarantined scenario"));
    }

    fn write_text_scenario(root: &Path, slug: &str, id: &str) {
        let dir = root.join(slug);
        std::fs::create_dir(&dir).unwrap();
        let authored = crate::expand::scenario_template(
            id,
            "A fixture selection test.",
            crate::expand::ScenarioTemplateKind::Text,
        );
        std::fs::write(
            dir.join("scenario.yaml"),
            crate::expand::render_authored_yaml(&authored).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn explicit_selection_isolated_from_unrelated_invalid_fixtures() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("system-prompt.txt"), "base").unwrap();
        let invalid = root.path().join("a-invalid");
        std::fs::create_dir(&invalid).unwrap();
        std::fs::write(invalid.join("scenario.yaml"), "not: [valid").unwrap();
        write_text_scenario(root.path(), "z-selected", "C-E2E-SELECTED");

        assert_eq!(
            scenario_fixtures(root.path(), "z-selected", false).unwrap()[0]
                .scenario
                .id,
            "C-E2E-SELECTED"
        );
        assert_eq!(
            scenario_fixtures(root.path(), "C-E2E-SELECTED", false).unwrap()[0]
                .scenario
                .id,
            "C-E2E-SELECTED"
        );
        assert!(scenario_fixtures(root.path(), "all", true).is_err());
    }

    #[test]
    fn duplicate_ids_are_rejected_and_unavailable_to_init() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("system-prompt.txt"), "base").unwrap();
        write_text_scenario(root.path(), "first", "C-E2E-DUPLICATE");
        write_text_scenario(root.path(), "second", "C-E2E-DUPLICATE");

        let all_error = scenario_fixtures(root.path(), "all", true).unwrap_err();
        assert!(format!("{all_error:#}").contains("duplicate scenario id"));
        let id_error = scenario_fixtures(root.path(), "C-E2E-DUPLICATE", true).unwrap_err();
        assert!(format!("{id_error:#}").contains("duplicated"));
        let slug_error = scenario_fixtures(root.path(), "first", true).unwrap_err();
        assert!(format!("{slug_error:#}").contains("duplicated"));
        let init_error = ensure_scenario_id_available(root.path(), "C-E2E-DUPLICATE").unwrap_err();
        assert!(format!("{init_error:#}").contains("already exists"));
    }

    #[test]
    fn checked_in_scenarios_are_single_file_and_compile() {
        fn files_under(dir: &Path, files: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    files_under(&path, files);
                } else {
                    files.push(path);
                }
            }
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
        let fixtures = scenario_fixtures(&root, "all", true).unwrap();
        assert_eq!(fixtures.len(), 5);
        assert_eq!(
            fixtures
                .iter()
                .filter(|fixture| fixture.scenario.quarantine)
                .count(),
            3
        );
        for fixture in fixtures {
            let mut files = Vec::new();
            files_under(&fixture.dir, &mut files);
            assert_eq!(
                files,
                vec![fixture.dir.join("scenario.yaml")],
                "{} must contain only scenario.yaml",
                fixture.dir.display()
            );
        }
    }

    #[test]
    fn all_selection_can_include_quarantines_for_validation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
        assert_eq!(scenario_fixtures(&root, "all", false).unwrap().len(), 2);
        assert_eq!(scenario_fixtures(&root, "all", true).unwrap().len(), 5);
        assert!(
            scenario_fixtures(&root, "crash-recovery-507", false).unwrap()[0]
                .scenario
                .quarantine
        );
    }
}
