//! Every shipped identity prompt must describe the surface the harness
//! actually serves.
//!
//! Every agent — top-level and spawned children alike — sees
//! `harness/prompts/default.txt` unless it runs as a directory agent profile,
//! whose resolved prompt replaces it; the separate sub-agent identity is
//! retired (children differ by policy, enrich layers or profile, never by
//! embedded prompt).

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

/// The single prompt file that the harness owns and sends to agents.
fn shipped_prompts() -> Vec<PathBuf> {
    vec![repo_root().join("harness/prompts/default.txt")]
}

fn label(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn harness_owns_the_only_shipped_prompts() {
    let prompts = shipped_prompts();
    assert!(prompts.iter().all(|path| path.is_file()));

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

/// The top-level identity prompt teaches the orchestration surface. The
/// sub-agent prompt intentionally omits it because children are leaves.
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
        let normalized = body.replace('\n', " ");
        assert!(
            body.contains("engine::register_trigger"),
            "{name} never mentions the callback primitive"
        );
        assert!(
            body.contains("harness::spawn"),
            "{name} never mentions how to start a sub-agent"
        );
        assert!(
            body.contains("display: { name, icon?, color? }"),
            "{name} never teaches the sub-agent display identity"
        );
        assert!(
            normalized.contains("user-visible identity, not its technical id"),
            "{name} conflates a child's display identity with its session id"
        );
        assert!(
            body.contains("orchestrator: true"),
            "{name} never mentions the leaf default's escape hatch"
        );
        assert!(
            normalized.contains("materially new investigative or action phase"),
            "{name} never asks for a user-visible update when a new phase starts"
        );
        assert!(
            normalized.contains("One update may cover any number of related function calls"),
            "{name} asks for progress without allowing one update to cover a call batch"
        );
        assert!(
            normalized.contains("lead with its concrete result"),
            "{name} never asks an update to report the preceding phase's result"
        );
        assert!(
            normalized.contains("summary of that whole batch"),
            "{name} does not connect the progress result to the completed call batch"
        );
        assert!(
            normalized.contains("still needs its concise `description`"),
            "{name} does not preserve the per-call activity description"
        );
        assert!(
            normalized.contains("For the ordinary text contract, use normal assistant text"),
            "{name} does not preserve the final answer after progress updates"
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
