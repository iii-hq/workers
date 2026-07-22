use super::loading::ScenarioFixture;

/// Resolve `--scenario <id|slug|all>` against the checked-in fixtures.
pub fn scenario_fixtures(selector: &str) -> anyhow::Result<Vec<ScenarioFixture>> {
    select_fixtures(crate::scenarios::all(), selector)
}

pub(crate) fn select_fixtures(
    fixtures: Vec<ScenarioFixture>,
    selector: &str,
) -> anyhow::Result<Vec<ScenarioFixture>> {
    reject_duplicate_identities(&fixtures)?;

    if selector == "all" {
        anyhow::ensure!(!fixtures.is_empty(), "selector \"all\" matched no scenario");
        for fixture in &fixtures {
            fixture.validate()?;
        }
        return Ok(fixtures);
    }

    if let Some(fixture) = fixtures
        .into_iter()
        .find(|fixture| fixture.slug == selector || fixture.scenario.id == selector)
    {
        fixture.validate()?;
        return Ok(vec![fixture]);
    }
    anyhow::bail!("no scenario matches selector {selector:?}")
}

fn reject_duplicate_identities(fixtures: &[ScenarioFixture]) -> anyhow::Result<()> {
    let mut slugs = std::collections::BTreeMap::new();
    let mut ids = std::collections::BTreeMap::new();
    for fixture in fixtures {
        if let Some(previous) = slugs.insert(fixture.slug.clone(), fixture.scenario.id.clone()) {
            anyhow::bail!(
                "duplicate scenario slug {:?} (ids {:?} and {:?})",
                fixture.slug,
                previous,
                fixture.scenario.id
            );
        }
        if let Some(previous) = ids.insert(fixture.scenario.id.clone(), fixture.slug.clone()) {
            anyhow::bail!(
                "duplicate scenario id {:?} in {:?} and {:?}",
                fixture.scenario.id,
                previous,
                fixture.slug
            );
        }
    }
    Ok(())
}
