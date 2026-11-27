//! `artifact::create`, `update`, `get`, `list` and `delete`: versioned
//! single-file artifacts.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Ctx, sharing};
use crate::config::WorkerConfig;
use crate::store::{ArtifactFormat, ArtifactRecord, IndexEntry, VersionMeta};
use crate::util::{random_id, unix_now};

pub const CREATE_ID: &str = "artifact::create";
pub const CREATE_DESC: &str = "Create an artifact from one self-contained file: `html` (a design or \
     clickable prototype, inline CSS/JS only — nothing loads from the network when shared), \
     `markdown` (an RFC or doc) or `svg`. Returns the record with its stable id at version 1.";

pub const UPDATE_ID: &str = "artifact::update";
pub const UPDATE_DESC: &str = "Store new content for an artifact as its next version. Earlier \
     versions stay readable and existing shares keep serving the version they froze.";

pub const GET_ID: &str = "artifact::get";
pub const GET_DESC: &str = "Read an artifact's record and the content of one version (the latest \
     when `version` is omitted).";

pub const LIST_ID: &str = "artifact::list";
pub const LIST_DESC: &str = "List artifacts, most recently updated first, optionally filtered by \
     kind. Returns summaries only, never content.";

pub const DELETE_ID: &str = "artifact::delete";
pub const DELETE_DESC: &str = "Delete an artifact with every version, revoking all of its shares \
     (their links stop working at once).";

const MAX_TITLE_CHARS: usize = 200;
const MAX_KIND_CHARS: usize = 40;
const MAX_NOTE_CHARS: usize = 500;
const DEFAULT_KIND: &str = "doc";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateRequest {
    /// Human-readable title.
    pub title: String,
    /// Lowercase category such as `design`, `prototype`, `rfc` or `doc`
    /// (letters, digits, `-`). Defaults to `doc`.
    #[serde(default)]
    pub kind: Option<String>,
    /// `html` (default), `markdown` or `svg`.
    #[serde(default)]
    pub format: Option<ArtifactFormat>,
    /// The whole file. HTML must be self-contained: shared pages cannot load
    /// scripts, styles, fonts or images from the network (use inline code and
    /// `data:` URIs).
    pub content: String,
    /// Optional note for version 1.
    #[serde(default)]
    pub note: Option<String>,
}

pub type CreateResponse = ArtifactRecord;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRequest {
    /// Artifact id.
    pub id: String,
    /// The whole new file, same format as the artifact.
    pub content: String,
    /// What changed in this version.
    #[serde(default)]
    pub note: Option<String>,
    /// New title; omit to keep the current one.
    #[serde(default)]
    pub title: Option<String>,
}

pub type UpdateResponse = ArtifactRecord;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRequest {
    /// Artifact id.
    pub id: String,
    /// Version to read; omit for the latest.
    #[serde(default)]
    pub version: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetResponse {
    pub artifact: ArtifactRecord,
    /// The version whose content is returned.
    pub version: u32,
    pub content: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Only artifacts of this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Most rows to return, capped by the configured `max_list`.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub artifacts: Vec<IndexEntry>,
    /// Matching artifacts before the limit.
    pub total: usize,
    pub truncated: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteRequest {
    /// Artifact id.
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteResponse {
    /// `false` when no artifact had this id.
    pub deleted: bool,
    /// Shares revoked with it.
    pub revoked_shares: usize,
    /// Whether the public tunnel lease was released because no public share
    /// remains.
    pub tunnel_released: bool,
}

pub async fn create(
    ctx: Arc<Ctx>,
    cfg: Arc<WorkerConfig>,
    req: CreateRequest,
) -> Result<CreateResponse, String> {
    let format = req.format.unwrap_or(ArtifactFormat::Html);
    let title = clean_title(&req.title)?;
    let kind = clean_kind(req.kind.as_deref())?;
    let note = clean_note(req.note)?;
    check_content(&req.content, format, &cfg)?;

    let _guard = ctx.store.lock().await;
    let id = mint_id(&ctx).await?;
    let now = unix_now();
    let record = ArtifactRecord {
        id,
        title,
        kind,
        format,
        latest_version: 1,
        versions: vec![VersionMeta {
            version: 1,
            created_at: now,
            bytes: req.content.len(),
            note,
        }],
        created_at: now,
        updated_at: now,
    };
    ctx.store.save_content(&record.id, 1, &req.content).await?;
    if let Err(e) = ctx.store.save_artifact(&record).await {
        // Never leave a stored but unlisted artifact behind: a retry would
        // create a second one instead of finding the first.
        ctx.store.discard_artifact(&record.id, 1).await;
        return Err(e);
    }
    Ok(record)
}

pub async fn update(
    ctx: Arc<Ctx>,
    cfg: Arc<WorkerConfig>,
    req: UpdateRequest,
) -> Result<UpdateResponse, String> {
    let note = clean_note(req.note)?;
    let title = req.title.as_deref().map(clean_title).transpose()?;

    let _guard = ctx.store.lock().await;
    let mut record = load(&ctx, &req.id).await?;
    check_content(&req.content, record.format, &cfg)?;
    if record.latest_version >= cfg.max_versions {
        return Err(format!(
            "artifact {} already has {} versions, the configured max_versions; create a new \
             artifact or raise the limit",
            record.id, record.latest_version
        ));
    }
    let version = record.latest_version + 1;
    let now = unix_now();
    ctx.store
        .save_content(&record.id, version, &req.content)
        .await?;
    record.versions.push(VersionMeta {
        version,
        created_at: now,
        bytes: req.content.len(),
        note,
    });
    record.latest_version = version;
    record.updated_at = now;
    if let Some(title) = title {
        record.title = title;
    }
    ctx.store.save_artifact(&record).await?;
    Ok(record)
}

pub async fn get(
    ctx: Arc<Ctx>,
    _cfg: Arc<WorkerConfig>,
    req: GetRequest,
) -> Result<GetResponse, String> {
    let artifact = load(&ctx, &req.id).await?;
    let version = resolve_version(&artifact, req.version)?;
    let content = ctx
        .store
        .load_content(&artifact.id, version)
        .await?
        .ok_or_else(|| {
            format!(
                "content of {} v{version} is missing from the store",
                artifact.id
            )
        })?;
    Ok(GetResponse {
        artifact,
        version,
        content,
    })
}

pub async fn list(
    ctx: Arc<Ctx>,
    cfg: Arc<WorkerConfig>,
    req: ListRequest,
) -> Result<ListResponse, String> {
    let kind = req.kind.as_deref().map(str::trim).map(str::to_lowercase);
    let matching: Vec<IndexEntry> = ctx
        .store
        .index()
        .await?
        .into_iter()
        .filter(|entry| kind.as_deref().is_none_or(|k| entry.kind == k))
        .collect();
    let total = matching.len();
    let limit = req.limit.unwrap_or(cfg.max_list).clamp(1, cfg.max_list);
    let artifacts: Vec<IndexEntry> = matching.into_iter().take(limit).collect();
    Ok(ListResponse {
        truncated: artifacts.len() < total,
        artifacts,
        total,
    })
}

pub async fn delete(
    ctx: Arc<Ctx>,
    cfg: Arc<WorkerConfig>,
    req: DeleteRequest,
) -> Result<DeleteResponse, String> {
    let _guard = ctx.store.lock().await;
    let Some(record) = ctx.store.load_artifact(&req.id).await? else {
        return Ok(DeleteResponse {
            deleted: false,
            revoked_shares: 0,
            tunnel_released: false,
        });
    };
    let revoked = ctx
        .store
        .remove_shares(|share| share.artifact_id == record.id)
        .await?;
    ctx.store.delete_artifact(&record).await?;
    let tunnel_released = sharing::release_if_idle(&ctx, &cfg, unix_now()).await?;
    Ok(DeleteResponse {
        deleted: true,
        revoked_shares: revoked.len(),
        tunnel_released,
    })
}

pub(crate) async fn load(ctx: &Ctx, id: &str) -> Result<ArtifactRecord, String> {
    ctx.store
        .load_artifact(id)
        .await?
        .ok_or_else(|| format!("no artifact with id `{id}`; artifact::list shows the ids"))
}

pub(crate) fn resolve_version(
    record: &ArtifactRecord,
    version: Option<u32>,
) -> Result<u32, String> {
    match version {
        None => Ok(record.latest_version),
        Some(v) if record.versions.iter().any(|m| m.version == v) => Ok(v),
        Some(v) => Err(format!(
            "artifact {} has no version {v}; it has versions 1..={}",
            record.id, record.latest_version
        )),
    }
}

fn clean_title(title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("title is empty".to_string());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("title is longer than {MAX_TITLE_CHARS} characters"));
    }
    Ok(title.to_string())
}

fn clean_kind(kind: Option<&str>) -> Result<String, String> {
    let kind = kind.map(str::trim).filter(|k| !k.is_empty());
    let Some(kind) = kind else {
        return Ok(DEFAULT_KIND.to_string());
    };
    let kind = kind.to_lowercase();
    let valid = kind.len() <= MAX_KIND_CHARS
        && kind
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        return Err(format!(
            "kind `{kind}` must be up to {MAX_KIND_CHARS} letters, digits or `-` (e.g. `design`, \
             `rfc`)"
        ));
    }
    Ok(kind)
}

fn clean_note(note: Option<String>) -> Result<Option<String>, String> {
    let Some(note) = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(format!("note is longer than {MAX_NOTE_CHARS} characters"));
    }
    Ok(Some(note))
}

fn check_content(content: &str, format: ArtifactFormat, cfg: &WorkerConfig) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("content is empty".to_string());
    }
    if content.len() > cfg.max_content_bytes {
        return Err(format!(
            "content is {} bytes; the configured cap is {} bytes",
            content.len(),
            cfg.max_content_bytes
        ));
    }
    if format == ArtifactFormat::Svg && !content.to_ascii_lowercase().contains("<svg") {
        return Err("svg content has no <svg> element".to_string());
    }
    Ok(())
}

async fn mint_id(ctx: &Ctx) -> Result<String, String> {
    for _ in 0..5 {
        let id = random_id()?;
        if ctx.store.load_artifact(&id).await?.is_none() {
            return Ok(id);
        }
    }
    Err("could not mint an unused artifact id after 5 attempts".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Arc<WorkerConfig> {
        Arc::new(WorkerConfig::default())
    }

    fn html(title: &str) -> CreateRequest {
        CreateRequest {
            title: title.to_string(),
            kind: Some("Prototype".to_string()),
            format: None,
            content: "<h1>v1</h1>".to_string(),
            note: None,
        }
    }

    #[tokio::test]
    async fn create_stores_version_one_with_defaults() {
        let ctx = Ctx::for_tests(None);
        let record = create(ctx.clone(), cfg(), html("  Phone  "))
            .await
            .expect("creates");
        assert_eq!(record.title, "Phone");
        assert_eq!(record.kind, "prototype");
        assert_eq!(record.format, ArtifactFormat::Html);
        assert_eq!(record.latest_version, 1);
        let got = get(
            ctx,
            cfg(),
            GetRequest {
                id: record.id,
                version: None,
            },
        )
        .await
        .expect("gets");
        assert_eq!(got.content, "<h1>v1</h1>");
    }

    #[tokio::test]
    async fn update_appends_a_version_and_keeps_the_old_one() {
        let ctx = Ctx::for_tests(None);
        let record = create(ctx.clone(), cfg(), html("p"))
            .await
            .expect("creates");
        let updated = update(
            ctx.clone(),
            cfg(),
            UpdateRequest {
                id: record.id.clone(),
                content: "<h1>v2</h1>".into(),
                note: Some("light theme".into()),
                title: None,
            },
        )
        .await
        .expect("updates");
        assert_eq!(updated.latest_version, 2);
        assert_eq!(updated.versions[1].note.as_deref(), Some("light theme"));
        let v1 = get(
            ctx.clone(),
            cfg(),
            GetRequest {
                id: record.id.clone(),
                version: Some(1),
            },
        )
        .await
        .expect("v1");
        assert_eq!(v1.content, "<h1>v1</h1>");
        let missing = get(
            ctx,
            cfg(),
            GetRequest {
                id: record.id,
                version: Some(9),
            },
        )
        .await;
        assert!(missing.expect_err("no v9").contains("no version 9"));
    }

    #[tokio::test]
    async fn update_refuses_past_max_versions() {
        let ctx = Ctx::for_tests(None);
        let capped = Arc::new(WorkerConfig {
            max_versions: 1,
            ..WorkerConfig::default()
        });
        let record = create(ctx.clone(), capped.clone(), html("p"))
            .await
            .expect("creates");
        let err = update(
            ctx,
            capped,
            UpdateRequest {
                id: record.id,
                content: "x".into(),
                note: None,
                title: None,
            },
        )
        .await
        .expect_err("capped");
        assert!(err.contains("max_versions"), "{err}");
    }

    #[tokio::test]
    async fn invalid_input_is_rejected() {
        let ctx = Ctx::for_tests(None);
        let mut empty = html("p");
        empty.content = "  ".into();
        assert!(create(ctx.clone(), cfg(), empty).await.is_err());

        let mut bad_kind = html("p");
        bad_kind.kind = Some("design system!".into());
        assert!(create(ctx.clone(), cfg(), bad_kind).await.is_err());

        let mut not_svg = html("p");
        not_svg.format = Some(ArtifactFormat::Svg);
        assert!(create(ctx.clone(), cfg(), not_svg).await.is_err());

        let small = Arc::new(WorkerConfig {
            max_content_bytes: 4,
            ..WorkerConfig::default()
        });
        assert!(create(ctx, small, html("p")).await.is_err());
    }

    #[tokio::test]
    async fn list_filters_by_kind_and_reports_truncation() {
        let ctx = Ctx::for_tests(None);
        create(ctx.clone(), cfg(), html("a")).await.expect("a");
        create(ctx.clone(), cfg(), html("b")).await.expect("b");
        let mut rfc = html("rfc");
        rfc.kind = Some("rfc".into());
        rfc.format = Some(ArtifactFormat::Markdown);
        create(ctx.clone(), cfg(), rfc).await.expect("rfc");

        let protos = list(
            ctx.clone(),
            cfg(),
            ListRequest {
                kind: Some("prototype".into()),
                limit: Some(1),
            },
        )
        .await
        .expect("lists");
        assert_eq!(protos.total, 2);
        assert_eq!(protos.artifacts.len(), 1);
        assert!(protos.truncated);

        let all = list(ctx, cfg(), ListRequest::default())
            .await
            .expect("lists");
        assert_eq!(all.total, 3);
    }

    #[tokio::test]
    async fn deleting_an_unknown_id_is_not_an_error() {
        let ctx = Ctx::for_tests(None);
        let out = delete(
            ctx,
            cfg(),
            DeleteRequest {
                id: "nope0000".into(),
            },
        )
        .await
        .expect("answers");
        assert!(!out.deleted);
    }
}
