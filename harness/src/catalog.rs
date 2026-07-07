//! The injected function catalog: a system-prompt block listing the
//! configured slice of the registry snapshot as `id — description` lines, so
//! agents answer "what can you do?"-class turns and find common ids without
//! an `engine::functions::list` round trip.
//!
//! Rendered per step from the live snapshot (always current — the
//! registry-change notice stays scoped to results the model fetched itself),
//! intersected with the turn's dispatch policy (never show an id the turn
//! cannot call), and byte-stable for identical (snapshot, policy, config)
//! inputs so the block prompt-caches.

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::clients::FunctionDescriptor;
use crate::policy::CompiledPolicy;

/// Rendered-entry cap: the catalog is an aid, not the registry — past this,
/// alphabetical order wins and the rest stays discoverable via `list`.
const MAX_ENTRIES: usize = 60;
/// Block size guard (bytes) — refuse to bloat the prompt on pathological
/// description lengths; the block is skipped entirely when exceeded.
const MAX_BLOCK_BYTES: usize = 24 * 1024;
/// Per-entry description cap (chars, after first-sentence truncation).
const MAX_DESC_CHARS: usize = 140;

const HEADER: &str = "Function catalog — a curated SUBSET of this engine's registry, current \
                      for this turn (not the full list: totals or exhaustive answers still \
                      need engine::functions::list). Call these directly; fetch contracts \
                      with engine::functions::info { function_ids }:";

/// Compile the configured catalog globs (policy-style semantics; invalid
/// globs are skipped with a warning, mirroring `CompiledPolicy`).
pub fn compile_globs(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        match Glob::new(p) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(_) => tracing::warn!(pattern = %p, "ignoring invalid catalog glob"),
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Render the catalog block, `None` when nothing qualifies (feature off,
/// empty intersection, or the size guard tripped).
pub fn render(
    functions: &[FunctionDescriptor],
    catalog: &GlobSet,
    policy: &CompiledPolicy,
) -> Option<String> {
    if catalog.is_empty() {
        return None;
    }
    let mut ids: Vec<&FunctionDescriptor> = functions
        .iter()
        .filter(|d| catalog.is_match(&d.function_id) && policy.allows(&d.function_id))
        .collect();
    if ids.is_empty() {
        return None;
    }
    ids.sort_by(|a, b| a.function_id.cmp(&b.function_id));
    ids.truncate(MAX_ENTRIES);

    let mut out = String::from(HEADER);
    for d in ids {
        out.push('\n');
        out.push_str(&d.function_id);
        let desc = first_sentence(d.description.as_deref().unwrap_or(""));
        if !desc.is_empty() {
            out.push_str(" — ");
            out.push_str(&desc);
        }
    }
    if out.len() > MAX_BLOCK_BYTES {
        tracing::warn!(
            bytes = out.len(),
            "catalog block exceeds the size guard; skipping injection"
        );
        return None;
    }
    Some(out)
}

/// First sentence of a description, capped at [`MAX_DESC_CHARS`] chars.
fn first_sentence(description: &str) -> String {
    let one_line = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let sentence = match one_line.find(". ") {
        Some(end) => &one_line[..end + 1],
        None => one_line.as_str(),
    };
    if sentence.chars().count() > MAX_DESC_CHARS {
        let truncated: String = sentence.chars().take(MAX_DESC_CHARS).collect();
        format!("{truncated}…")
    } else {
        sentence.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::turn::{ExposeMode, FunctionPolicy};

    fn desc(id: &str, description: &str) -> FunctionDescriptor {
        FunctionDescriptor {
            function_id: id.into(),
            description: (!description.is_empty()).then(|| description.to_string()),
            parameters: None,
        }
    }

    fn pol(allow: &[&str]) -> CompiledPolicy {
        CompiledPolicy::from(Some(&FunctionPolicy {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: vec![],
            expose: ExposeMode::AgentTrigger,
        }))
    }

    fn globs(patterns: &[&str]) -> GlobSet {
        compile_globs(&patterns.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn empty_config_disables_the_block() {
        let fns = vec![desc("shell::exec", "Run a command.")];
        assert!(render(&fns, &globs(&[]), &pol(&["*"])).is_none());
    }

    #[test]
    fn policy_intersection_hides_uncallable_ids() {
        let fns = vec![
            desc("shell::exec", "Run a command."),
            desc("web::fetch", "Fetch a URL."),
        ];
        let out = render(&fns, &globs(&["*"]), &pol(&["shell::*"])).unwrap();
        assert!(out.contains("shell::exec"));
        assert!(!out.contains("web::fetch"));
    }

    #[test]
    fn renders_sorted_first_sentence_lines() {
        let fns = vec![
            desc("web::fetch", "Fetch a URL. Second sentence never shows."),
            desc(
                "shell::exec",
                "Run one command to completion.\nMore detail.",
            ),
            desc("shell::noop", ""),
        ];
        let out = render(&fns, &globs(&["*"]), &pol(&["*"])).unwrap();
        let lines: Vec<&str> = out.lines().skip(1).collect();
        assert_eq!(
            lines,
            vec![
                "shell::exec — Run one command to completion.",
                "shell::noop",
                "web::fetch — Fetch a URL.",
            ]
        );
        assert!(!out.contains("Second sentence"));
    }

    #[test]
    fn long_descriptions_are_capped() {
        let fns = vec![desc("a::b", &"x".repeat(500))];
        let out = render(&fns, &globs(&["*"]), &pol(&["*"])).unwrap();
        assert!(out.lines().nth(1).unwrap().len() < 200);
        assert!(out.contains('…'));
    }

    #[test]
    fn entry_cap_keeps_alphabetical_head() {
        let fns: Vec<_> = (0..80).map(|i| desc(&format!("w::f{i:03}"), "d")).collect();
        let out = render(&fns, &globs(&["*"]), &pol(&["*"])).unwrap();
        assert_eq!(out.lines().count(), 1 + MAX_ENTRIES);
        assert!(out.contains("w::f000"));
        assert!(!out.contains("w::f079"));
    }

    #[test]
    fn byte_stable_for_identical_inputs() {
        let fns = vec![desc("b::y", "Second."), desc("a::x", "First.")];
        let a = render(&fns, &globs(&["*"]), &pol(&["*"])).unwrap();
        let b = render(&fns, &globs(&["*"]), &pol(&["*"])).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn invalid_glob_is_skipped_not_fatal() {
        let gs = compile_globs(&["shell::*".to_string(), "[".to_string()]);
        let fns = vec![desc("shell::exec", "Run.")];
        assert!(render(&fns, &gs, &pol(&["*"])).is_some());
    }
}
