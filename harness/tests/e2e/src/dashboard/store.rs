use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};

use super::{JobStatus, RunMetadata, LOCAL_SCHEMA_VERSION};
use crate::report::E2eReport;

pub(super) struct StoredRun {
    pub(super) metadata: RunMetadata,
    pub(super) report: Option<E2eReport>,
}

pub(super) fn write_metadata(run_dir: &Path, metadata: &RunMetadata) -> Result<()> {
    fs::create_dir_all(run_dir).with_context(|| format!("create {}", run_dir.display()))?;
    let target = run_dir.join("metadata.json");
    let temporary = run_dir.join("metadata.json.tmp");
    let mut bytes = serde_json::to_vec_pretty(metadata)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes).with_context(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, &target).with_context(|| format!("replace {}", target.display()))?;
    Ok(())
}

pub(super) fn read_metadata(run_dir: &Path) -> Result<Option<RunMetadata>> {
    let path = run_dir.join("metadata.json");
    if !path.is_file() {
        return Ok(None);
    }
    let value: RunMetadata = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("decode {}", path.display()))?;
    if value.schema_version != LOCAL_SCHEMA_VERSION {
        bail!(
            "unsupported local run schema {} in {}; expected {}",
            value.schema_version,
            path.display(),
            LOCAL_SCHEMA_VERSION
        );
    }
    Ok(Some(value))
}

pub(super) fn read_report(run_dir: &Path) -> Result<Option<E2eReport>> {
    let path = run_dir.join("results/results.json");
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(E2eReport::read_from(&path)?.0))
}

pub(super) fn recover_interrupted_runs(runs_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(mut metadata) = read_metadata(&entry.path())? else {
            continue;
        };
        if metadata.status.active() {
            metadata.status = JobStatus::Failed;
            metadata.completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            metadata.error = "dashboard stopped before the runner completed".into();
            write_metadata(&entry.path(), &metadata)?;
        }
    }
    Ok(())
}

pub(super) fn load_runs(runs_dir: &Path) -> Result<Vec<StoredRun>> {
    let mut runs = Vec::new();
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(metadata) = read_metadata(&entry.path())? else {
            continue;
        };
        runs.push(StoredRun {
            metadata,
            report: read_report(&entry.path())?,
        });
    }
    Ok(runs)
}
