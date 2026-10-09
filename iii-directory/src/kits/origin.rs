//! Where an agent profile came from.
//!
//! An `agents/<id>.md` in the project root is owned by, in order:
//!
//! 1. a kit, when `kits.lock` lists the path (and the entry is not
//!    `skipped`);
//! 2. a worker, when that worker's skills marker
//!    (`<skills_folder>/<worker>/.iii-skill-complete`) records the agent it
//!    wrote with its sha256;
//! 3. otherwise the project itself (`local`): created by hand or by a
//!    template.
//!
//! A profile served from the user-global root is `global`; one served from
//! the worker binary is `builtin`. `modified` compares the file with the
//! content its owner wrote.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::lock::{local_sha, KitsLock};
use crate::functions::download::{read_worker_markers, WorkerMarker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Kit,
    Worker,
    Local,
    Global,
    Builtin,
}

/// `source` on `directory::agents::list` / `get` rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentSource {
    pub kind: SourceKind,
    /// Owning kit (`<handle>/<name>`) when `kind` is `kit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    /// Owning worker when `kind` is `worker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// Version of the kit or worker that wrote the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The file differs from what its kit or worker wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<bool>,
}

impl AgentSource {
    pub fn simple(kind: SourceKind) -> Self {
        Self {
            kind,
            kit: None,
            worker: None,
            version: None,
            modified: None,
        }
    }
}

/// Everything needed to classify agent origins, read once per call.
pub struct OriginIndex {
    pub lock: KitsLock,
    /// Agent id → (worker marker, recorded sha).
    worker_agents: BTreeMap<String, (WorkerMarker, String)>,
}

impl OriginIndex {
    pub fn load(lock: KitsLock, skills_folder: &Path) -> Self {
        let mut worker_agents = BTreeMap::new();
        for marker in read_worker_markers(skills_folder) {
            for (id, sha) in &marker.agents {
                worker_agents.insert(id.clone(), (marker.clone(), sha.clone()));
            }
        }
        Self {
            lock,
            worker_agents,
        }
    }

    /// The worker whose marker records agent `id`, with its recorded sha.
    pub fn worker_for_agent(&self, id: &str) -> Option<(&WorkerMarker, &str)> {
        self.worker_agents
            .get(id)
            .map(|(marker, sha)| (marker, sha.as_str()))
    }

    /// Classify a project-root agent file (`abs` is its current location).
    pub fn classify_project_agent(&self, id: &str, abs: &Path) -> AgentSource {
        let path = format!("agents/{id}.md");
        let local = local_sha(abs);
        if let Some((kit, locked)) = self.lock.owner_of(&path) {
            let base = locked.files.get(&path).map(|f| f.sha256.as_str());
            return AgentSource {
                kind: SourceKind::Kit,
                kit: Some(kit.to_string()),
                worker: None,
                version: Some(locked.version.clone()),
                modified: Some(local.as_deref() != base),
            };
        }
        if let Some((marker, sha)) = self.worker_for_agent(id) {
            return AgentSource {
                kind: SourceKind::Worker,
                kit: None,
                worker: Some(marker.worker.clone()),
                version: marker.version.clone(),
                modified: Some(local.as_deref() != Some(sha)),
            };
        }
        AgentSource::simple(SourceKind::Local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kits::lock::{sha256_hex, LockedFile, LockedKit};

    #[test]
    fn kit_beats_worker_beats_local() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = tmp.path().join("skills");
        let agents = tmp.path().join("agents");
        std::fs::create_dir_all(skills.join("kanban")).unwrap();
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("reviewer.md"), "w").unwrap();
        std::fs::write(agents.join("planner.md"), "k").unwrap();
        std::fs::write(agents.join("mine.md"), "m").unwrap();
        std::fs::write(
            skills.join("kanban/.iii-skill-complete"),
            serde_json::json!({
                "worker": "kanban", "source": "registry", "tag_or_version": "latest",
                "schema": 2, "version": "1.6.1",
                "agents": { "reviewer": sha256_hex(b"w"), "planner": sha256_hex(b"old") }
            })
            .to_string(),
        )
        .unwrap();
        let mut lock = KitsLock::default();
        lock.kits.insert(
            "acme/team".into(),
            LockedKit {
                requested: "latest".into(),
                version: "1.0.0".into(),
                installed_at: String::new(),
                workers: Default::default(),
                files: [(
                    "agents/planner.md".to_string(),
                    LockedFile {
                        sha256: sha256_hex(b"k"),
                        skipped: false,
                    },
                )]
                .into(),
                ignored_versions: vec![],
            },
        );
        let index = OriginIndex::load(lock, &skills);

        let planner = index.classify_project_agent("planner", &agents.join("planner.md"));
        assert_eq!(planner.kind, SourceKind::Kit);
        assert_eq!(planner.kit.as_deref(), Some("acme/team"));
        assert_eq!(planner.modified, Some(false));

        let reviewer = index.classify_project_agent("reviewer", &agents.join("reviewer.md"));
        assert_eq!(reviewer.kind, SourceKind::Worker);
        assert_eq!(reviewer.worker.as_deref(), Some("kanban"));
        assert_eq!(reviewer.version.as_deref(), Some("1.6.1"));
        assert_eq!(reviewer.modified, Some(false));

        let mine = index.classify_project_agent("mine", &agents.join("mine.md"));
        assert_eq!(mine.kind, SourceKind::Local);
    }
}
