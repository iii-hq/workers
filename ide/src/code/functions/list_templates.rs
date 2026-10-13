//! `coder::list-templates` — the worker templates `coder::scaffold-worker`
//! creates from, read from `code.templates`: a local dir, or a cached
//! shallow git clone (see `crate::code::templates`). Read-only.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::templates::{
    load_worker_templates, resolve_source, Language, ResolvedSource, TemplateSourceInfo,
};

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_list_templates_input")]
pub struct ListTemplatesInput {
    /// Re-fetch the cached git clone now instead of after
    /// `code.templates.refresh_secs`. No effect on a local dir.
    #[serde(default)]
    pub refresh: bool,
}

// examples are wire-contract; goldens pin them.
fn example_list_templates_input() -> serde_json::Value {
    serde_json::json!({ "refresh": true })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListTemplatesOutput {
    /// Where the templates were read from.
    pub source: TemplateSourceInfo,
    pub templates: Vec<TemplateInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateInfo {
    /// Pass as `template` to coder::scaffold-worker.
    pub id: String,
    pub name: String,
    pub description: String,
    pub language: Language,
    /// Compose containers the worker needs, e.g. `http`.
    pub requires: Vec<String>,
}

pub async fn handle(
    cfg: Arc<CoderConfig>,
    req: ListTemplatesInput,
) -> Result<ListTemplatesOutput, String> {
    // III_TEMPLATE_DIR / III_TEMPLATE_URL win over the config (spec §3.2).
    let source = resolve_source(&cfg.templates.clone().with_env(), req.refresh)
        .await
        .map_err(err_to_string)?;
    listing(source).map_err(err_to_string)
}

/// The listing for a resolved source. Split from `handle` so tests run on
/// a fixture without git or the `III_TEMPLATE_*` environment.
fn listing(source: ResolvedSource) -> Result<ListTemplatesOutput, CoderError> {
    let (templates, warnings) = load_worker_templates(&source.root)?;
    for warning in warnings {
        tracing::warn!(%warning, "coder::list-templates left out an invalid template");
    }
    Ok(ListTemplatesOutput {
        source: source.info,
        templates: templates
            .into_iter()
            .map(|t| TemplateInfo {
                id: t.id,
                name: t.name,
                description: t.description,
                language: t.language,
                requires: t.worker.requires,
            })
            .collect(),
    })
}

/// A tiny templates root for tests: `bare` has no `worker:` block;
/// `worker-node-ade` has text and executable files plus project files
/// outside `worker.dir`; `collide` lists two files that land on one path
/// when the worker is named "a".
#[cfg(test)]
pub(crate) mod fixture {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    const NODE_ADE: &str = "\
name: Worker (Node, ADE)
description: Node worker with an ADE page.
requires: [typescript]
files:
  - README.md
  - worker-compose.yaml
  - workers/my-worker/package.json
  - workers/my-worker/src/index.ts
  - workers/my-worker/scripts/dev.sh
next_steps:
  - 'Call: iii trigger my-worker::hello'
worker:
  dir: workers/my-worker
  name: my-worker
  compose: my-worker
  requires: [http]
";

    const NODE_ADE_COMPOSE: &str = "\
containers:
  my-worker:
    worker: path://./workers/my-worker
    scripts:
      pre_run: pnpm install
      run: pnpm dev
    environment:
      WORKER_NAME: my-worker
    start_after: [console]
  http:
    worker: package://http
";

    const COLLIDE: &str = "\
name: Collide
description: Two files that land on one path for the name a.
requires: [typescript]
files:
  - workers/my-worker/a.txt
  - workers/my-worker/my-worker.txt
next_steps: []
worker:
  dir: workers/my-worker
  name: my-worker
  compose: my-worker
";

    pub(crate) fn write(root: &Path) {
        let text: &[(&str, &str)] = &[
            (
                "template.yaml",
                "templates:\n  - bare\n  - worker-node-ade\n  - collide\n",
            ),
            (
                "bare/template.yaml",
                "name: Bare\ndescription: No worker block.\nrequires: [typescript]\nfiles: [README.md]\n",
            ),
            ("bare/README.md", "# bare\n"),
            ("worker-node-ade/template.yaml", NODE_ADE),
            ("worker-node-ade/worker-compose.yaml", NODE_ADE_COMPOSE),
            ("worker-node-ade/README.md", "# project\n"),
            (
                "worker-node-ade/workers/my-worker/package.json",
                "{\"name\": \"my-worker\"}\n",
            ),
            (
                "worker-node-ade/workers/my-worker/src/index.ts",
                "export const id = 'my-worker::hello'\n",
            ),
            (
                "worker-node-ade/workers/my-worker/scripts/dev.sh",
                "#!/bin/sh\necho my-worker\n",
            ),
            ("collide/template.yaml", COLLIDE),
            (
                "collide/worker-compose.yaml",
                "containers:\n  my-worker:\n    worker: path://./workers/my-worker\n",
            ),
            ("collide/workers/my-worker/a.txt", "a\n"),
            ("collide/workers/my-worker/my-worker.txt", "b\n"),
        ];
        for (path, body) in text {
            put(&root.join(path), body.as_bytes());
        }
        std::fs::set_permissions(
            root.join("worker-node-ade/workers/my-worker/scripts/dev.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_returns_the_scaffoldable_templates_and_the_source() {
        let tmp = tempfile::tempdir().unwrap();
        fixture::write(tmp.path());
        let location = tmp.path().display().to_string();
        let out = listing(ResolvedSource {
            info: TemplateSourceInfo {
                kind: "dir".into(),
                location: location.clone(),
                git_ref: None,
                revision: None,
                warning: None,
            },
            root: tmp.path().to_path_buf(),
        })
        .unwrap();

        let wire = serde_json::to_value(&out).unwrap();
        assert_eq!(wire["source"]["kind"], "dir");
        assert_eq!(wire["source"]["location"], location);
        assert!(
            wire["source"].get("ref").is_none(),
            "no ref for a dir: {wire}"
        );

        let mut ids: Vec<&str> = out.templates.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            ["collide", "worker-node-ade"],
            "bare has no worker: block"
        );
        let ade = wire["templates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "worker-node-ade")
            .unwrap();
        assert_eq!(
            ade,
            &serde_json::json!({
                "id": "worker-node-ade",
                "name": "Worker (Node, ADE)",
                "description": "Node worker with an ADE page.",
                "language": "node",
                "requires": ["http"]
            })
        );
    }
}
