use security_scan::{
    AnalysisConfigV1, RepositoryConfigV1, RepositoryGitHubConfigV1, RepositoryScheduleV1,
    ScanModeV1, SecurityScanError, WorkerConfig,
};

fn valid_config() -> WorkerConfig {
    WorkerConfig {
        repositories: vec![RepositoryConfigV1 {
            id: "iii-hq/iii".into(),
            path: "/srv/repos/iii".into(),
            github: None,
            schedule: Some(RepositoryScheduleV1 {
                expression: "0 0 3 * * *".into(),
                target_ref: "refs/heads/main".into(),
                mode: ScanModeV1::Scan,
            }),
        }],
        analysis: AnalysisConfigV1 {
            model: "security-review-model".into(),
            provider: None,
            max_turns: 4,
            max_output_tokens: 8_000,
            max_total_tokens: 50_000,
            max_cost_usd: Some(2.0),
        },
    }
}

#[test]
fn config_fails_closed_without_an_operator_model() {
    let mut config = valid_config();
    config.analysis.model.clear();

    let error = config.validate().unwrap_err();
    assert!(matches!(error, SecurityScanError::InvalidRequest(_)));
}

#[test]
fn config_rejects_duplicate_repository_schedule_ids_and_relative_paths() {
    let mut duplicate = valid_config();
    duplicate
        .repositories
        .push(duplicate.repositories[0].clone());
    assert!(duplicate.validate().is_err());

    let mut relative = valid_config();
    relative.repositories[0].path = "repos/iii".into();
    assert!(relative.validate().is_err());
}

#[test]
fn valid_operator_config_is_accepted() {
    valid_config().validate().unwrap();
}

#[test]
fn empty_registry_defaults_boot_in_an_idle_fail_closed_state() {
    let mut config = valid_config();
    config.repositories.clear();
    config.analysis.model.clear();

    config.validate().unwrap();
}

#[test]
fn old_repository_config_without_a_schedule_remains_compatible() {
    let repository: RepositoryConfigV1 = serde_json::from_value(serde_json::json!({
        "id": "iii-hq/iii",
        "path": "/srv/repos/iii"
    }))
    .unwrap();

    assert!(repository.github.is_none());
    assert!(repository.schedule.is_none());
}

#[test]
fn github_mapping_is_optional_and_requires_an_explicit_full_name() {
    let mut config = valid_config();
    config.repositories[0].github = Some(RepositoryGitHubConfigV1 {
        full_name: "iii-hq/iii".into(),
    });
    config.validate().unwrap();

    for full_name in [
        "",
        "iii-hq",
        "iii-hq/iii/extra",
        "/iii",
        "iii-hq/",
        "iii hq/iii",
        "iii-hq/../iii",
    ] {
        config.repositories[0].github = Some(RepositoryGitHubConfigV1 {
            full_name: full_name.into(),
        });
        assert!(config.validate().is_err(), "accepted {full_name:?}");
    }
}

#[test]
fn schedule_accepts_six_or_seven_fields_and_rejects_invalid_expressions() {
    for expression in ["0 0 3 * * *", "0 0 3 * * * 2027"] {
        let mut config = valid_config();
        config.repositories[0].schedule.as_mut().unwrap().expression = expression.into();
        config.validate().unwrap();
    }

    for expression in ["0 3 * * *", "0 0 25 * * *", " 0 0 3 * * *"] {
        let mut config = valid_config();
        config.repositories[0].schedule.as_mut().unwrap().expression = expression.into();
        assert!(config.validate().is_err(), "accepted {expression:?}");
    }
}

#[test]
fn schedule_rejects_revision_syntax_and_argv_shaped_refs() {
    for target_ref in [
        "",
        "@",
        "--help",
        "refs/heads/main^{tree}",
        "refs/heads/main~1",
        "refs/heads/main..next",
        "refs/heads/main lock",
        "refs/heads/main\u{2003}lock",
        "refs/heads/.hidden",
        "refs/heads/main.lock",
    ] {
        let mut config = valid_config();
        config.repositories[0].schedule.as_mut().unwrap().target_ref = target_ref.into();
        assert!(config.validate().is_err(), "accepted {target_ref:?}");
    }
}
