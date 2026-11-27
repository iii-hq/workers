//! The checks here (spec §4): fast, read-only, on the workspace. Pure over a
//! `LocalInput`, so each rule is unit-tested without a network or a process.
//! Also the compose manifest reader and the install table the run reuses.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::check::problem_id;
use crate::types::*;

// ------------------------------------------------------------ the manifest

/// The compose file the environment deploys, parsed loosely.
pub struct Manifest {
    pub file: PathBuf,
    pub dir: PathBuf,
    pub text: String,
    pub yaml: Value,
}

impl Manifest {
    /// `manifest` is relative to the repository root (the environment's
    /// `manifest`); default `worker-compose.yaml` at the root.
    pub fn load(workspace: &Path, manifest: Option<&str>) -> Option<Manifest> {
        if manifest.is_some_and(|m| confined(m).is_none()) {
            return None;
        }
        let file = workspace.join(manifest.unwrap_or("worker-compose.yaml"));
        let text = std::fs::read_to_string(&file).ok()?;
        let yaml: Value = serde_yaml_ng::from_str(&text).ok()?;
        Some(Manifest {
            dir: file.parent()?.to_path_buf(),
            file,
            text,
            yaml,
        })
    }

    pub fn containers(&self) -> Vec<(String, &Value)> {
        self.yaml
            .get("containers")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v)).collect())
            .unwrap_or_default()
    }

    /// The directory of a `path://` container's worker.
    pub fn path_worker_dir(&self, container: &Value) -> Option<PathBuf> {
        let rest = container.get("worker")?.as_str()?.strip_prefix("path://")?;
        let p = Path::new(rest);
        Some(if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dir.join(p)
        })
    }

    pub fn is_package(container: &Value, name: &str) -> bool {
        container
            .get("worker")
            .and_then(Value::as_str)
            .is_some_and(|w| {
                w.strip_prefix("package://")
                    .is_some_and(|n| n.split(['@', '/']).next() == Some(name))
            })
    }

    /// The cloud's rule: only `package://http` is the http worker.
    pub fn is_http_worker(&self, _name: &str, container: &Value) -> bool {
        Self::is_package(container, "http")
    }

    /// A container named `http` (in the manifest, or in its own `iii.worker.yaml`)
    /// that is not `package://http`: plain code in the cloud, serving no route.
    pub fn fake_http_containers(&self) -> Vec<String> {
        self.containers()
            .into_iter()
            .filter(|(n, c)| !self.is_http_worker(n, c) && self.named_http(n, c))
            .map(|(n, _)| n)
            .collect()
    }

    fn named_http(&self, name: &str, container: &Value) -> bool {
        name == "http"
            || self
                .path_worker_dir(container)
                .and_then(|dir| std::fs::read_to_string(dir.join("iii.worker.yaml")).ok())
                .and_then(|text| serde_yaml_ng::from_str::<Value>(&text).ok())
                .and_then(|own| own.get("name").and_then(Value::as_str).map(str::to_owned))
                .is_some_and(|n| n == "http")
    }

    /// The names of the containers that are http workers.
    pub fn http_containers(&self) -> Vec<String> {
        self.containers()
            .into_iter()
            .filter(|(n, c)| self.is_http_worker(n, c))
            .map(|(n, _)| n)
            .collect()
    }

    pub fn cloud(&self) -> Option<&Value> {
        self.yaml.get("cloud")
    }

    /// `cloud.build`: `{command}`, or a plain string, or `build_command`.
    pub fn cloud_build(&self) -> Option<String> {
        let cloud = self.cloud()?;
        let build = cloud.get("build");
        build
            .and_then(|b| b.get("command"))
            .or(build.filter(|b| b.is_string()))
            .or_else(|| cloud.get("build_command"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    pub fn cloud_release(&self) -> Option<Release> {
        let r = self.cloud()?.get("release")?;
        Some(Release {
            worker: r.get("worker")?.as_str()?.to_string(),
            command: r.get("command")?.as_str()?.to_string(),
            timeout: r.get("timeout").and_then(Value::as_str).map(str::to_owned),
        })
    }

    pub fn cloud_healthcheck(&self) -> Option<Healthcheck> {
        let h = self.cloud()?.get("healthcheck")?;
        Some(Healthcheck {
            path: h.get("path")?.as_str()?.to_string(),
            timeout: h.get("timeout").and_then(Value::as_str).map(str::to_owned),
        })
    }

    /// `cloud.web.api.prefix` and `strip` (default false).
    pub fn web_prefix(&self) -> (Option<String>, bool) {
        let api = self
            .cloud()
            .and_then(|c| c.get("web"))
            .and_then(|w| w.get("api"));
        let prefix = api
            .and_then(|a| a.get("prefix"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let strip = api
            .and_then(|a| a.get("strip"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (prefix, strip)
    }

    pub fn lock_path(&self) -> PathBuf {
        self.file.with_extension("lock")
    }

    /// Database resources the run starts: containers on `package://database`,
    /// with the aliases of `config_override.databases` (else `primary`, as the
    /// cloud's plan does: `crates/plan/src/resources.rs:83-90`).
    /// The names iii cloud binds for the declared databases (`DATABASE_<A>_URL`, `..._DIRECT_URL`).
    pub fn database_bindings(&self) -> Vec<String> {
        let mut out = vec![];
        for (_, aliases) in self.database_aliases() {
            for a in aliases {
                let b = database_binding(&a);
                out.push(b.replace("_URL", "_DIRECT_URL"));
                out.push(b);
            }
        }
        out
    }

    pub fn database_aliases(&self) -> Vec<(String, Vec<String>)> {
        self.containers()
            .into_iter()
            .filter(|(_, c)| Self::is_package(c, "database"))
            .map(|(name, c)| {
                let aliases: Vec<String> = c
                    .pointer("/config_override/databases")
                    .and_then(Value::as_object)
                    .map(|m| m.keys().cloned().collect())
                    .filter(|v: &Vec<String>| !v.is_empty())
                    .unwrap_or_else(|| vec!["primary".to_string()]);
                (name, aliases)
            })
            .collect()
    }
}

/// `(containers, has a cloud: section)` of a compose file of the repository,
/// for the app file pickers. `None` containers: not here, or it does not parse.
pub fn compose_facts(root: &Path, path: &str) -> (Option<u32>, bool) {
    match Manifest::load(root, Some(path)) {
        Some(m) => (
            Some(m.containers().len() as u32),
            m.cloud().is_some_and(|c| !c.is_null()),
        ),
        None => (None, false),
    }
}

/// A path the cloud sent, kept inside the workspace: relative, no `..`, no
/// backslash.
pub fn confined(path: &str) -> Option<&str> {
    let p = Path::new(path);
    let ok = !path.contains('\\')
        && !p.is_absolute()
        && !p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        });
    ok.then_some(path)
}

/// The cloud's rule for a compose file it plans (`iii_plan::manifest_path`):
/// a `worker-compose*.yaml` from the repository root, `/` separated, with no
/// empty, `.` or `..` segment, no control character, at most 1024 bytes.
pub fn manifest_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.starts_with("worker-compose")
        && file.ends_with(".yaml")
        && path.len() <= 1024
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub worker: String,
    pub command: String,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Healthcheck {
    pub path: String,
    pub timeout: Option<String>,
}

/// `DATABASE_<ALIAS>_URL`, as the cloud binds a Postgres resource
/// (`crates/plan/src/resources.rs:390-402`).
pub fn database_binding(alias: &str) -> String {
    format!("DATABASE_{}_URL", alias.to_uppercase().replace('-', "_"))
}

/// One `${NAME}` in a container of the compose file, as iii cloud reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarUse {
    pub name: String,
    /// `None`: `${NAME}`; `Some("")`: `${NAME:-}`; else the default as written.
    pub default: Option<String>,
    pub container: String,
    /// The first line of the file with it written (1-based); comments don't count.
    pub line: Option<u32>,
}

/// A `${…}` iii cloud delivers to the container as literal text: `${A:?}`,
/// `${A?}`, `${A-x}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    pub written: String,
    pub container: String,
    pub line: Option<u32>,
}

fn is_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The references of one value, a port of the cloud's `parts`
/// (`crates/plan/src/variables.rs`): `${NAME}`, `${NAME:-default}`
/// (`${NAME:default}` in `config_override`); `$$` is a literal `$`; anything
/// else is text. Returns (name, default, written) and the unsupported forms.
/// A reference: its name, its default and the `${…}` as written.
type Ref = (String, Option<String>, String);

fn value_refs(text: &str, in_config: bool) -> (Vec<Ref>, Vec<String>) {
    let (mut refs, mut bad) = (vec![], vec![]);
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        rest = &rest[at..];
        if let Some(after) = rest.strip_prefix("$$") {
            rest = after;
        } else if let Some(after) = rest.strip_prefix("${") {
            let Some(end) = after.find('}') else { break };
            let body = &after[..end];
            let written = rest[..end + 3].to_string();
            rest = &after[end + 1..];
            let split = body
                .split_once(":-")
                .or_else(|| in_config.then(|| body.split_once(':')).flatten());
            let (name, default) = match split {
                Some((name, default)) => (name.trim(), Some(default)),
                None => (body.trim(), None),
            };
            if is_var_name(name) {
                refs.push((name.to_string(), default.map(str::to_string), written));
                continue;
            }
            let cut = body
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(body.len());
            let (head, tail) = body.split_at(cut);
            if is_var_name(head) && (tail.starts_with(":?") || tail.starts_with(['?', '-'])) {
                bad.push(written);
            }
        } else {
            rest = &rest[1..];
        }
    }
    (refs, bad)
}

impl Manifest {
    /// Every reference in `containers.*`, in document order, as iii cloud
    /// scans them: only parsed YAML counts (a comment is not a reference).
    pub fn variable_uses(&self) -> (Vec<VarUse>, Vec<Unsupported>) {
        use serde_yaml_ng::Value as Yaml;
        type Found = Vec<(String, Option<String>, String, String)>;
        fn walk(
            value: &Yaml,
            container: &str,
            in_config: bool,
            out: &mut Found,
            bad: &mut Vec<(String, String)>,
        ) {
            match value {
                Yaml::String(text) => {
                    let (refs, unsupported) = value_refs(text, in_config);
                    out.extend(
                        refs.into_iter()
                            .map(|(n, d, w)| (n, d, w, container.to_string())),
                    );
                    bad.extend(unsupported.into_iter().map(|w| (w, container.to_string())));
                }
                Yaml::Mapping(map) => {
                    for (key, child) in map {
                        if key.as_str().is_some() {
                            walk(child, container, in_config, out, bad);
                        }
                    }
                }
                Yaml::Sequence(items) => {
                    for child in items {
                        walk(child, container, in_config, out, bad);
                    }
                }
                _ => {}
            }
        }
        // Parsed again for its order: the document's, as the cloud walks it.
        let raw: Yaml = serde_yaml_ng::from_str(&self.text).unwrap_or(Yaml::Null);
        let (mut out, mut bad) = (vec![], vec![]);
        for (key, container) in raw
            .get("containers")
            .and_then(Yaml::as_mapping)
            .into_iter()
            .flatten()
        {
            let Some(name) = key.as_str() else { continue };
            // The cloud leaves the sandbox adapter out before it reads anything.
            let adapter = container
                .get("worker")
                .and_then(Yaml::as_str)
                .and_then(|w| w.strip_prefix("package://"))
                .is_some_and(|r| {
                    let (host, package) = r.rsplit_once('/').unwrap_or(("", r));
                    (host.is_empty() || host == "api.workers.iii.dev")
                        && package == "sandbox-libkrun"
                });
            if adapter {
                continue;
            }
            match container.as_mapping() {
                Some(fields) => {
                    for (field, child) in fields {
                        if let Some(field) = field.as_str() {
                            walk(child, name, field == "config_override", &mut out, &mut bad);
                        }
                    }
                }
                None => walk(container, name, false, &mut out, &mut bad),
            }
        }
        let line_of = |written: &str| {
            self.text
                .lines()
                .position(|l| !l.trim_start().starts_with('#') && l.contains(written))
                .map(|i| i as u32 + 1)
        };
        (
            out.into_iter()
                .map(|(name, default, written, container)| VarUse {
                    line: line_of(&written),
                    name,
                    default,
                    container,
                })
                .collect(),
            bad.into_iter()
                .map(|(written, container)| Unsupported {
                    line: line_of(&written),
                    written,
                    container,
                })
                .collect(),
        )
    }
}

/// The run's own `.env`: the workspace root's, then the compose file's
/// folder's (which wins), the last line of a name winning.
pub fn worktree_env(workspace: &Path, m: &Manifest) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for dir in [workspace, m.dir.as_path()] {
        if let Ok(text) = std::fs::read_to_string(dir.join(".env")) {
            for (k, v) in parse_env(&text) {
                out.retain(|(n, _)| *n != k);
                out.push((k, v));
            }
        }
    }
    out
}

/// Names the compose file reads with an empty default (`${NAME:-}`) and no read without a default.
/// iii cloud gives them their saved value when it has one, so the run here must too.
pub fn empty_default_variables(m: &Manifest) -> Vec<String> {
    let uses = m.variable_uses().0;
    let mut found: Vec<String> = vec![];
    for u in &uses {
        if u.default.as_deref() == Some("")
            && !found.contains(&u.name)
            && !uses.iter().any(|x| x.name == u.name && x.default.is_none())
        {
            found.push(u.name.clone());
        }
    }
    found
}

/// Names the compose file reads with no default (`${NAME}`), in order, without repeats.
pub fn required_variables(m: &Manifest) -> Vec<String> {
    let mut found: Vec<String> = vec![];
    for u in m.variable_uses().0 {
        if u.default.is_none() && !found.contains(&u.name) {
            found.push(u.name);
        }
    }
    found
}

/// The lines of a `.env` file, as the console's import reads them (`parseEnv`,
/// `apps/web/src/plan/env-import.tsx`): `NAME=value`, `export NAME=value`, a
/// value in single or double quotes (in double quotes `\n` is a new line and
/// `\"` a quote), a `#` comment on its own line or after an unquoted value.
/// The last line of a name wins; the order is of those last lines.
// ponytail: no raw multi-line quoted values
pub fn parse_env(text: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = vec![];
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = match line.strip_prefix("export") {
            Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
            _ => line,
        };
        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim_end();
        if name.is_empty() || name.contains(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim_start();
        // The first closing quote ends the value; in double quotes `\"` does not close it.
        let close = |q: char| {
            rest.char_indices()
                .skip(1)
                .find(|&(i, c)| c == q && !(q == '"' && rest[..i].ends_with('\\')))
                .map(|(i, _)| (q, i))
        };
        let quote = rest.chars().next().filter(|q| *q == '"' || *q == '\'');
        let value = match quote.and_then(close) {
            Some(('"', end)) => rest[1..end].replace("\\n", "\n").replace("\\\"", "\""),
            Some((_, end)) => rest[1..end].to_string(),
            None => {
                let cut = rest
                    .char_indices()
                    .find(|&(i, c)| c == '#' && i > 0 && rest[..i].ends_with(char::is_whitespace))
                    .map(|(i, _)| i)
                    .unwrap_or(rest.len());
                rest[..cut].trim().to_string()
            }
        };
        found.retain(|(n, _)| n != name);
        found.push((name.to_string(), value));
    }
    found
}

// ---------------------------------------------------------- install table

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Pip,
    Uv,
    Go,
    Cargo,
}

impl Lock {
    pub fn file(self) -> &'static str {
        match self {
            Self::Npm => "package-lock.json",
            Self::Pnpm => "pnpm-lock.yaml",
            Self::Yarn => "yarn.lock",
            Self::Bun => "bun.lock",
            Self::Pip => "requirements.txt",
            Self::Uv => "uv.lock",
            Self::Go => "go.sum",
            Self::Cargo => "Cargo.lock",
        }
    }

    pub fn present(self, root: &Path) -> bool {
        match self {
            Self::Bun => root.join("bun.lock").exists() || root.join("bun.lockb").exists(),
            Self::Go => root.join("go.sum").exists() || root.join("go.mod").exists(),
            other => root.join(other.file()).exists(),
        }
    }

    const ALL: [Lock; 8] = [
        Self::Npm,
        Self::Pnpm,
        Self::Yarn,
        Self::Bun,
        Self::Pip,
        Self::Uv,
        Self::Go,
        Self::Cargo,
    ];
}

/// The runtime name without its version: `node@24` is `node`.
pub fn runtime_base(runtime: &str) -> &str {
    runtime.split('@').next().unwrap_or(runtime)
}

/// Tools the cloud's image of a runtime is KNOWN not to hold: `(runtime base, tool)`. Only what is
/// confirmed goes here (a false block is worse than a miss); a row is one line.
const NOT_IN_IMAGE: &[(&str, &str)] = &[("node", "bun"), ("node", "bunx")];

/// The parts of a shell command, split on `&&`, `||`, `;` and `|`: each part's words from its command
/// word on (leading `NAME=value` words and `exec` skipped).
fn command_parts(command: &str) -> Vec<Vec<&str>> {
    // The body of a heredoc (`<<WORD` ... `WORD`) is data, not commands: its lines are skipped.
    let mut end: Option<String> = None;
    let mut code: Vec<&str> = vec![];
    for line in command.lines() {
        if let Some(word) = &end {
            if line.trim() == word {
                end = None;
            }
            continue;
        }
        code.push(line);
        if let Some(at) = line.find("<<") {
            let word: String = line[at + 2..]
                .trim_start_matches('-')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            if !word.is_empty() {
                end = Some(word);
            }
        }
    }
    code.into_iter()
        .flat_map(|line| line.split(['&', '|', ';']))
        .map(|part| {
            // Redirections are not part of the command: `env > /tmp/x` is `env`.
            let mut words: Vec<&str> = vec![];
            let mut skip_target = false;
            for w in part.split_whitespace() {
                if skip_target {
                    skip_target = false;
                    continue;
                }
                let redirect = w.trim_start_matches(|c: char| c.is_ascii_digit());
                if redirect.starts_with('>') || redirect.starts_with('<') {
                    // `>`, `>>`, `2>`, `<`: the next word is its target; `>file` carries it.
                    skip_target = redirect.chars().all(|c| c == '>' || c == '<');
                    continue;
                }
                words.push(w);
            }
            words
                .into_iter()
                .skip_while(|w| {
                    *w == "exec"
                        || w.split_once('=').is_some_and(|(k, _)| {
                            !k.is_empty()
                                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        })
                })
                .collect()
        })
        .filter(|words: &Vec<&str>| !words.is_empty())
        .collect()
}

/// The tools of `command` that the image of `runtime` is known not to have: the command word of
/// each part (see `command_parts`).
pub fn tools_missing_in_image(runtime: &str, command: &str) -> Vec<String> {
    let mut found: Vec<String> = vec![];
    for words in command_parts(command) {
        let t = words[0];
        if NOT_IN_IMAGE.iter().any(|(r, x)| *r == runtime && x == &t)
            && !found.iter().any(|f| f == t)
        {
            found.push(t.to_string());
        }
    }
    found
}

/// The part of `command` that prints every variable of the environment: `env` alone, `printenv`
/// (with or without a name), `set` alone, `export -p`. (`env FOO=1 cmd` runs `cmd`: not this.)
pub fn prints_environment(command: &str) -> Option<String> {
    command_parts(command)
        .into_iter()
        .find(|w| {
            matches!(
                w.as_slice(),
                ["env"] | ["set"] | ["export"] | ["export", "-p"] | ["printenv", ..]
            )
        })
        .map(|w| w.join(" "))
}

fn accepted_locks(runtime: &str) -> &'static [Lock] {
    match runtime {
        "node" => &[Lock::Npm, Lock::Pnpm, Lock::Yarn],
        "bun" => &[Lock::Bun],
        "python" => &[Lock::Pip, Lock::Uv],
        "go" => &[Lock::Go],
        "rust" => &[Lock::Cargo],
        _ => &[],
    }
}

/// The lock the runtime installs from at `root`, in the cloud's preference order.
pub fn lock_for(runtime: &str, root: &Path) -> Option<Lock> {
    accepted_locks(runtime_base(runtime))
        .iter()
        .copied()
        .find(|l| l.present(root))
}

/// The cloud's frozen install (`crates/build/src/lib.rs:411-430`), with the
/// container-only parts dropped (corepack's install directory; the venv goes
/// to `<root>/.venv`). `(command, extra environment)`.
pub fn install_command(lock: Lock) -> (&'static str, &'static [(&'static str, &'static str)]) {
    match lock {
        Lock::Npm => ("npm ci", &[]),
        Lock::Pnpm => ("pnpm install --frozen-lockfile", &[]),
        Lock::Yarn => ("yarn install --frozen-lockfile", &[]),
        Lock::Bun => ("bun install --frozen-lockfile", &[]),
        Lock::Pip => (
            "python3 -m venv .venv && .venv/bin/pip install -r requirements.txt",
            &[],
        ),
        Lock::Uv => (
            "uv sync --frozen --inexact",
            &[("UV_PROJECT_ENVIRONMENT", ".venv")],
        ),
        Lock::Go => ("go mod download", &[]),
        Lock::Cargo => ("cargo fetch --locked", &[]),
    }
}

fn language_runtime(language: &str) -> &str {
    match language {
        "typescript" | "javascript" | "node" => "node",
        other => other,
    }
}

/// A worker's runtime: the plan's, else `cloud.runtimes`, else its own
/// `iii.worker.yaml`.
/// The runtime the cloud itself would use: the plan's, else `cloud.runtimes`. (Not the worker's
/// `iii.worker.yaml` language: the cloud detects a bun project from its start command, so that
/// guess may differ from what it will run.)
pub fn declared_runtime(
    manifest: &Manifest,
    name: &str,
    plan: Option<&[(String, Option<String>)]>,
) -> Option<String> {
    if let Some(r) = plan
        .and_then(|p| p.iter().find(|(n, _)| n == name))
        .and_then(|(_, r)| r.clone())
    {
        return Some(r);
    }
    manifest
        .cloud()
        .and_then(|c| c.pointer(&format!("/runtimes/{name}")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub fn worker_runtime(
    manifest: &Manifest,
    name: &str,
    dir: &Path,
    plan: Option<&[(String, Option<String>)]>,
) -> Option<String> {
    if let Some(r) = declared_runtime(manifest, name, plan) {
        return Some(r);
    }
    let own: Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(dir.join("iii.worker.yaml")).ok()?)
            .ok()?;
    let r = own
        .get("runtime")
        .or_else(|| own.get("language"))?
        .as_str()?;
    Some(language_runtime(r).to_string())
}

// ----------------------------------------------------------- the checks

pub struct LocalInput<'a> {
    pub workspace: &'a Path,
    pub manifest: Option<&'a Manifest>,
    /// Names the run here takes from the person's folder's env files
    /// (`values::here_values`): they have a value here.
    pub filled: &'a [String],
    /// Names iii cloud has a value for in this environment (the plan's variables with status `set`).
    pub cloud_set: &'a [String],
    /// `settings` of the environment (`GET .../environments/{e}`).
    pub settings: &'a Value,
    /// `GET /support-matrix`, when it answered.
    pub matrix: Option<&'a Value>,
    pub local_iii: Option<&'a str>,
    pub plan_status: PlanStatus,
    pub plan_workers: Option<&'a [(String, Option<String>)]>,
}

pub fn here(
    code: &str,
    title: String,
    detail: Option<String>,
    owner: Owner,
    file: Option<String>,
) -> Problem {
    Problem {
        skipped: false,
        variable: None,
        id: problem_id(Source::CheckHere, code, file.as_deref(), None, &title),
        source: Source::CheckHere,
        code: code.to_string(),
        cloud_code: None,
        severity: Severity::Block,
        owner,
        title,
        detail,
        file,
        line: None,
        excerpt: None,
        fix: crate::problems::fix_for(code, true),
    }
}

/// A name that reads like a database connection URL.
pub fn database_shaped(name: &str) -> bool {
    let n = name.to_uppercase();
    n.ends_with("DATABASE_URL")
        || n.ends_with("DB_URL")
        || (n.starts_with("DATABASE_") && n.ends_with("_URL"))
        || (n.starts_with("POSTGRES") && n.contains("URL"))
        || (n.starts_with("PG") && n.ends_with("URL"))
}

/// The concrete fix for a database read from the environment.
pub fn database_fix(manifest: &str, container: &str, name: &str, alias: Option<&str>) -> String {
    match alias {
        Some(a) => format!(
            "Set `{name}: ${{{}}}` on {container} in {manifest}: the `database` worker is already declared, and iii cloud creates a managed Postgres (pgvector included) when you confirm the deploy; the run here starts a local one. Never ask the person for a database URL.",
            database_binding(a)
        ),
        None => format!(
            "Add the `database` worker (package://database, `version: latest`) to {manifest} and set `{name}: ${{DATABASE_PRIMARY_URL}}` on {container}. iii cloud creates a managed Postgres (pgvector included) when you confirm the deploy; the run here starts a local one. Never ask the person for a database URL."
        ),
    }
}

/// The first `major.minor.patch` token of `iii --version` output.
pub fn parse_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_owned)
}

/// Numeric `(major, minor, patch)`; a missing part is 0, a suffix is cut.
pub fn version_tuple(v: &str) -> (u64, u64, u64) {
    let mut parts = v
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
                .unwrap_or(0)
        });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

pub fn accepts_cloud_block(iii_version: Option<&str>) -> bool {
    iii_version.is_some_and(|v| version_tuple(v) >= (0, 27, 0))
}

/// The resolved `iii-sdk` version of a worker directory, or the lowest
/// version of its declared range.
pub fn sdk_version(dir: &Path, root: &Path) -> Option<String> {
    for base in [dir, root] {
        if let Some(v) = lock_version(base) {
            return Some(v);
        }
    }
    // ponytail: the lowest version of the declared range, not what npm or pip would resolve.
    declared_version(dir)
}

fn lock_version(dir: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(dir.join("package-lock.json")) {
        let lock: Value = serde_json::from_str(&text).ok()?;
        let v = lock
            .pointer("/packages/node_modules~1iii-sdk/version")
            .or_else(|| {
                lock.get("packages")?
                    .get("node_modules/iii-sdk")?
                    .get("version")
            })
            .or_else(|| lock.pointer("/dependencies/iii-sdk/version"))
            .and_then(Value::as_str);
        if let Some(v) = v {
            return Some(v.to_string());
        }
    }
    for file in ["Cargo.lock", "uv.lock"] {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            if line.trim() == r#"name = "iii-sdk""#
                && let Some(v) = lines
                    .next()
                    .and_then(|l| l.trim().strip_prefix("version = "))
            {
                return Some(v.trim_matches('"').to_string());
            }
        }
    }
    None
}

fn lowest(range: &str) -> Option<String> {
    let start = range.find(|c: char| c.is_ascii_digit())?;
    let v: String = range[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (!v.is_empty()).then_some(v)
}

fn declared_version(dir: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(dir.join("package.json")) {
        let pkg: Value = serde_json::from_str(&text).ok()?;
        for section in ["dependencies", "devDependencies", "peerDependencies"] {
            if let Some(r) = pkg
                .get(section)
                .and_then(|d| d.get("iii-sdk"))
                .and_then(Value::as_str)
            {
                return lowest(r);
            }
        }
    }
    for file in ["pyproject.toml", "requirements.txt", "Cargo.toml"] {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        for line in text.lines() {
            let l = line.trim().trim_matches(|c| c == '"' || c == ',');
            if let Some(rest) = l.strip_prefix("iii-sdk") {
                if rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '-') {
                    continue;
                }
                return lowest(rest);
            }
        }
    }
    None
}

const MIGRATIONS: [&str; 7] = [
    "db:migrate",
    "migrate deploy",
    "migrate up",
    "alembic upgrade",
    "sqlx migrate run",
    "diesel migration run",
    "knex migrate",
];

/// A workflow file that runs a migration: `(file, command)`.
pub fn migration_in_ci(workspace: &Path) -> Option<(String, String)> {
    let dir = workspace.join(".github/workflows");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    for file in files {
        if !matches!(
            file.extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            if let Some(cmd) = MIGRATIONS.iter().find(|m| line.contains(*m)) {
                let name = file.file_name()?.to_string_lossy();
                return Some((format!(".github/workflows/{name}"), (*cmd).to_string()));
            }
        }
    }
    None
}

pub fn local_checks(i: &LocalInput) -> Vec<Problem> {
    let mut out = vec![];
    let engine = i
        .matrix
        .and_then(|m| m.get("engine"))
        .and_then(Value::as_str);
    if let Some(engine) = engine
        && i.local_iii != Some(engine)
    {
        let local = i.local_iii.unwrap_or("none");
        out.push(here(
            "LOCAL_ENGINE_VERSION",
            format!("iii {local} here, the cloud runs {engine}"),
            Some(
                "The run here uses the cloud's engine. 0.26 also refuses the `cloud:` block."
                    .into(),
            ),
            Owner::Person,
            None,
        ));
    }
    let root_dir = i
        .settings
        .get("root_directory")
        .and_then(Value::as_str)
        .unwrap_or("");
    let root = i.workspace.join(root_dir);
    if let Some(m) = i.manifest {
        for name in m.fake_http_containers() {
            out.push(here(
                "HTTP_WORKER_NOT_PACKAGE",
                format!("The worker {name} is not package://http"),
                Some("iii cloud routes traffic only to `package://http`; this path worker would serve nothing in the cloud. Use `package://http` with a pinned version in the cloud manifest.".into()),
                Owner::Agent,
                None,
            ));
        }
        let min = i
            .matrix
            .and_then(|m| m.get("minimum_sdk"))
            .and_then(Value::as_str);
        let mut seen_runtimes: Vec<String> = vec![];
        for (name, container) in m.containers() {
            let Some(dir) = m.path_worker_dir(container) else {
                continue;
            };
            if let (Some(min), Some(v)) = (min, sdk_version(&dir, &root))
                && version_tuple(&v) < version_tuple(min)
            {
                out.push(here(
                    "SDK_TOO_OLD",
                    format!("iii-sdk {v} is older than {min}"),
                    Some(
                        "The deploy would go green and the http group would serve no route.".into(),
                    ),
                    Owner::Agent,
                    Some(rel(i.workspace, &dir)),
                ));
            }
            // The commands that run in this worker's image: its run script, and the release
            // command when the manifest ties it to this worker. (`cloud.build` is one command
            // for the whole project, not tied to a worker's image: not read here.)
            let release = m.cloud_release().filter(|r| r.worker == name);
            let commands = [
                container
                    .pointer("/scripts/run")
                    .and_then(Value::as_str)
                    .map(|c| ("run", c)),
                release.as_ref().map(|r| ("release", r.command.as_str())),
            ];
            // A command that prints every variable puts the secrets in the log.
            for (what, command) in commands.into_iter().flatten() {
                if let Some(part) = prints_environment(command) {
                    out.push(here(
                        "COMMAND_PRINTS_ENVIRONMENT",
                        format!("{name}'s {what} command runs `{part}`"),
                        Some(format!(
                            "This command prints every variable, secrets included, into the log. Its {what} command is `{command}`. Take it out; read the whole log of the run with `iii-cloud::check::read` instead."
                        )),
                        Owner::Agent,
                        Some(rel(i.workspace, &dir)),
                    ));
                }
            }
            let Some(runtime) = worker_runtime(m, &name, &dir, i.plan_workers) else {
                continue;
            };
            let base = runtime_base(&runtime).to_string();
            // Only a runtime the cloud itself declares: not the folder's language guess.
            let declared =
                declared_runtime(m, &name, i.plan_workers).map(|r| runtime_base(&r).to_string());
            for (what, command) in commands.into_iter().flatten() {
                if declared.as_deref() != Some(base.as_str()) {
                    break;
                }
                for tool in tools_missing_in_image(&base, command) {
                    out.push(here(
                        "RUN_COMMAND_NOT_IN_IMAGE",
                        format!("{name} runs with {tool}, but its runtime is {base}"),
                        Some(format!(
                            "The cloud's {base} image has no {tool}: the worker would exit as it starts (127). Its {what} command is `{command}`."
                        )),
                        Owner::Agent,
                        Some(rel(i.workspace, &dir)),
                    ));
                }
            }
            if seen_runtimes.contains(&base) || accepted_locks(&base).is_empty() {
                continue;
            }
            seen_runtimes.push(base.clone());
            if lock_for(&base, &root).is_none() {
                let found = Lock::ALL
                    .iter()
                    .find(|l| l.present(&root))
                    .map(|l| l.file());
                out.push(here(
                    "RUNTIME_LOCK_MISSING",
                    match found {
                        Some(f) => {
                            format!("Runs with {base}, but the repository has only a {f} lock")
                        }
                        None => format!("Runs with {base}, but the repository has no lock"),
                    },
                    Some(format!(
                        "The cloud installs {base} projects from {}: nothing would be installed.",
                        accepted_locks(&base)
                            .iter()
                            .map(|l| l.file())
                            .collect::<Vec<_>>()
                            .join(" or ")
                    )),
                    Owner::Person,
                    None,
                ));
            }
        }
        if let Some(command) = m.cloud_build()
            && let Some(part) = prints_environment(&command)
        {
            out.push(here(
                "COMMAND_PRINTS_ENVIRONMENT",
                format!("The build command runs `{part}`"),
                Some(format!(
                    "This command prints every variable, secrets included, into the log. The build command is `{command}`. Take it out; read the whole log of the run with `iii-cloud::check::read` instead."
                )),
                Owner::Agent,
                None,
            ));
        }
        let has_release =
            m.cloud_release().is_some() || !i.settings.get("release").is_none_or(Value::is_null);
        if !has_release && let Some((file, cmd)) = migration_in_ci(i.workspace) {
            out.push(here(
                "RELEASE_IN_CI",
                "Migrations run in your CD, not in the cloud".into(),
                Some(format!("{file} runs {cmd}; it moves to cloud.release.")),
                Owner::Agent,
                Some(file),
            ));
        }
        let has_health = m.cloud_healthcheck().is_some()
            || !i.settings.get("healthcheck").is_none_or(Value::is_null);
        if !has_health {
            out.push(here(
                "HEALTHCHECK_MISSING",
                "No healthcheck".into(),
                Some("Without one, a deploy is Ready even when nothing answers.".into()),
                Owner::Agent,
                None,
            ));
        }
        // `${NAME}` with no default and no value here (the run's `.env`, or
        // the folder's env files it fills from), outside resource bindings.
        let mut env: Vec<String> = worktree_env(i.workspace, m)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        env.extend(i.filled.iter().cloned());
        let bindings = m.database_bindings();
        let first_alias = m
            .database_aliases()
            .into_iter()
            .next()
            .and_then(|(_, a)| a.into_iter().next());
        let (uses, unsupported) = m.variable_uses();
        let manifest = rel(i.workspace, &m.file);
        let mut seen: Vec<&str> = vec![];
        for u in &uses {
            if seen.contains(&u.name.as_str()) || bindings.contains(&u.name) {
                continue;
            }
            let name = u.name.as_str();
            let of_name = || uses.iter().filter(move |x| x.name == name);
            // A database URL is never the person's to give: iii cloud makes the
            // database. `${NAME:-}` reads one from the environment too.
            if database_shaped(name)
                && let Some(read) =
                    of_name().find(|x| x.default.as_deref().is_none_or(str::is_empty))
            {
                seen.push(name);
                let container = read.container.as_str();
                let mut problem = here(
                    "DATABASE_NOT_MANAGED",
                    format!("{name} is not a managed database"),
                    Some(format!(
                        "{container} reads {name} from the environment; iii cloud can create this database."
                    )),
                    Owner::Agent,
                    None,
                );
                problem.fix = Some(database_fix(
                    &manifest,
                    container,
                    name,
                    first_alias.as_deref(),
                ));
                out.push(problem);
                continue;
            }
            // `${NAME:-}` that iii cloud has a value for: the cloud gives it that value, so the run needs it too.
            if of_name().any(|x| x.default.is_none())
                || (i.cloud_set.iter().any(|n| n == name)
                    && empty_default_variables(m).iter().any(|n| n == name))
            {
                seen.push(name);
                if !env.iter().any(|e| e == name) {
                    // Only an empty default that iii cloud has a value for is needed for that reason.
                    let because_cloud_has_it = of_name().all(|x| x.default.is_some());
                    let mut unset = here(
                        "LOCAL_VARIABLE_UNSET",
                        format!("{name} has no value here"),
                        Some(if because_cloud_has_it {
                            "iii cloud has a value for it, so the run here needs one too: add it to an env file of your folder, or skip it to run it empty."
                                .into()
                        } else {
                            "The run here needs it: it is not in the run's .env, nor in your folder's .env files."
                                .into()
                        }),
                        Owner::Person,
                        None,
                    );
                    unset.variable = Some(name.to_string());
                    out.push(unset);
                }
            }
        }
        // `${A:?}`, `${A?}`, `${A-x}` reach the container as the text itself.
        let mut said: Vec<&str> = vec![];
        for u in &unsupported {
            if said.contains(&u.written.as_str()) {
                continue;
            }
            said.push(&u.written);
            let mut problem = here(
                "VARIABLE_SYNTAX_UNSUPPORTED",
                format!("{} reaches {} as text", u.written, u.container),
                Some(
                    "iii cloud reads `${NAME}` and `${NAME:-default}` (`${NAME:default}` in config_override); this form is delivered as written."
                        .into(),
                ),
                Owner::Agent,
                Some(manifest.clone()),
            );
            problem.line = u.line;
            out.push(problem);
        }
        if !matches!(i.plan_status, PlanStatus::Clean | PlanStatus::Blocked)
            && !m.lock_path().exists()
        {
            let lock = rel(i.workspace, &m.lock_path());
            out.push(here(
                "LOCK_MISSING",
                "Lock missing".into(),
                Some(lock.clone()),
                Owner::Agent,
                Some(lock),
            ));
        }
    }
    out
}

fn rel(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(text: &str) -> Manifest {
        Manifest {
            file: PathBuf::from("/r/worker-compose.yaml"),
            dir: PathBuf::from("/r"),
            text: text.to_string(),
            yaml: serde_yaml_ng::from_str(text).unwrap(),
        }
    }

    #[test]
    fn the_walker_reads_references_as_iii_cloud_does() {
        let m = manifest(concat!(
            "containers:\n",
            "  api:\n",
            "    env:\n",
            "      A: ${A}\n",
            "      B: ${B:-}\n",
            "      C: \"${C:-x} and $${D}\"\n",
            "    # E: ${E}\n",
            "    args: [\"--f=${F}\"]\n",
            "    config_override:\n",
            "      g: ${G:on}\n",
            "  web:\n",
            "    env:\n",
            "      A: ${A:-y}\n",
            "other:\n",
            "  H: ${H}\n",
        ));
        let (uses, bad) = m.variable_uses();
        let got: Vec<(&str, Option<&str>, &str, Option<u32>)> = uses
            .iter()
            .map(|u| {
                (
                    u.name.as_str(),
                    u.default.as_deref(),
                    u.container.as_str(),
                    u.line,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("A", None, "api", Some(4)),
                ("B", Some(""), "api", Some(5)),
                ("C", Some("x"), "api", Some(6)),
                ("F", None, "api", Some(8)),
                ("G", Some("on"), "api", Some(10)),
                ("A", Some("y"), "web", Some(13)),
            ],
            "a comment, `$$` and a reference outside containers are not read"
        );
        assert!(bad.is_empty());
        assert_eq!(required_variables(&m), vec!["A", "F"]);
    }

    #[test]
    fn forms_iii_cloud_delivers_as_text_are_unsupported_not_references() {
        let m = manifest(concat!(
            "containers:\n",
            "  api:\n",
            "    env:\n",
            "      A: ${A:?}\n",
            "      B: ${B?}\n",
            "      C: ${C-x}\n",
            "      D: ${D}\n",
        ));
        let (uses, bad) = m.variable_uses();
        assert_eq!(
            uses.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
            vec!["D"]
        );
        assert_eq!(
            bad.iter()
                .map(|b| (b.written.as_str(), b.container.as_str(), b.line))
                .collect::<Vec<_>>(),
            vec![
                ("${A:?}", "api", Some(4)),
                ("${B?}", "api", Some(5)),
                ("${C-x}", "api", Some(6)),
            ]
        );
        let problems = local_checks(&LocalInput {
            workspace: Path::new("/r"),
            manifest: Some(&m),
            filled: &[],
            cloud_set: &[],
            settings: &serde_json::json!({}),
            matrix: None,
            local_iii: None,
            plan_status: PlanStatus::Clean,
            plan_workers: None,
        });
        let syntax: Vec<&Problem> = problems
            .iter()
            .filter(|p| p.code == "VARIABLE_SYNTAX_UNSUPPORTED")
            .collect();
        assert_eq!(syntax.len(), 3);
        assert_eq!(syntax[0].title, "${A:?} reaches api as text");
        assert_eq!(
            (syntax[0].owner, syntax[0].severity, syntax[0].line),
            (Owner::Agent, Severity::Block, Some(4))
        );
        assert!(
            syntax[0]
                .fix
                .as_deref()
                .is_some_and(|f| f.contains("literal text"))
        );
    }

    /// What `crates/plan/src/variables.rs` does with the same text: a nested
    /// default ends at the first `}`, `<<` is a key like any other (its alias
    /// expanded), only a `config_override` field of the container itself reads
    /// `${A:x}`, and the sandbox adapter is left out.
    #[test]
    fn the_walker_matches_the_cloud_on_nesting_merge_keys_and_the_adapter() {
        let m = manifest(concat!(
            "common: &common\n",
            "  C: ${C:c}\n",
            "  D: ${D:-d}\n",
            "containers:\n",
            "  api:\n",
            "    environment:\n",
            "      A: ${A:-${B}}\n",
            "      <<: *common\n",
            "    config_override:\n",
            "      <<: *common\n",
            "  web:\n",
            "    <<:\n",
            "      config_override:\n",
            "        E: ${E:e}\n",
            "  vm:\n",
            "    worker: package://sandbox-libkrun\n",
            "    environment:\n",
            "      F: ${F}\n",
            "  vm2:\n",
            "    worker: package://api.workers.iii.dev/sandbox-libkrun\n",
            "    environment:\n",
            "      G: ${G}\n",
            "  vm3:\n",
            "    worker: package://sandbox-libkrun@1.0.0\n",
            "    environment:\n",
            "      H: ${H}\n",
        ));
        let got: Vec<(String, Option<String>, String)> = m
            .variable_uses()
            .0
            .into_iter()
            .map(|u| (u.name, u.default, u.container))
            .collect();
        let row = |n: &str, d: Option<&str>, c: &str| {
            (n.to_string(), d.map(str::to_string), c.to_string())
        };
        assert_eq!(
            got,
            vec![
                row("A", Some("${B"), "api"),
                row("D", Some("d"), "api"),
                row("C", Some("c"), "api"),
                row("D", Some("d"), "api"),
                row("H", None, "vm3"),
            ]
        );
    }

    /// The fixture the page's `parseEnv` port is tested on too.
    #[test]
    fn parse_env_reads_a_dotenv_as_the_console_does() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/env-parse.json")).unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let expected: Vec<(String, String)> = case["expected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| {
                    (
                        e["name"].as_str().unwrap().to_string(),
                        e["value"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            assert_eq!(parse_env(text), expected, "{text:?}");
        }
    }
}
