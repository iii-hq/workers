use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use cucumber::World;
use iii_sdk::{protocol::TriggerRequest, IIIClient};
use serde::Serialize;
use serde_json::Value;
use shell::code::{
    config::CoderConfig,
    functions::{
        create_file, delete_file, info, list_folder, move_file, read_file, search, tree,
        update_file,
    },
    path::PathResolver,
};
use tempfile::TempDir;

#[derive(World)]
#[world(init = Self::new)]
pub struct CodeWorld {
    surface: Option<CodeSurface>,
    live_client: Option<Arc<IIIClient>>,
    skip_reason: Option<String>,
    last_ok: Option<Value>,
    last_err: Option<String>,
}

pub struct CodeSurface {
    _primary_tmp: TempDir,
    _secondary_tmp: TempDir,
    _outside_tmp: TempDir,
    pub root: PathBuf,
    pub secondary_root: PathBuf,
    pub outside_root: PathBuf,
    pub session_dir: Option<PathBuf>,
    pub resolver: Arc<PathResolver>,
    pub cfg: Arc<CoderConfig>,
}

impl CodeWorld {
    pub fn new() -> Self {
        Self {
            surface: None,
            live_client: None,
            skip_reason: None,
            last_ok: None,
            last_err: None,
        }
    }

    pub fn setup_direct_surface(&mut self) {
        self.surface = Some(CodeSurface::new());
        self.live_client = None;
        self.skip_reason = None;
        self.clear_result();
    }

    pub fn setup_live_client(&mut self, client: Arc<IIIClient>) {
        self.surface = None;
        self.live_client = Some(client);
        self.skip_reason = None;
        self.clear_result();
    }

    pub fn soft_skip(&mut self, reason: impl Into<String>) {
        self.skip_reason = Some(reason.into());
        self.clear_result();
    }

    pub fn is_skipped(&self) -> bool {
        self.skip_reason.is_some()
    }

    pub fn surface(&self) -> &CodeSurface {
        self.surface
            .as_ref()
            .expect("expected a direct code surface; add `Given a jailed code surface`")
    }

    pub fn surface_mut(&mut self) -> &mut CodeSurface {
        self.surface
            .as_mut()
            .expect("expected a direct code surface; add `Given a jailed code surface`")
    }

    pub fn clear_result(&mut self) {
        self.last_ok = None;
        self.last_err = None;
    }

    pub fn last_ok(&self) -> &Value {
        self.last_ok
            .as_ref()
            .unwrap_or_else(|| panic!("expected last call to succeed, got {:?}", self.last_err))
    }

    pub fn last_err(&self) -> &str {
        self.last_err
            .as_deref()
            .unwrap_or_else(|| panic!("expected last call to fail, got {:?}", self.last_ok))
    }

    pub fn expand(&self, input: &str) -> String {
        let mut value = input.to_string();
        if let Some(surface) = &self.surface {
            value = value.replace("{{root}}", &surface.root.to_string_lossy());
            value = value.replace("{{secondary}}", &surface.secondary_root.to_string_lossy());
            value = value.replace("{{outside}}", &surface.outside_root.to_string_lossy());
            if let Some(session_dir) = &surface.session_dir {
                value = value.replace("{{session}}", &session_dir.to_string_lossy());
            }
        }
        value
    }

    pub async fn call_function(&mut self, function_id: &str, payload: Value) {
        if self.is_skipped() {
            return;
        }

        self.clear_result();
        let result = if let Some(client) = self.live_client.clone() {
            call_live(client, function_id, payload).await
        } else {
            call_direct(self.surface(), function_id, payload).await
        };

        match result {
            Ok(value) => self.last_ok = Some(value),
            Err(err) => self.last_err = Some(err),
        }
    }

    pub fn path_matches(&self, actual: &str, expected: &str) -> bool {
        if actual == expected {
            return true;
        }

        let expanded = self.expand(expected);
        if actual == expanded {
            return true;
        }

        let actual_path = Path::new(actual);
        if actual_path == Path::new(&expanded) {
            return true;
        }

        if let Some(surface) = &self.surface {
            return actual_path == surface.root.join(expected)
                || actual_path == surface.secondary_root.join(expected)
                || actual_path == surface.outside_root.join(expected);
        }

        false
    }
}

impl fmt::Debug for CodeWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeWorld")
            .field(
                "surface",
                &self.surface.as_ref().map(|surface| &surface.root),
            )
            .field("live_client", &self.live_client.is_some())
            .field("skip_reason", &self.skip_reason)
            .field("last_ok", &self.last_ok)
            .field("last_err", &self.last_err)
            .finish()
    }
}

impl CodeSurface {
    fn new() -> Self {
        let primary_tmp = tempfile::tempdir().expect("create primary jail");
        let secondary_tmp = tempfile::tempdir().expect("create secondary jail");
        let outside_tmp = tempfile::tempdir().expect("create outside dir");

        let root = primary_tmp
            .path()
            .canonicalize()
            .expect("canonical primary jail");
        let secondary_root = secondary_tmp
            .path()
            .canonicalize()
            .expect("canonical secondary jail");
        let outside_root = outside_tmp
            .path()
            .canonicalize()
            .expect("canonical outside dir");

        let cfg = Arc::new(CoderConfig {
            base_paths: vec![root.clone(), secondary_root.clone()],
            non_accessible_globs: vec![
                "**/.env".to_string(),
                "**/.env.*".to_string(),
                "**/*.pem".to_string(),
                "**/*.key".to_string(),
                "**/secrets/**".to_string(),
            ],
            max_read_bytes: 512,
            max_write_bytes: 512,
            max_output_bytes: 384,
            batch_read_budget_bytes: 160,
            search_response_budget_bytes: 512,
            search_default_max_matches: 3,
            search_default_max_line_bytes: 80,
            list_default_page_size: 2,
            list_max_page_size: 4,
            tree_default_depth: 2,
            tree_per_folder_limit: 3,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).expect("build path resolver"));

        Self {
            _primary_tmp: primary_tmp,
            _secondary_tmp: secondary_tmp,
            _outside_tmp: outside_tmp,
            root,
            secondary_root,
            outside_root,
            session_dir: None,
            resolver,
            cfg,
        }
    }

    pub fn write_file(&self, rel: &str, content: &[u8]) {
        let path = self.root.join(rel);
        write_fixture(&path, content);
    }

    pub fn write_secondary_file(&self, rel: &str, content: &[u8]) {
        let path = self.secondary_root.join(rel);
        write_fixture(&path, content);
    }

    pub fn create_dir(&self, rel: &str) {
        fs::create_dir_all(self.root.join(rel)).expect("create fixture directory");
    }

    pub fn set_session_dir(&mut self, rel: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(&path).expect("create session directory");
        self.session_dir = Some(path);
    }
}

async fn call_direct(
    surface: &CodeSurface,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    match function_id {
        "coder::info" => {
            let input: info::InfoInput = decode_payload(payload)?;
            serialize_result(
                info::handle(surface.resolver.clone(), surface.cfg.clone(), input).await,
            )
        }
        "coder::create-file" => {
            let input = decode_payload(payload)?;
            serialize_result(
                create_file::handle(surface.resolver.clone(), surface.cfg.clone(), input).await,
            )
        }
        "coder::read-file" => {
            let input = decode_payload(payload)?;
            serialize_result(
                read_file::handle(surface.resolver.clone(), surface.cfg.clone(), input).await,
            )
        }
        "coder::update-file" => {
            let input = decode_payload(payload)?;
            serialize_result(
                update_file::handle(surface.resolver.clone(), surface.cfg.clone(), input).await,
            )
        }
        "coder::delete-file" => {
            let input = decode_payload(payload)?;
            serialize_result(delete_file::handle(surface.resolver.clone(), input).await)
        }
        "coder::move" => {
            let input = decode_payload(payload)?;
            serialize_result(move_file::handle(surface.resolver.clone(), input).await)
        }
        "coder::list-folder" => {
            let input = decode_payload(payload)?;
            serialize_result(
                list_folder::handle(surface.resolver.clone(), surface.cfg.clone(), input).await,
            )
        }
        "coder::tree" => {
            let input = decode_payload(payload)?;
            serialize_result(
                tree::handle(surface.resolver.clone(), surface.cfg.clone(), input).await,
            )
        }
        "coder::search" => {
            let input = decode_payload(payload)?;
            serialize_result(
                search::handle(surface.resolver.clone(), surface.cfg.clone(), input).await,
            )
        }
        other => Err(format!("unknown function id {other}")),
    }
}

async fn call_live(
    client: Arc<IIIClient>,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let response = client
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|err| err.to_string())?;
    Ok(response)
}

fn decode_payload<T>(payload: Value) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(payload).map_err(|err| format!("invalid payload: {err}"))
}

fn serialize_result<T>(result: Result<T, String>) -> Result<Value, String>
where
    T: Serialize,
{
    result.and_then(|value| serde_json::to_value(value).map_err(|err| err.to_string()))
}

fn write_fixture(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture parent directory");
    }
    fs::write(path, content).expect("write fixture file");
}
