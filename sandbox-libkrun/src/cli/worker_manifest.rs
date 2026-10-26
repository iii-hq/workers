// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! Typed `iii.worker.yaml` manifest.
//!
//! Single source of truth for the manifest schema: the [`WorkerManifest`]
//! struct drives the unknown-key check of `project::validate_manifest_keys`.
//!
//! Design notes:
//! - Deprecated fields are REAL typed fields (so readers stay clean and they
//!   are still accepted) while every section carries a `#[serde(flatten)]`
//!   catch-all map — non-empty catch-all = unknown keys = hard error. This
//!   keeps deprecated keys accepted, which `#[serde(deny_unknown_fields)]`
//!   cannot express.
//! - Parsing goes Value-first then `from_value`: serde_yaml 0.9 detects
//!   duplicate mapping keys only at the `Value` layer (typed maps silently
//!   last-win), so the two-step keeps duplicate keys a hard error.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_yaml::Value as YamlValue;

/// Strip control characters from a manifest key before it is echoed into a
/// terminal-bound error or warning. Manifest keys are third-party (a cloned
/// worker's `iii.worker.yaml`), so an unknown key like `"\x1b]2;…\x07"`
/// must not reach the user's terminal raw. Mirrors the established
/// terminal-sanitization pattern in `engine/src/cli_trigger/help.rs`.
/// `is_control` covers only Cc, so the invisible Cf-category spoofing chars
/// (bidi overrides, zero-widths) are filtered explicitly — U+202E alone can
/// visually reverse an echoed key and disguise what the error points at.
pub(crate) fn sanitize_key(s: &str) -> String {
    fn is_invisible_format_char(c: char) -> bool {
        matches!(c,
            '\u{200b}'..='\u{200f}' // zero-widths + LRM/RLM
            | '\u{202a}'..='\u{202e}' // bidi embeddings/overrides
            | '\u{2066}'..='\u{2069}' // bidi isolates
            | '\u{061c}' // Arabic letter mark
            | '\u{feff}' // BOM / zero-width no-break space
        )
    }
    s.chars()
        .filter(|c| !c.is_control() && !is_invisible_format_char(*c))
        .collect()
}

// Every field defines an accepted, type-checked manifest key; not all are read.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkerManifest {
    /// Required. Worker id; a single path segment (alphanumerics, '-', '_', '.').
    pub name: Option<String>,

    pub description: Option<String>,

    pub runtime: Option<RuntimeSection>,

    pub scripts: Option<ScriptsSection>,

    pub env: Option<BTreeMap<String, String>>,

    pub dependencies: Option<BTreeMap<String, String>>,

    pub resources: Option<ResourcesSection>,

    // Registry publish metadata. The workers-repo release CI requires these
    // keys (language and deploy/manifest pick the release artifact type and
    // version source; tags provide discovery aliases; config is the
    // published default config); they are accepted so one iii.worker.yaml
    // can satisfy both validators, but never read at start.
    pub iii: Option<String>,

    pub deploy: Option<String>,

    pub manifest: Option<String>,

    pub tags: Option<Vec<String>>,

    pub license: Option<String>,

    pub config: Option<YamlValue>,

    pub language: Option<String>,

    #[serde(flatten)]
    pub unknown: BTreeMap<String, YamlValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RuntimeSection {
    pub base_image: Option<String>,

    pub kind: Option<String>,

    pub package_manager: Option<String>,

    pub entry: Option<String>,

    pub language: Option<String>,

    #[serde(flatten)]
    pub unknown: BTreeMap<String, YamlValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScriptsSection {
    pub setup: Option<String>,

    pub install: Option<String>,

    pub start: Option<String>,

    #[serde(flatten)]
    pub unknown: BTreeMap<String, YamlValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ResourcesSection {
    pub cpus: Option<u32>,

    pub memory: Option<u32>,

    #[serde(flatten)]
    pub unknown: BTreeMap<String, YamlValue>,
}

/// Human-readable YAML type label for shape-error messages. Numbers echo
/// their value (safe — serde_yaml renders them numerically); everything else
/// gets a type name so hostile string content never reaches the terminal.
fn yaml_value_desc(v: &YamlValue) -> String {
    match v {
        YamlValue::Null => "null".into(),
        YamlValue::Bool(b) => format!("the boolean {b}"),
        YamlValue::Number(n) => format!("the number {n}"),
        YamlValue::String(_) => "a string".into(),
        YamlValue::Sequence(_) => "a list".into(),
        YamlValue::Mapping(_) => "a mapping".into(),
        YamlValue::Tagged(_) => "a tagged value".into(),
    }
}

/// Treat YAML `null` like an absent field (serde maps it to `None`), so the
/// shape checks below only fire on values serde would actually reject.
fn present(v: Option<&YamlValue>) -> Option<&YamlValue> {
    v.filter(|v| !v.is_null())
}

/// Value-layer shape checks for every documented field, collected with dotted
/// paths so type mistakes read as "`runtime` must be a mapping ..., got a
/// string" instead of serde's "invalid type: string \"node\", expected struct
/// RuntimeSection" (Rust internals were leaking into user errors — caught by
/// the live DX audit). Must stay consistent with the typed struct: flag only
/// what serde would reject anyway; serde remains the backstop for anything
/// not covered here.
fn shape_problems(doc: &YamlValue) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(map) = doc.as_mapping() else {
        return problems;
    };

    for key in map.keys() {
        if key.as_str().is_none() {
            problems.push(format!(
                "manifest keys must be strings, got {}",
                yaml_value_desc(key)
            ));
        }
    }

    for field in [
        "name",
        "description",
        "language",
        "iii",
        "deploy",
        "manifest",
        "license",
    ] {
        if let Some(v) = present(doc.get(field))
            && v.as_str().is_none()
        {
            problems.push(format!(
                "`{field}` must be a string, got {}",
                yaml_value_desc(v)
            ));
        }
    }

    for section in ["runtime", "scripts", "resources"] {
        if let Some(v) = present(doc.get(section))
            && v.as_mapping().is_none()
        {
            problems.push(format!(
                "`{section}` must be a mapping of indented `key: value` fields, got {}",
                yaml_value_desc(v)
            ));
        }
    }

    if let Some(tags) = present(doc.get("tags")) {
        if let Some(entries) = tags.as_sequence() {
            for (index, value) in entries.iter().enumerate() {
                if value.as_str().is_none() {
                    problems.push(format!(
                        "`tags[{index}]` must be a string, got {}",
                        yaml_value_desc(value)
                    ));
                }
            }
        } else {
            problems.push(format!(
                "`tags` must be a list of strings, got {}",
                yaml_value_desc(tags)
            ));
        }
    }

    if let Some(rt) = doc.get("runtime").and_then(YamlValue::as_mapping) {
        for field in ["base_image", "kind", "package_manager", "entry", "language"] {
            if let Some(v) = present(rt.get(field))
                && v.as_str().is_none()
            {
                problems.push(format!(
                    "`runtime.{field}` must be a string, got {}",
                    yaml_value_desc(v)
                ));
            }
        }
    }

    if let Some(scripts) = doc.get("scripts").and_then(YamlValue::as_mapping) {
        for field in ["setup", "install", "start"] {
            if let Some(v) = present(scripts.get(field))
                && v.as_str().is_none()
            {
                let hint = if v.is_sequence() {
                    " — use one command string, chaining with `&&` if needed"
                } else {
                    ""
                };
                problems.push(format!(
                    "`scripts.{field}` must be a command string, got {}{hint}",
                    yaml_value_desc(v)
                ));
            }
        }
    }

    if let Some(res) = doc.get("resources").and_then(YamlValue::as_mapping) {
        for field in ["cpus", "memory"] {
            if let Some(v) = present(res.get(field)) {
                match v.as_u64() {
                    Some(n) if n <= u64::from(u32::MAX) => {}
                    // A non-negative integer that's just too big needs the
                    // bound named, not a self-contradictory "must be a
                    // non-negative integer, got 4294967296".
                    Some(n) => problems.push(format!(
                        "`resources.{field}` must be at most {}, got {n}",
                        u32::MAX
                    )),
                    None => problems.push(format!(
                        "`resources.{field}` must be a non-negative integer, got {}",
                        yaml_value_desc(v)
                    )),
                }
            }
        }
    }

    // env/dependencies are opaque maps key-wise, but their VALUES must be
    // strings — the classic trap is `PORT: 8080` (a YAML number), so the
    // message says how to quote it.
    for section in ["env", "dependencies"] {
        let Some(v) = present(doc.get(section)) else {
            continue;
        };
        let Some(entries) = v.as_mapping() else {
            problems.push(format!(
                "`{section}` must be a mapping of string values, got {}",
                yaml_value_desc(v)
            ));
            continue;
        };
        for (key, val) in entries {
            let Some(key_name) = key.as_str().map(sanitize_key) else {
                problems.push(format!(
                    "`{section}` keys must be strings, got {}",
                    yaml_value_desc(key)
                ));
                continue;
            };
            if val.as_str().is_none() {
                let hint = match val {
                    YamlValue::Null => format!(" — write `{key_name}: \"\"` for an empty value"),
                    YamlValue::Bool(_) | YamlValue::Number(_) => {
                        format!(" — quote it (`{key_name}: \"...\"`)")
                    }
                    _ => String::new(),
                };
                problems.push(format!(
                    "`{section}.{key_name}` must be a string, got {}{hint}",
                    yaml_value_desc(val)
                ));
            }
        }
    }

    problems
}

impl WorkerManifest {
    /// Deserialize from an already-parsed YAML document. Returns `Err` with a
    /// human-readable message when the document's shape doesn't match (e.g.
    /// `runtime: node` where a mapping is required, or `resources.cpus: "four"`).
    /// Friendly [`shape_problems`] run first so the common type mistakes get
    /// dotted-path messages; raw serde errors are the backstop only.
    pub fn from_value(doc: &YamlValue) -> Result<Self, String> {
        let problems = shape_problems(doc);
        if !problems.is_empty() {
            return Err(format!("invalid manifest shape: {}", problems.join("; ")));
        }
        serde_yaml::from_value(doc.clone()).map_err(|e| format!("invalid manifest shape: {e}"))
    }

    /// Every unknown manifest key as a dotted path, sanitized for terminal
    /// display and sorted.
    pub fn unknown_keys(&self) -> Vec<String> {
        let mut unknown: Vec<String> = self.unknown.keys().map(|k| sanitize_key(k)).collect();
        if let Some(rt) = &self.runtime {
            for k in rt.unknown.keys() {
                unknown.push(format!("runtime.{}", sanitize_key(k)));
            }
        }
        if let Some(s) = &self.scripts {
            for k in s.unknown.keys() {
                unknown.push(format!("scripts.{}", sanitize_key(k)));
            }
        }
        if let Some(r) = &self.resources {
            for k in r.unknown.keys() {
                unknown.push(format!("resources.{}", sanitize_key(k)));
            }
        }
        unknown.sort();
        unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_value_rejects_non_mapping_document_via_serde_backstop() {
        let doc: YamlValue = serde_yaml::from_str("just-a-string").unwrap();
        let err = WorkerManifest::from_value(&doc).expect_err("non-mapping must fail");
        assert!(err.starts_with("invalid manifest shape:"), "got: {err}");
    }

    #[test]
    fn unknown_keys_skip_the_accepted_deprecated_ones() {
        let doc: YamlValue = serde_yaml::from_str(
            "name: w\nruntime:\n  kind: bun\n  foo: 1\nconfig: {}\nbad: 2\nentry: w.ts\n",
        )
        .unwrap();
        let m = WorkerManifest::from_value(&doc).unwrap();
        // Top-level `entry` was the iii-worker spelling of `runtime.entry`.
        assert_eq!(m.unknown_keys(), vec!["bad", "entry", "runtime.foo"]);
    }
}
