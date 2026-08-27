//! The single shipped identity prompt must describe the surface the harness
//! actually serves.
//!
//! Every agent — top-level and spawned children alike — sees
//! `harness/prompts/default.txt`: the basic engine functions and the
//! discovery loop. Roles differ by policy and enrich layers, never by a
//! separate embedded identity.

use std::path::{Path, PathBuf};

/// Ids and concepts the harness no longer serves. A prompt naming one sends
/// every agent into a registration error.
const REMOVED: &[&str] = &[
    "harness::react",
    "harness::notify_agent",
    "harness::trigger-call",
];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/harness.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness/ has a parent")
        .to_path_buf()
}

fn shipped_prompt() -> PathBuf {
    repo_root().join("harness/prompts/default.txt")
}

fn label(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn harness_owns_the_only_shipped_prompt() {
    assert!(shipped_prompt().is_file());

    // The separate sub-agent identity is retired: one prompt for every agent.
    let retired = repo_root().join("harness/prompts/subagent.txt");
    assert!(
        !retired.exists(),
        "the sub-agent prompt must stay removed: {}",
        label(&retired)
    );

    let provider_prompts: Vec<PathBuf> = std::fs::read_dir(repo_root())
        .expect("repo root is readable")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("provider-"))
        .map(|entry| entry.path().join("prompts/identity.txt"))
        .filter(|path| path.is_file())
        .collect();
    assert!(
        provider_prompts.is_empty(),
        "provider identity prompts must stay removed: {:?}",
        provider_prompts
            .iter()
            .map(|path| label(path))
            .collect::<Vec<_>>()
    );
}

#[test]
fn shipped_prompt_teaches_no_removed_id() {
    let body = std::fs::read_to_string(shipped_prompt()).expect("prompt is readable");
    for gone in REMOVED {
        assert!(
            !body.contains(gone),
            "prompt teaches a surface the harness no longer serves: {gone}"
        );
    }
}

/// The identity is minimal by design: the engine's own surface plus
/// `directory::search_functions` — the default discovery path, served by the
/// directory worker that installs alongside the harness (engine discovery is
/// the fallback). Naming any other worker preempts discovery; teaching an
/// orchestration process re-imports doctrine that lives in contracts, hooks,
/// and skills.
#[test]
fn shipped_prompt_teaches_discovery_and_nothing_else() {
    let body = std::fs::read_to_string(shipped_prompt()).expect("prompt is readable");
    assert!(body.contains("directory::search_functions"));
    assert!(body.contains("engine::functions::list"));
    assert!(body.contains("engine::functions::info"));
    assert!(body.contains("engine::register_trigger"));

    for named in [
        "fp::",
        "coder::",
        "compose::",
        "directory::registry",
        "directory::agents",
        "harness::spawn",
    ] {
        assert!(
            !body.contains(named),
            "prompt names a surface agents are meant to discover: {named}"
        );
    }
}

/// Phrases that prescribe an orchestration PROCESS — a fixed ordering, a
/// topology recipe, a mandated shared-state flow. Orchestration guidance is
/// opt-in (harness/skills/orchestration.md, or the task prompt), never baked
/// into the identity.
const PRESCRIBED_PROCESS: &[&str] = &[
    "wire, spawn, stop",
    "wire → spawn → stop",
    "wire the consumers",
    "wire consumers first",
    "delegation is one-way",
    "results flow back only through",
    "results come back only through",
    "fan-out",
    "fan-in",
    "react trigger",
    "reaction graph",
    r#"function_id: "harness::spawn""#,
];

#[test]
fn shipped_prompt_prescribes_no_orchestration_process() {
    let body = std::fs::read_to_string(shipped_prompt())
        .expect("prompt is readable")
        .to_lowercase();
    for phrase in PRESCRIBED_PROCESS {
        assert!(
            !body.contains(&phrase.to_lowercase()),
            "prompt prescribes an orchestration process: {phrase:?}"
        );
    }
}
