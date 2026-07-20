use std::path::Path;

use super::loading::ScenarioFixture;
use crate::scenarios::RegisteredScenario;

/// Resolve `--scenario <id|slug|all>` against the code registry and return
/// already-compiled fixtures.
///
/// `include_quarantined` is intended for validation. Explicit id/slug
/// selection always includes the requested fixture; for `all`, normal runs
/// exclude quarantines while validation includes them.
pub fn scenario_fixtures(
    scenarios_root: &Path,
    selector: &str,
    include_quarantined: bool,
) -> anyhow::Result<Vec<ScenarioFixture>> {
    select_fixtures(
        crate::scenarios::all(),
        scenarios_root,
        selector,
        include_quarantined,
    )
}

/// Registry-parameterized core so selection semantics stay testable without
/// the checked-in scenario set.
pub(crate) fn select_fixtures(
    registered: Vec<RegisteredScenario>,
    scenarios_root: &Path,
    selector: &str,
    include_quarantined: bool,
) -> anyhow::Result<Vec<ScenarioFixture>> {
    reject_duplicate_identities(&registered)?;

    if selector == "all" {
        let mut selected = Vec::new();
        for entry in &registered {
            if include_quarantined || !entry.authored.quarantine {
                selected.push(ScenarioFixture::from_registered(entry, scenarios_root)?);
            }
        }
        if selected.is_empty() {
            let qualifier = if include_quarantined {
                ""
            } else {
                " non-quarantined"
            };
            anyhow::bail!("selector \"all\" matched no{qualifier} registered scenario");
        }
        return Ok(selected);
    }

    // Slug and id selection compile only the requested scenario, so an
    // unrelated fixture that fails compilation cannot block it. Malformed
    // unrelated fixtures remain the responsibility of `validate --scenario
    // all`.
    if let Some(entry) = registered.iter().find(|entry| entry.slug == selector) {
        return Ok(vec![ScenarioFixture::from_registered(
            entry,
            scenarios_root,
        )?]);
    }
    if let Some(entry) = registered
        .iter()
        .find(|entry| entry.authored.id == selector)
    {
        return Ok(vec![ScenarioFixture::from_registered(
            entry,
            scenarios_root,
        )?]);
    }
    anyhow::bail!("no scenario matches selector {selector:?}")
}

/// Identity checks read only authored data, never compile, so they cannot be
/// masked by an unrelated compilation failure.
fn reject_duplicate_identities(registered: &[RegisteredScenario]) -> anyhow::Result<()> {
    let mut slugs = std::collections::BTreeMap::new();
    let mut ids = std::collections::BTreeMap::new();
    for entry in registered {
        if let Some(previous) = slugs.insert(entry.slug.clone(), entry.authored.id.clone()) {
            anyhow::bail!(
                "duplicate scenario slug {:?} (ids {:?} and {:?})",
                entry.slug,
                previous,
                entry.authored.id
            );
        }
        if let Some(previous) = ids.insert(entry.authored.id.clone(), entry.slug.clone()) {
            anyhow::bail!(
                "duplicate scenario id {:?} in {:?} and {:?}",
                entry.authored.id,
                previous,
                entry.slug
            );
        }
    }
    Ok(())
}
