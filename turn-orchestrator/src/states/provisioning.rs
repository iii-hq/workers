//! `provisioning` state handler.

use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::agent_call;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};
use crate::system_prompt;

pub async fn handle(
    iii: &III,
    _cfg: &std::sync::Arc<crate::config::TurnOrchestratorConfig>,
    record: &mut TurnStateRecord,
) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;

    let tools = json!([agent_call::agent_call_tool()]);
    persistence::save_function_schemas(iii, &record.session_id, tools.clone()).await;

    let override_prompt = request
        .get("system_prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let cwd = request.get("cwd").and_then(Value::as_str);
    let skills_index = fetch_skills_bootstrap(iii).await;
    let prompt = system_prompt::build(skills_index.as_deref(), cwd, override_prompt);
    let mut updated = request.clone();
    if let Some(obj) = updated.as_object_mut() {
        obj.insert("system_prompt".into(), json!(prompt));
    }
    persistence::save_run_request(iii, &record.session_id, updated).await;

    // Sandbox provisioning is the LLM's responsibility, learned from a
    // skill (registered separately via the skills worker). The dispatcher
    // does not auto-provision; sessions that need a sandbox follow a
    // skill recipe to call sandbox::list / sandbox::create themselves.

    record.transition_to(TurnState::AwaitingAssistant);
    Ok(())
}

/// One enriched `directory::skills::list` row, projected from the JSON
/// the worker returns. Kept tiny on purpose so the bootstrap helper is
/// unit-testable without an iii engine.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillRow {
    id: String,
    title: String,
    description: String,
}

/// Best-effort bootstrap of the skills surface for the system prompt.
///
/// Concatenates:
///   1. an indented client-side index built from `directory::skills::list`
///      (links every registered skill, deeper paths indented), and
///   2. the bodies of every **root-depth** registered skill (no `/` in the
///      id), each fetched via `directory::skills::get`.
///
/// The agent therefore boots with both the table of contents AND the
/// router-style bodies the agent normally reaches for first — eliminating
/// the round-trip to fetch the index on the first turn.
///
/// Any sub-step that fails returns `None` for that piece; the caller
/// degrades gracefully (the fallback section in `system_prompt::build`
/// covers a fully missing skills surface).
async fn fetch_skills_bootstrap(iii: &III) -> Option<String> {
    let rows = list_skills(iii).await;
    let index = if rows.is_empty() {
        None
    } else {
        Some(render_index_from_rows(&rows))
    };
    let root_ids: Vec<String> = rows
        .iter()
        .filter(|r| is_root_skill_id(&r.id))
        .map(|r| r.id.clone())
        .collect();
    let bodies = if root_ids.is_empty() {
        None
    } else {
        fetch_root_bodies(iii, &root_ids).await
    };

    match (index, bodies) {
        (Some(idx), Some(bod)) => Some(format!("{idx}\n\n---\n\n# Root skill bodies\n\n{bod}")),
        (Some(idx), None) => Some(idx),
        (None, Some(bod)) => Some(bod),
        (None, None) => None,
    }
}

/// Pull the enriched `directory::skills::list` rows. Empty list on any
/// failure or unexpected shape.
async fn list_skills(iii: &III) -> Vec<SkillRow> {
    let Ok(resp) = iii
        .trigger(TriggerRequest {
            function_id: "directory::skills::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    else {
        return Vec::new();
    };
    let Some(arr) = resp.get("skills").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter().filter_map(parse_skill_row).collect()
}

fn parse_skill_row(entry: &Value) -> Option<SkillRow> {
    let id = entry.get("id").and_then(Value::as_str)?.to_string();
    if id.is_empty() {
        return None;
    }
    let title = entry
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_string();
    let description = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(SkillRow {
        id,
        title,
        description,
    })
}

/// Indented bullet list of every skill, depth derived from the number
/// of `/` separators in the id. Output matches the line shape the
/// harness web slash-menu parser expects:
///
/// ```text
/// - [`<id>`](iii://<id>) — <title>
/// - [`<id>`](iii://<id>) — <title> — <description>
/// ```
fn render_index_from_rows(rows: &[SkillRow]) -> String {
    let mut out = String::from(
        "# Skills\n\nRead each skill's body for orientation on when and why to call its functions. \
         Sub-skills are indented under their parent path so a top-level skill stays small \
         and the LLM can drill in only when it needs more detail.\n\n",
    );
    for row in rows {
        let depth = row.id.matches('/').count();
        let indent = " ".repeat(depth * 2);
        let suffix = if row.description.is_empty() {
            String::new()
        } else {
            format!(" — {}", row.description)
        };
        out.push_str(&format!(
            "{indent}- [`{id}`](iii://{id}) — {title}{suffix}\n",
            id = row.id,
            title = row.title,
        ));
    }
    out
}

/// Fetch the body of every root-depth skill via `directory::skills::get`
/// and join them with `\n\n---\n\n`. Errors per id drop that body but
/// the rest of the bootstrap proceeds.
async fn fetch_root_bodies(iii: &III, root_ids: &[String]) -> Option<String> {
    let mut sections: Vec<String> = Vec::with_capacity(root_ids.len());
    for id in root_ids {
        let Ok(resp) = iii
            .trigger(TriggerRequest {
                function_id: "directory::skills::get".into(),
                payload: json!({ "id": id }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
        else {
            continue;
        };
        let body = resp.get("body").and_then(Value::as_str).unwrap_or("");
        if body.is_empty() {
            continue;
        }
        sections.push(format!("# iii://{id}\n\n{}", body.trim_end()));
    }
    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n---\n\n"))
    }
}

/// `iii`, `harness`, and `<single-segment>/index` are root; `resend/email`,
/// `shell/bash` are not.
fn is_root_skill_id(id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    match id.split('/').collect::<Vec<_>>().as_slice() {
        [single] => !single.is_empty(),
        [head, "index"] => !head.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_root_skill_id, parse_skill_row, render_index_from_rows, SkillRow};
    use serde_json::json;

    #[test]
    fn single_segment_ids_are_root() {
        assert!(is_root_skill_id("iii"));
        assert!(is_root_skill_id("harness"));
        assert!(is_root_skill_id("shell-bash"));
    }

    #[test]
    fn index_suffixed_ids_are_root() {
        assert!(is_root_skill_id("shell/index"));
    }

    #[test]
    fn nested_ids_are_not_root() {
        assert!(!is_root_skill_id("resend/email"));
        assert!(!is_root_skill_id("shell/bash"));
        assert!(!is_root_skill_id("shell/bash/exec"));
    }

    #[test]
    fn empty_id_is_not_root() {
        assert!(!is_root_skill_id(""));
    }

    #[test]
    fn parse_row_extracts_required_id_and_optional_title_description() {
        let v = json!({ "id": "shell", "title": "Shell", "description": "Run commands." });
        let row = parse_skill_row(&v).unwrap();
        assert_eq!(row.id, "shell");
        assert_eq!(row.title, "Shell");
        assert_eq!(row.description, "Run commands.");
    }

    #[test]
    fn parse_row_falls_back_to_id_when_title_missing() {
        let v = json!({ "id": "shell" });
        let row = parse_skill_row(&v).unwrap();
        assert_eq!(row.title, "shell");
        assert_eq!(row.description, "");
    }

    #[test]
    fn parse_row_skips_entries_without_id() {
        assert!(parse_skill_row(&json!({})).is_none());
        assert!(parse_skill_row(&json!({ "title": "x" })).is_none());
        assert!(parse_skill_row(&json!({ "id": "" })).is_none());
    }

    #[test]
    fn render_index_indents_by_depth_and_includes_link_label_description() {
        let rows = vec![
            SkillRow {
                id: "shell".into(),
                title: "Shell".into(),
                description: "Run commands.".into(),
            },
            SkillRow {
                id: "shell/exec".into(),
                title: "shell::exec".into(),
                description: "Foreground exec.".into(),
            },
            SkillRow {
                id: "shell/exec/bg".into(),
                title: "shell::exec_bg".into(),
                description: String::new(),
            },
        ];
        let idx = render_index_from_rows(&rows);
        assert!(idx.contains("# Skills"));
        assert!(idx.contains("- [`shell`](iii://shell) — Shell — Run commands."));
        assert!(
            idx.contains("  - [`shell/exec`](iii://shell/exec) — shell::exec — Foreground exec.")
        );
        // Empty description: trailing em-dash dropped.
        assert!(idx.contains("    - [`shell/exec/bg`](iii://shell/exec/bg) — shell::exec_bg\n"));
    }

    #[test]
    fn render_index_handles_zero_rows() {
        let idx = render_index_from_rows(&[]);
        assert!(idx.contains("# Skills"));
    }
}
