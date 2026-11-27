//! What a person and an agent read in a problem: a short title per code, the
//! one-line fix (from `skills/plan-codes.md`, the single source, embedded at
//! build time) made concrete with the problem's own data, and a detail that
//! says what is wrong in the cloud's words.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

/// The skill's tables: `| CODE | ... | Fix |` for the cloud's codes and
/// `| CODE | Owner | Fix |` for the checks here.
const PLAN_CODES: &str = include_str!("../skills/plan-codes.md");

struct Fixes {
    cloud: HashMap<String, String>,
    local: HashMap<String, String>,
}

fn fixes() -> &'static Fixes {
    static FIXES: OnceLock<Fixes> = OnceLock::new();
    FIXES.get_or_init(|| {
        let mut f = Fixes {
            cloud: HashMap::new(),
            local: HashMap::new(),
        };
        for line in PLAN_CODES.lines().filter(|l| l.starts_with("| `")) {
            let cells: Vec<&str> = line
                .trim()
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            let (Some(code), Some(fix)) = (cells.first(), cells.last()) else {
                continue;
            };
            let code = code.trim_matches('`').to_string();
            // Five columns: the cloud's table. Three: the local one.
            let table = if cells.len() >= 5 {
                &mut f.cloud
            } else {
                &mut f.local
            };
            table.entry(code).or_insert_with(|| (*fix).to_string());
        }
        f
    })
}

/// Every code the skill lists, with its fix.
pub fn known_codes() -> Vec<(String, String)> {
    let f = fixes();
    f.cloud
        .iter()
        .chain(f.local.iter())
        .map(|(c, x)| (c.clone(), x.clone()))
        .collect()
}

/// The one-line fix of a code. `here` prefers the checks-here table (a code in
/// both, like `LOCK_MISSING`, says different things).
pub fn fix_for(code: &str, here: bool) -> Option<String> {
    let f = fixes();
    let (first, second) = if here {
        (&f.local, &f.cloud)
    } else {
        (&f.cloud, &f.local)
    };
    first.get(code).or_else(|| second.get(code)).cloned()
}

/// A short title for a code; never the bare code.
pub fn title_for(code: &str) -> String {
    let named = match code {
        "MISSING_WORKER_DIRECTORY" => "A worker directory is missing",
        "LOCK_MISSING" => "The lock file is missing",
        "COMPOSE_LOCK_OUT_OF_DATE" => "The lock file is out of date",
        "INVALID_COMPOSE_FILE" => "The compose file is not valid",
        "MANIFEST_NOT_FOUND" => "The compose file was not found",
        "INVALID_CLOUD_BLOCK" => "The cloud: block is not valid",
        "PLURAL_FILENAME" => "The compose file name is plural",
        "PATH_OUTSIDE_BUNDLE" => "A path:// folder is outside the repository",
        "RUNTIME_UNSUPPORTED" => "The runtime is not supported",
        "LITERAL_CREDENTIAL" => "A credential is written in the file",
        "SECRET_MISSING" => "A secret has no value",
        "MULTIPLE_HTTP_GROUPS" => "More than one group runs http",
        "UNGROUPED_WORKER" => "A worker is in no group",
        "SQLITE_UNSUPPORTED" => "SQLite is not supported",
        "SQLITE_DEFAULT_REPLACED" => "SQLite is replaced by Postgres",
        "ENV_FILE_MISSING" => "A referenced .env file is missing",
        "LOCAL_CONFIG_NOT_SET" => "The workers' config isn't in iii cloud yet",
        "EMPTY_DEFAULT_VARIABLE" => "A variable defaults to empty",
        "HEALTHCHECK_FAILED" => "The health check failed",
        "RELEASE_FAILED" => "The release command failed",
        "BUILD_FAILED" => "The build failed",
        "DATABASE_NOT_MANAGED" => "The database is not managed by iii cloud",
        "BASE_NOT_IN_HISTORY" => "The base branch moved on",
        "UNCOMMITTED_CHANGES" => "There are changes that are not committed",
        "HTTP_WORKER_NOT_PACKAGE" => "The http worker is not package://http",
        "OLD_ATTEMPT_COMMITS" => "These commits come from an earlier try",
        "RUN_COMMAND_NOT_IN_IMAGE" => "A command uses a tool the runtime's image does not have",
        "RUN_LEFT_FILES" => "The local run left files changed in the worktree",
        "BROUGHT_FILES" => "The worktree holds copies of files from your folder",
        "BROUGHT_FILES_COMMIT" => "A commit holds files copied from your folder",
        "SIGN_IN_REQUIRED" => "Not signed in to iii cloud",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    // `QUEUE_ADAPTER_MISSING` -> "Queue adapter missing".
    let words = code.replace('_', " ").to_lowercase();
    let mut chars = words.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn text<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// What is wrong, in the cloud's words: its `context.message`, `message` and
/// `hint`, then where in the YAML (`location.path`). Never "CODE · file".
pub fn plan_detail(issue: &Value) -> Option<String> {
    let mut parts: Vec<&str> = vec![];
    for p in ["/context/message", "/message", "/hint"] {
        if let Some(t) = text(issue, p)
            && !parts.contains(&t)
        {
            parts.push(t);
        }
    }
    let mut detail = parts.join(" ");
    if detail.is_empty() {
        // No message at all (a missing lock says only where): say what that means.
        let file = text(issue, "/location/file");
        let line = issue.pointer("/location/line").and_then(Value::as_u64);
        detail = match (issue.get("code").and_then(Value::as_str), file, line) {
            (Some("LOCK_MISSING"), Some(f), _) => format!("{f} is not in the repository."),
            (_, Some(f), Some(l)) => format!("See {f}:{l}."),
            (_, Some(f), None) => format!("See {f}."),
            _ => String::new(),
        };
    }
    if let Some(path) = text(issue, "/location/path")
        && !detail.contains(path)
    {
        detail = format!("{detail} (at {path})").trim().to_string();
    }
    (!detail.is_empty()).then_some(detail)
}

/// The directory a `MISSING_WORKER_DIRECTORY` message names.
fn missing_dir(message: &str) -> Option<&str> {
    let rest = message.split("worker directory ").nth(1)?;
    rest.split(" does not exist")
        .next()
        .map(str::trim)
        .filter(|d| !d.is_empty())
}

/// The web setting can come from the compose file or from what the environment
/// stored (an earlier manifest, the console): the fix says where to change it.
fn web_not_servable_fix(issue: &Value, plan: &Value) -> String {
    let source = text(plan, "/settings/web/container_source").unwrap_or("");
    let mut fix = if matches!(source, "file" | "manifest") {
        "Remove `cloud.web` from the compose file, or point it at a container that serves its own http port."
    } else {
        "Change Web in the environment's settings (or remove it): the value is stored in the environment, so removing `cloud.web` from the compose file does not clear it. Console: the environment's Settings, Web."
    }
    .to_string();
    if text(issue, "/context/role") == Some("http") {
        fix.push_str(" The http package already serves your routes: set Web to none.");
    }
    fix
}

/// A value problem about a database URL is the agent's: iii cloud makes the
/// database, so the person has no value to give (the Values editor shows the
/// row without an input). The other codes and names are left as they are.
pub fn database_value_owner(p: &mut crate::types::Problem, name: &str) {
    let value_code = matches!(
        p.code.as_str(),
        "EMPTY_DEFAULT_VARIABLE" | "SECRET_MISSING" | "LOCAL_VARIABLE_UNSET"
    );
    if value_code && crate::local::database_shaped(name) {
        p.owner = crate::types::Owner::Agent;
        p.fix = Some(format!(
            "Point {name} at the managed database's binding (`${{DATABASE_<ALIAS>_URL}}`) in the app file; iii cloud makes the database."
        ));
    }
}

/// The variable a plan issue is about: its `context.variable`, else the
/// first word of its message ("SESSION_SECRET has no value.").
pub fn issue_variable(issue: &Value) -> Option<&str> {
    text(issue, "/context/variable").or_else(|| {
        text(issue, "/message")?
            .split_whitespace()
            .next()
            .map(|w| w.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
            .filter(|w| !w.is_empty())
    })
}

/// The fix of a plan issue: the cloud's own `fix` when it sent one, else the
/// skill's one line, made concrete with the issue's file, path and message.
pub fn plan_fix(code: &str, issue: &Value, plan: &Value) -> Option<String> {
    if code == "WEB_CONTAINER_NOT_SERVABLE" {
        return Some(web_not_servable_fix(issue, plan));
    }
    if let Some(own) = text(issue, "/fix") {
        return Some(own.to_string());
    }
    let file = text(issue, "/location/file");
    match code {
        "MISSING_WORKER_DIRECTORY" => {
            let dir = text(issue, "/context/message").and_then(missing_dir);
            let path = text(issue, "/location/path");
            let root = file.and_then(|f| f.rsplit_once('/')).map(|(d, _)| d);
            if let (Some(dir), Some(path)) = (dir, path) {
                let rel = root
                    .map(|r| format!("; paths are relative to {r}/"))
                    .unwrap_or_default();
                return Some(format!(
                    "Point {path} at an existing directory, or add {dir}{rel}."
                ));
            }
        }
        "LOCK_MISSING" => {
            if let Some(f) = file {
                let manifest = format!("{}.yaml", f.trim_end_matches(".lock"));
                return Some(format!(
                    "Create the lock: run `iii compose build -f {manifest}` with iii 0.27 in the worktree, and commit {f} next to it."
                ));
            }
        }
        _ => {}
    }
    fix_for(code, false)
}
