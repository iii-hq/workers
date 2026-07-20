use std::path::{Path, PathBuf};

use anyhow::Context;

use super::loading::ScenarioFixture;

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
