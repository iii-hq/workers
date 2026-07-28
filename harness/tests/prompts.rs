//! Every shipped identity prompt must describe the surface the harness
//! actually serves.
//!
//! The prompt an agent sees is the PROVIDER's (`router::system_prompt::get`);
//! `harness/prompts/default.txt` is only the fallback when a provider declares
//! none or the operator switches provider prompts off. When `harness::react`
//! was removed, the fallback was rewritten and all eight provider prompts were
//! not — so every agent kept being told to bind a function that no longer
//! existed. A test that reads only the harness's own copy cannot catch that,
//! so this one walks the whole repo.

use std::path::{Path, PathBuf};

/// Ids and concepts the harness no longer serves. A prompt naming one sends
/// every agent into a registration error.
const REMOVED: &[&str] = &[
    "harness::react",
    "harness::notify_agent",
    "harness::trigger-call",
];

/// Prompts must not name a worker the agent is meant to DISCOVER: naming one
/// preempts discovery and skews any evaluation of whether the agent finds it.
/// (`fp` advertises itself through its own presence-gated guidance hook.)
const UNDISCOVERABLE: &[&str] = &["fp::"];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/harness.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness/ has a parent")
        .to_path_buf()
}

/// Every `*/prompts/*.txt` in the repo — the harness's own two plus each
/// provider's identity prompt.
fn shipped_prompts() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let root = repo_root();
    let Ok(workers) = std::fs::read_dir(&root) else {
        return found;
    };
    for worker in workers.flatten() {
        let prompts = worker.path().join("prompts");
        if !prompts.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&prompts) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "txt") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn label(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn the_sweep_actually_finds_the_shipped_prompts() {
    // A silent zero-file walk would make every assertion below vacuous.
    let prompts = shipped_prompts();
    assert!(
        prompts.len() >= 8,
        "expected the harness prompts plus one identity prompt per provider, found {}: {:?}",
        prompts.len(),
        prompts.iter().map(|p| label(p)).collect::<Vec<_>>()
    );
    assert!(prompts
        .iter()
        .any(|p| label(p).ends_with("harness/prompts/default.txt")));
    assert!(prompts
        .iter()
        .any(|p| label(p).contains("provider-anthropic")));
}

#[test]
fn no_shipped_prompt_teaches_a_removed_id() {
    let mut stale = Vec::new();
    for path in shipped_prompts() {
        let body = std::fs::read_to_string(&path).expect("prompt is readable");
        for gone in REMOVED {
            if body.contains(gone) {
                stale.push(format!("{} names `{gone}`", label(&path)));
            }
        }
    }
    assert!(
        stale.is_empty(),
        "prompts teach a surface the harness no longer serves:\n  {}",
        stale.join("\n  ")
    );
}

#[test]
fn no_shipped_prompt_preempts_worker_discovery() {
    let mut named = Vec::new();
    for path in shipped_prompts() {
        let body = std::fs::read_to_string(&path).expect("prompt is readable");
        for worker in UNDISCOVERABLE {
            if body.contains(worker) {
                named.push(format!("{} names `{worker}`", label(&path)));
            }
        }
    }
    assert!(
        named.is_empty(),
        "prompts name a worker agents are meant to discover:\n  {}",
        named.join("\n  ")
    );
}

/// Identity prompts only: the file a top-level agent is booted with. Other
/// workers ship prompts for their own purposes (a Slack channel preamble, a
/// summarizer) and owe the binding contract nothing.
fn is_identity_prompt(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("identity.txt") | Some("default.txt")
    )
}

#[test]
fn every_identity_prompt_teaches_the_live_surface() {
    // Tool guidance, not process: a prompt must still NAME the callback
    // primitive, the spawn function, and the leaf-escape option — those are
    // the API surface — while prescribing no topology (the sweep below owns
    // that half).
    for path in shipped_prompts()
        .into_iter()
        .filter(|p| is_identity_prompt(p))
    {
        let name = label(&path);
        let body = std::fs::read_to_string(&path).expect("prompt is readable");
        assert!(
            body.contains("engine::register_trigger"),
            "{name} never mentions the callback primitive"
        );
        assert!(
            body.contains("harness::spawn"),
            "{name} never mentions how to start a sub-agent"
        );
        assert!(
            body.contains("orchestrator: true"),
            "{name} never mentions the leaf default's escape hatch"
        );
        assert!(
            !body.contains(r#"function_id: "harness::spawn""#),
            "{name} still teaches spawn as a binding target"
        );
        // `condition_function_id` reaches the engine's own contract, where an
        // unknown or erroring condition skips every fire silently and forever.
        // A prompt may name it — but only to steer agents off it.
        if body.contains("condition_function_id") {
            assert!(
                body.contains("refused") || body.contains("Do NOT") || body.contains("do NOT"),
                "{name} names `condition_function_id` without warning against it"
            );
        }
    }
}

/// Phrases that prescribe an orchestration PROCESS — a fixed ordering, a
/// topology recipe, a mandated shared-state flow. The runtime no longer has a
/// trigger→spawn path, and the prompts must not smuggle its doctrine back in:
/// orchestration guidance is opt-in (harness/skills/orchestration.md, or the
/// task prompt), never baked into an identity.
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
fn no_shipped_prompt_prescribes_an_orchestration_process() {
    let mut prescriptive = Vec::new();
    for path in shipped_prompts() {
        let body = std::fs::read_to_string(&path)
            .expect("prompt is readable")
            .to_lowercase();
        for phrase in PRESCRIBED_PROCESS {
            if body.contains(&phrase.to_lowercase()) {
                prescriptive.push(format!("{} says {phrase:?}", label(&path)));
            }
        }
    }
    assert!(
        prescriptive.is_empty(),
        "prompts prescribe an orchestration process:\n  {}",
        prescriptive.join("\n  ")
    );
}
