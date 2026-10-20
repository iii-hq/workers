//! Native filesystem-backed local object storage.
//!
//! Objects are stored under the configured data directory without an S3
//! sidecar. Small RPC reads/writes use the normal [`Backend`] interface; an
//! optional Axum server streams signed downloads, raw signed PUTs (backward
//! compatibility), and multipart presigned POST uploads for large files.

use super::*;
use crate::config::{LocalHttpConfig, LocalProviderConfig};
use crate::triggers::dispatcher::EventDispatcher;
use crate::triggers::normalize::{EventKind, ObjectEventNormalized};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_util::io::ReaderStream;
use tower_http::cors::{Any, CorsLayer};
use url::Url;

type HmacSha256 = Hmac<Sha256>;

const OBJECTS_DIR: &str = "objects";
const TMP_SUFFIX: &str = ".upload";

#[derive(Clone)]
pub struct LocalBackendCtx {
    service: Arc<LocalService>,
}

pub struct PreparedLocalService {
    pub context: LocalBackendCtx,
    listener: Option<TcpListener>,
}

pub struct LocalHttpHandle {
    pub addr: SocketAddr,
    control: LocalHttpControl,
}

impl LocalHttpHandle {
    pub fn shutdown(self) {
        stop_old_http(self.control);
    }
}

type ActiveLocalService = Arc<RwLock<Option<Arc<LocalService>>>>;

#[derive(Clone)]
struct LocalHttpState {
    active: ActiveLocalService,
    reconfigure_gate: Arc<RwLock<()>>,
}

struct LocalHttpControl {
    addr: SocketAddr,
    graceful: oneshot::Sender<()>,
    abort: AbortHandle,
    join: JoinHandle<()>,
}

struct LocalHttpRuntimeState {
    bind_address: Option<String>,
    control: Option<LocalHttpControl>,
}

/// Long-lived native-local runtime. Each configuration application prepares a
/// fresh service generation, while the HTTP listener reads the active
/// generation per request. This lets data directories, bucket mappings,
/// browser-facing URLs, and the listener itself change without a restart.
pub struct LocalRuntime {
    dispatcher: Arc<dyn EventDispatcher>,
    signing_secret: [u8; 32],
    active: ActiveLocalService,
    http: Mutex<LocalHttpRuntimeState>,
    reconfigure_gate: Arc<RwLock<()>>,
}

/// Fallible local resources prepared before a configuration is made visible.
/// Dropping this value before `commit` closes any newly-bound listener and
/// leaves the running generation untouched.
pub struct PreparedLocalUpdate {
    context: Option<LocalBackendCtx>,
    service: Option<Arc<LocalService>>,
    listener: Option<TcpListener>,
    bind_address: Option<String>,
    keep_listener: bool,
}

impl PreparedLocalUpdate {
    pub fn context(&self) -> Option<&LocalBackendCtx> {
        self.context.as_ref()
    }
}

impl LocalRuntime {
    pub fn new(dispatcher: Arc<dyn EventDispatcher>, reconfigure_gate: Arc<RwLock<()>>) -> Self {
        let mut signing_secret = [0_u8; 32];
        rand::thread_rng().fill_bytes(&mut signing_secret);
        Self {
            dispatcher,
            signing_secret,
            active: Arc::new(RwLock::new(None)),
            http: Mutex::new(LocalHttpRuntimeState {
                bind_address: None,
                control: None,
            }),
            reconfigure_gate,
        }
    }

    /// Resolve all fallible local resources for a candidate configuration.
    /// When the bind address is unchanged, the existing socket is retained and
    /// only the per-request service generation is replaced.
    pub async fn prepare_update(
        &self,
        config: Option<&LocalProviderConfig>,
        needs_local_backend: bool,
    ) -> Result<PreparedLocalUpdate, BackendError> {
        let needs_service = needs_local_backend || config.is_some();
        if !needs_service {
            return Ok(PreparedLocalUpdate {
                context: None,
                service: None,
                listener: None,
                bind_address: None,
                keep_listener: false,
            });
        }

        let data_dir = config
            .map(|value| value.data_dir.as_str())
            .unwrap_or("data/storage");
        let root = iii_worker_paths::resolve_path(data_dir);
        tokio::fs::create_dir_all(&root).await.map_err(|error| {
            provider_error(format!("create local data dir {}: {error}", root.display()))
        })?;

        // Signed transfers need the HTTP listener; when the operator did not
        // configure one, fall back to a free loopback port instead of leaving
        // presign calls to fail with PRESIGN_UNSUPPORTED.
        let default_http = LocalHttpConfig::default();
        let desired_http = Some(
            config
                .and_then(|value| value.http.as_ref())
                .unwrap_or(&default_http),
        );
        let desired_bind = desired_http.map(|http| http.bind_address.clone());
        let (current_bind, current_addr, current_running) = {
            let current = self.http.lock().await;
            (
                current.bind_address.clone(),
                current.control.as_ref().map(|control| control.addr),
                current
                    .control
                    .as_ref()
                    .is_some_and(|control| !control.join.is_finished()),
            )
        };
        let keep_listener = desired_bind.is_some()
            && desired_bind == current_bind
            && current_addr.is_some()
            && current_running;

        let (listener, resolved_addr) = match desired_http {
            Some(_) if keep_listener => (None, current_addr),
            Some(http) => {
                let listener = TcpListener::bind(&http.bind_address)
                    .await
                    .map_err(|error| {
                        provider_error(format!(
                            "bind local HTTP server at {}: {error}",
                            http.bind_address
                        ))
                    })?;
                let addr = listener
                    .local_addr()
                    .map_err(|error| provider_error(format!("read local HTTP address: {error}")))?;
                (Some(listener), Some(addr))
            }
            None => (None, None),
        };

        let public_url = match desired_http {
            Some(http) => Some(match http.public_url.as_deref() {
                Some(value) => normalize_public_url(value)?,
                None => normalize_public_url(&format!(
                    "http://{}",
                    browser_address(resolved_addr.expect("HTTP listener address resolved"))
                ))?,
            }),
            None => None,
        };
        let service = Arc::new(LocalService {
            root,
            public_url,
            signing_secret: self.signing_secret,
            stores: StdRwLock::new(HashMap::new()),
            dispatcher: self.dispatcher.clone(),
        });

        Ok(PreparedLocalUpdate {
            context: Some(LocalBackendCtx {
                service: service.clone(),
            }),
            service: Some(service),
            listener,
            bind_address: desired_bind,
            keep_listener,
        })
    }

    /// Publish a fully-prepared local generation. Callers hold the shared
    /// reconfiguration write gate while also swapping RPC backends, so neither
    /// signed HTTP requests nor RPC calls can observe a half-applied config.
    pub async fn commit(&self, mut prepared: PreparedLocalUpdate) {
        *self.active.write().await = prepared.service.take();

        let mut current = self.http.lock().await;
        if prepared.keep_listener {
            current.bind_address = prepared.bind_address;
            return;
        }

        let replacement = prepared.listener.take().map(|listener| {
            spawn_http_server(
                listener,
                LocalHttpState {
                    active: self.active.clone(),
                    reconfigure_gate: self.reconfigure_gate.clone(),
                },
            )
        });
        let old = std::mem::replace(&mut current.control, replacement);
        current.bind_address = prepared.bind_address;
        drop(current);
        if let Some(old) = old {
            stop_old_http(old);
        }
    }

    pub async fn current_addr(&self) -> Option<SocketAddr> {
        self.http
            .lock()
            .await
            .control
            .as_ref()
            .map(|control| control.addr)
    }

    pub async fn shutdown(&self) {
        *self.active.write().await = None;
        if let Some(control) = self.http.lock().await.control.take() {
            let _ = control.graceful.send(());
            let _ = control.join.await;
        }
    }
}

/// Prepare the native local store and, when configured, bind its HTTP socket.
/// Binding happens before backends are built so startup fails cleanly instead
/// of handing out signed URLs for a server that never became reachable.
pub async fn prepare(
    config: Option<&LocalProviderConfig>,
    dispatcher: Arc<dyn EventDispatcher>,
) -> Result<PreparedLocalService, BackendError> {
    let data_dir = config
        .map(|value| value.data_dir.as_str())
        .unwrap_or("data/storage");
    let root = iii_worker_paths::resolve_path(data_dir);
    tokio::fs::create_dir_all(&root).await.map_err(|error| {
        provider_error(format!("create local data dir {}: {error}", root.display()))
    })?;

    let default_http = LocalHttpConfig::default();
    let (listener, public_url) = match Some(
        config
            .and_then(|value| value.http.as_ref())
            .unwrap_or(&default_http),
    ) {
        Some(http) => {
            let listener = TcpListener::bind(&http.bind_address)
                .await
                .map_err(|error| {
                    provider_error(format!(
                        "bind local HTTP server at {}: {error}",
                        http.bind_address
                    ))
                })?;
            let addr = listener
                .local_addr()
                .map_err(|error| provider_error(format!("read local HTTP address: {error}")))?;
            let public_url = match http.public_url.as_deref() {
                Some(value) => normalize_public_url(value)?,
                None => normalize_public_url(&format!("http://{}", browser_address(addr)))?,
            };
            (Some(listener), Some(public_url))
        }
        None => (None, None),
    };

    let mut signing_secret = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut signing_secret);
    let service = Arc::new(LocalService {
        root,
        public_url,
        signing_secret,
        stores: StdRwLock::new(HashMap::new()),
        dispatcher,
    });
    Ok(PreparedLocalService {
        context: LocalBackendCtx { service },
        listener,
    })
}

/// Start serving a prepared listener. Returns `None` when local HTTP was not
/// configured; inline local storage remains fully functional in that case.
pub fn start_http(prepared: &mut PreparedLocalService) -> Option<LocalHttpHandle> {
    let listener = prepared.listener.take()?;
    let addr = listener.local_addr().ok()?;
    let active = Arc::new(RwLock::new(Some(prepared.context.service.clone())));
    let reconfigure_gate = Arc::new(RwLock::new(()));
    let control = spawn_http_server(
        listener,
        LocalHttpState {
            active,
            reconfigure_gate,
        },
    );
    Some(LocalHttpHandle { addr, control })
}

fn local_http_router(state: LocalHttpState) -> Router {
    Router::new()
        .route(
            "/v1/objects/download",
            get(download_handler).head(download_handler),
        )
        .route(
            "/v1/objects/upload",
            put(put_upload_handler).post(post_upload_handler),
        )
        .layer(DefaultBodyLimit::disable())
        // Signed URLs are often consumed by the Console on another local
        // origin. The signature remains the authorization boundary; CORS
        // only lets browsers send the already-authorized request.
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD, Method::PUT, Method::POST])
                .allow_headers([header::CONTENT_TYPE, header::RANGE, header::IF_RANGE])
                .expose_headers([
                    header::CONTENT_LENGTH,
                    header::CONTENT_TYPE,
                    header::ETAG,
                    header::ACCEPT_RANGES,
                    header::CONTENT_RANGE,
                    header::CONTENT_DISPOSITION,
                    header::LAST_MODIFIED,
                ]),
        )
        .with_state(state)
}

fn spawn_http_server(listener: TcpListener, state: LocalHttpState) -> LocalHttpControl {
    let addr = listener
        .local_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    let app = local_http_router(state);
    let (graceful, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        tracing::info!(address = %addr, "native local storage HTTP server listening");
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::error!(error = %error, "local storage HTTP server stopped unexpectedly");
        }
    });
    let abort = join.abort_handle();
    LocalHttpControl {
        addr,
        graceful,
        abort,
        join,
    }
}

fn stop_old_http(old: LocalHttpControl) {
    let _ = old.graceful.send(());
    let abort = old.abort;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        abort.abort();
    });
}

pub fn build(
    ctx: &LocalBackendCtx,
    worker_bucket: String,
    underlying_bucket: String,
) -> Result<Arc<dyn Backend>, BackendError> {
    ctx.service.backend(worker_bucket, underlying_bucket)
}

pub struct LocalService {
    root: PathBuf,
    public_url: Option<String>,
    signing_secret: [u8; 32],
    stores: StdRwLock<HashMap<String, Arc<LocalBackend>>>,
    dispatcher: Arc<dyn EventDispatcher>,
}

impl LocalService {
    fn backend(
        self: &Arc<Self>,
        worker_bucket: String,
        underlying_bucket: String,
    ) -> Result<Arc<dyn Backend>, BackendError> {
        if let Some(existing) = self
            .stores
            .read()
            .expect("local store registry poisoned")
            .get(&worker_bucket)
            .cloned()
        {
            if existing.store.underlying_bucket != underlying_bucket {
                return Err(provider_error(format!(
                    "local bucket alias `{worker_bucket}` was already bound to `{}`",
                    existing.store.underlying_bucket
                )));
            }
            return Ok(existing);
        }
        let bucket_root = self.root.join("buckets").join(&underlying_bucket);
        std::fs::create_dir_all(bucket_root.join(OBJECTS_DIR)).map_err(|error| {
            provider_error(format!(
                "create local bucket directory {}: {error}",
                bucket_root.display()
            ))
        })?;
        let backend = Arc::new(LocalBackend {
            service: self.clone(),
            store: Arc::new(LocalStore {
                worker_bucket: worker_bucket.clone(),
                underlying_bucket,
                bucket_root,
                commit_lock: Mutex::new(()),
            }),
        });
        self.stores
            .write()
            .expect("local store registry poisoned")
            .insert(worker_bucket, backend.clone());
        Ok(backend)
    }

    fn resolve_backend(&self, worker_bucket: &str) -> Option<Arc<LocalBackend>> {
        self.stores
            .read()
            .expect("local store registry poisoned")
            .get(worker_bucket)
            .cloned()
    }

    fn issue_token(&self, operation: &SignedOperation) -> Result<String, BackendError> {
        let payload = serde_json::to_vec(operation)
            .map_err(|error| provider_error(format!("serialize signed operation: {error}")))?;
        let encoded = URL_SAFE_NO_PAD.encode(payload);
        let mut mac = HmacSha256::new_from_slice(&self.signing_secret)
            .map_err(|error| provider_error(format!("initialize URL signer: {error}")))?;
        mac.update(encoded.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        Ok(format!("{encoded}.{signature}"))
    }

    fn verify_token(
        &self,
        token: &str,
        expected_kind: SignedKind,
    ) -> Result<SignedOperation, StatusCode> {
        let (encoded, signature) = token.split_once('.').ok_or(StatusCode::UNAUTHORIZED)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let mut mac = HmacSha256::new_from_slice(&self.signing_secret)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        mac.update(encoded.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let operation: SignedOperation =
            serde_json::from_slice(&payload).map_err(|_| StatusCode::UNAUTHORIZED)?;
        if operation.kind != expected_kind || operation.expires_at < Utc::now().timestamp() {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(operation)
    }

    fn signed_url(&self, route: &str, operation: &SignedOperation) -> Result<String, BackendError> {
        let base = self.public_url.as_deref().ok_or_else(|| {
            BackendError::PresignUnsupported(
                "local signed transfers require providers.local.http configuration".into(),
            )
        })?;
        let token = self.issue_token(operation)?;
        let mut url = Url::parse(&format!("{}{}", base.trim_end_matches('/'), route))
            .map_err(|error| provider_error(format!("build signed local URL: {error}")))?;
        url.query_pairs_mut().append_pair("token", &token);
        Ok(url.into())
    }

    fn emit(&self, event: ObjectEventNormalized) {
        let dispatcher = self.dispatcher.clone();
        tokio::spawn(async move {
            if !dispatcher.dispatch(event).await {
                tracing::warn!("one or more local object event subscribers did not acknowledge");
            }
        });
    }
}

pub struct LocalBackend {
    service: Arc<LocalService>,
    store: Arc<LocalStore>,
}

struct LocalStore {
    worker_bucket: String,
    underlying_bucket: String,
    bucket_root: PathBuf,
    commit_lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMetadata {
    key: String,
    data_file: String,
    content_type: String,
    cache_control: Option<String>,
    metadata: HashMap<String, String>,
    etag: String,
    size: u64,
    last_modified: DateTime<Utc>,
}

struct PendingUpload {
    file: File,
    temp_path: PathBuf,
    data_path: PathBuf,
    metadata_path: PathBuf,
    hasher: Sha256,
    size: u64,
}

impl LocalBackend {
    async fn metadata(&self, key: &str) -> Result<StoredMetadata, BackendError> {
        let metadata_path = self.store.metadata_path(key);
        let bytes = tokio::fs::read(&metadata_path).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                BackendError::NotFound
            } else {
                provider_error(format!(
                    "read metadata {}: {error}",
                    metadata_path.display()
                ))
            }
        })?;
        let metadata: StoredMetadata = serde_json::from_slice(&bytes).map_err(|error| {
            provider_error(format!(
                "parse local object metadata {}: {error}",
                metadata_path.display()
            ))
        })?;
        if metadata.key != key {
            return Err(provider_error("local object metadata key mismatch"));
        }
        Ok(metadata)
    }

    async fn begin_upload(&self, key: &str) -> Result<PendingUpload, BackendError> {
        let metadata_path = self.store.metadata_path(key);
        let parent = metadata_path
            .parent()
            .ok_or_else(|| provider_error("local object path has no parent"))?;
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            provider_error(format!("create object shard {}: {error}", parent.display()))
        })?;
        let mut nonce = [0_u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        let data_file = format!("{digest}-{}.data", hex::encode(nonce));
        let data_path = parent.join(&data_file);
        let temp_path = parent.join(format!("{data_file}-{TMP_SUFFIX}"));
        let file = File::create(&temp_path).await.map_err(|error| {
            provider_error(format!(
                "create upload temp file {}: {error}",
                temp_path.display()
            ))
        })?;
        Ok(PendingUpload {
            file,
            temp_path,
            data_path,
            metadata_path,
            hasher: Sha256::new(),
            size: 0,
        })
    }

    async fn write_chunk(
        pending: &mut PendingUpload,
        chunk: &[u8],
        max_size_bytes: Option<u64>,
    ) -> Result<(), BackendError> {
        let next_size = pending.size.saturating_add(chunk.len() as u64);
        if max_size_bytes.is_some_and(|limit| next_size > limit) {
            return Err(BackendError::ObjectTooLarge {
                actual_size: next_size,
                cap: max_size_bytes.unwrap_or_default(),
            });
        }
        pending
            .file
            .write_all(chunk)
            .await
            .map_err(|error| provider_error(format!("write local upload: {error}")))?;
        pending.hasher.update(chunk);
        pending.size = next_size;
        Ok(())
    }

    async fn commit_upload(
        &self,
        mut pending: PendingUpload,
        key: String,
        content_type: String,
        cache_control: Option<String>,
        metadata: HashMap<String, String>,
    ) -> Result<StoredMetadata, BackendError> {
        pending
            .file
            .flush()
            .await
            .map_err(|error| provider_error(format!("flush local upload: {error}")))?;
        pending
            .file
            .sync_data()
            .await
            .map_err(|error| provider_error(format!("sync local upload: {error}")))?;
        drop(pending.file);
        let stored = StoredMetadata {
            key: key.clone(),
            data_file: pending
                .data_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| provider_error("local data path is not valid UTF-8"))?
                .to_string(),
            content_type,
            cache_control,
            metadata,
            etag: format!("\"{}\"", hex::encode(pending.hasher.finalize())),
            size: pending.size,
            last_modified: Utc::now(),
        };
        let encoded = serde_json::to_vec(&stored)
            .map_err(|error| provider_error(format!("serialize local object metadata: {error}")))?;
        let metadata_temp = pending
            .metadata_path
            .with_extension(format!("json-{}-{TMP_SUFFIX}", rand::random::<u64>()));
        tokio::fs::write(&metadata_temp, encoded)
            .await
            .map_err(|error| {
                provider_error(format!(
                    "write metadata temp file {}: {error}",
                    metadata_temp.display()
                ))
            })?;

        let _guard = self.store.commit_lock.lock().await;
        let previous = tokio::fs::read(&pending.metadata_path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoredMetadata>(&bytes).ok());
        tokio::fs::rename(&pending.temp_path, &pending.data_path)
            .await
            .map_err(|error| {
                provider_error(format!(
                    "commit object {}: {error}",
                    pending.data_path.display()
                ))
            })?;
        tokio::fs::rename(&metadata_temp, &pending.metadata_path)
            .await
            .map_err(|error| {
                provider_error(format!(
                    "commit metadata {}: {error}",
                    pending.metadata_path.display()
                ))
            })?;
        if let Some(previous) = previous {
            if previous.data_file != stored.data_file {
                let _ = tokio::fs::remove_file(self.store.data_path(&previous.data_file)).await;
            }
        }
        self.emit_created(&stored);
        Ok(stored)
    }

    fn emit_created(&self, metadata: &StoredMetadata) {
        self.service.emit(ObjectEventNormalized {
            bucket: self.store.worker_bucket.clone(),
            key: metadata.key.clone(),
            size: metadata.size,
            etag: metadata.etag.clone(),
            content_type: Some(metadata.content_type.clone()),
            created_at: Some(metadata.last_modified),
            deleted_at: None,
            event_kind: EventKind::Created,
            raw_event_id: random_event_id(),
        });
    }

    fn emit_deleted(&self, key: String) {
        self.service.emit(ObjectEventNormalized {
            bucket: self.store.worker_bucket.clone(),
            key,
            size: 0,
            etag: String::new(),
            content_type: None,
            created_at: None,
            deleted_at: Some(Utc::now()),
            event_kind: EventKind::Deleted,
            raw_event_id: random_event_id(),
        });
    }

    async fn abort_upload(pending: PendingUpload) {
        drop(pending.file);
        let _ = tokio::fs::remove_file(pending.temp_path).await;
    }

    async fn stream_into_pending<S, E>(
        &self,
        stream: S,
        key: String,
        content_type: String,
        max_size_bytes: Option<u64>,
    ) -> Result<StoredMetadata, BackendError>
    where
        S: Stream<Item = Result<Bytes, E>>,
        E: Display,
    {
        let mut pending = self.begin_upload(&key).await?;
        futures_util::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    Self::abort_upload(pending).await;
                    return Err(provider_error(format!("read upload stream: {error}")));
                }
            };
            if let Err(error) = Self::write_chunk(&mut pending, &chunk, max_size_bytes).await {
                Self::abort_upload(pending).await;
                return Err(error);
            }
        }
        self.commit_upload(pending, key, content_type, None, HashMap::new())
            .await
    }
}

impl LocalStore {
    fn metadata_path(&self, key: &str) -> PathBuf {
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        let directory = self.bucket_root.join(OBJECTS_DIR).join(&digest[..2]);
        directory.join(format!("{digest}.json"))
    }

    fn data_path(&self, data_file: &str) -> PathBuf {
        let shard = data_file.get(..2).unwrap_or("__");
        self.bucket_root
            .join(OBJECTS_DIR)
            .join(shard)
            .join(data_file)
    }

    async fn all_metadata(&self) -> Result<Vec<StoredMetadata>, BackendError> {
        let root = self.bucket_root.join(OBJECTS_DIR);
        tokio::task::spawn_blocking(move || scan_metadata(&root))
            .await
            .map_err(|error| provider_error(format!("join local metadata scan: {error}")))?
    }
}

#[async_trait]
impl Backend for LocalBackend {
    async fn put(&self, req: PutReq) -> Result<PutResp, BackendError> {
        let mut pending = self.begin_upload(&req.key).await?;
        if let Err(error) = Self::write_chunk(&mut pending, &req.body, None).await {
            Self::abort_upload(pending).await;
            return Err(error);
        }
        let stored = self
            .commit_upload(
                pending,
                req.key,
                req.content_type,
                req.cache_control,
                req.metadata,
            )
            .await?;
        Ok(PutResp {
            etag: stored.etag,
            size: stored.size,
            version_id: None,
        })
    }

    async fn get(&self, req: GetReq) -> Result<GetResp, BackendError> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        reject_version(req.version_id.as_deref())?;
        // Keep an overwrite from retiring the referenced data file between
        // reading its metadata and opening its bytes.
        let (metadata, file) = {
            let _guard = self.store.commit_lock.lock().await;
            let metadata = self.metadata(&req.key).await?;
            let (start, end) = match req.range {
                Some(range) => range.span(metadata.size)?,
                None => (0, metadata.size),
            };
            ByteRange::check_cap(start, end, req.max_inline_bytes)?;
            let data_path = self.store.data_path(&metadata.data_file);
            let file = File::open(&data_path).await.map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    BackendError::NotFound
                } else {
                    provider_error(format!("read object {}: {error}", data_path.display()))
                }
            })?;
            (metadata, (file, start, end))
        };
        // Once open, the descriptor keeps this version readable even if an
        // overwrite retires its directory entry.
        let (mut file, start, end) = file;
        if start > 0 {
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|error| provider_error(format!("seek local object: {error}")))?;
        }
        let mut body = Vec::with_capacity((end - start) as usize);
        file.take(end - start)
            .read_to_end(&mut body)
            .await
            .map_err(|error| provider_error(format!("read local object: {error}")))?;
        Ok(GetResp {
            body,
            content_type: metadata.content_type,
            etag: metadata.etag,
            last_modified: metadata.last_modified,
            size: metadata.size,
            offset: start,
        })
    }

    async fn head(&self, req: HeadReq) -> Result<HeadResp, BackendError> {
        reject_version(req.version_id.as_deref())?;
        let metadata = self.metadata(&req.key).await?;
        Ok(HeadResp {
            content_type: metadata.content_type,
            etag: metadata.etag,
            last_modified: metadata.last_modified,
            size: metadata.size,
        })
    }

    async fn delete(&self, req: DeleteReq) -> Result<DeleteResp, BackendError> {
        reject_version(req.version_id.as_deref())?;
        let metadata_path = self.store.metadata_path(&req.key);
        let _guard = self.store.commit_lock.lock().await;
        let existed = tokio::fs::try_exists(&metadata_path)
            .await
            .map_err(|error| provider_error(format!("stat local object: {error}")))?;
        if !existed {
            return Ok(DeleteResp { deleted: false });
        }
        let metadata = self.metadata(&req.key).await?;
        let data_path = self.store.data_path(&metadata.data_file);
        tokio::fs::remove_file(&data_path).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                BackendError::NotFound
            } else {
                provider_error(format!("delete object {}: {error}", data_path.display()))
            }
        })?;
        tokio::fs::remove_file(&metadata_path)
            .await
            .map_err(|error| provider_error(format!("delete object metadata: {error}")))?;
        drop(_guard);
        self.emit_deleted(req.key);
        Ok(DeleteResp { deleted: true })
    }

    async fn list(&self, req: ListReq) -> Result<ListResp, BackendError> {
        let all = self.store.all_metadata().await?;
        let delimiter = req.delimiter.as_deref();
        let mut entries: BTreeMap<(String, u8), ListEntry> = BTreeMap::new();
        for metadata in all {
            if !metadata.key.starts_with(&req.prefix) {
                continue;
            }
            let remainder = &metadata.key[req.prefix.len()..];
            if let Some(delimiter) = delimiter {
                if let Some(index) = remainder.find(delimiter) {
                    let prefix = format!("{}{}", req.prefix, &remainder[..index + delimiter.len()]);
                    entries
                        .entry((prefix.clone(), 0))
                        .or_insert(ListEntry::Prefix(prefix));
                    continue;
                }
            }
            let key = metadata.key.clone();
            entries.insert((key, 1), ListEntry::Object(metadata));
        }

        let cursor = req.cursor.as_deref().map(decode_cursor).transpose()?;
        let mut filtered = entries
            .into_iter()
            .filter(|(entry_cursor, _)| cursor.as_ref().is_none_or(|value| entry_cursor > value));
        let mut selected = Vec::new();
        for _ in 0..req.limit.max(1) {
            if let Some(entry) = filtered.next() {
                selected.push(entry);
            } else {
                break;
            }
        }
        let has_more = filtered.next().is_some();
        let next_cursor = has_more
            .then(|| selected.last().map(|(cursor, _)| encode_cursor(cursor)))
            .flatten();
        let mut objects = Vec::new();
        let mut common_prefixes = Vec::new();
        for (_, entry) in selected {
            match entry {
                ListEntry::Prefix(prefix) => common_prefixes.push(prefix),
                ListEntry::Object(metadata) => objects.push(ObjectSummary {
                    key: metadata.key,
                    etag: metadata.etag,
                    size: metadata.size,
                    last_modified: metadata.last_modified,
                    content_type: Some(metadata.content_type),
                }),
            }
        }
        Ok(ListResp {
            objects,
            common_prefixes,
            next_cursor,
        })
    }

    async fn presign(&self, req: PresignReq) -> Result<PresignResp, BackendError> {
        let expires_at = Utc::now() + chrono::Duration::seconds(req.expires_in_seconds as i64);
        let kind = match req.method {
            PresignMethod::Get => SignedKind::Download,
            PresignMethod::Put => SignedKind::Put,
        };
        let operation = SignedOperation {
            kind,
            bucket: self.store.worker_bucket.clone(),
            key: req.key,
            expires_at: expires_at.timestamp(),
            content_type: req.content_type,
            max_size_bytes: None,
            response_content_disposition: req.response_content_disposition,
            response_content_type: req.response_content_type,
        };
        let route = if kind == SignedKind::Download {
            "/v1/objects/download"
        } else {
            "/v1/objects/upload"
        };
        Ok(PresignResp {
            url: self.service.signed_url(route, &operation)?,
            expires_at,
        })
    }

    async fn presign_post(&self, req: PresignPostReq) -> Result<PresignPostResp, BackendError> {
        let expires_at = Utc::now() + chrono::Duration::seconds(req.expires_in_seconds as i64);
        let operation = SignedOperation {
            kind: SignedKind::Post,
            bucket: self.store.worker_bucket.clone(),
            key: req.key.clone(),
            expires_at: expires_at.timestamp(),
            content_type: Some(req.content_type.clone()),
            max_size_bytes: req.max_size_bytes,
            response_content_disposition: None,
            response_content_type: None,
        };
        Ok(PresignPostResp {
            url: self.service.signed_url("/v1/objects/upload", &operation)?,
            fields: HashMap::from([
                ("key".to_string(), req.key),
                ("Content-Type".to_string(), req.content_type),
            ]),
            expires_at,
        })
    }

    fn provider(&self) -> &'static str {
        "local"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SignedKind {
    Download,
    Put,
    Post,
}

#[derive(Debug, Serialize, Deserialize)]
struct SignedOperation {
    kind: SignedKind,
    bucket: String,
    key: String,
    expires_at: i64,
    content_type: Option<String>,
    max_size_bytes: Option<u64>,
    response_content_disposition: Option<String>,
    response_content_type: Option<String>,
}

#[derive(Deserialize)]
struct TokenQuery {
    token: String,
}

async fn download_handler(
    State(state): State<LocalHttpState>,
    Query(query): Query<TokenQuery>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let Some(service) = active_service(&state).await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let operation = match service.verify_token(&query.token, SignedKind::Download) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    let backend = match service.resolve_backend(&operation.bucket) {
        Some(value) => value,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    // Open the version referenced by the metadata while overwrites are
    // excluded. Once open, the stream owns the file descriptor even if a
    // later overwrite retires the old directory entry.
    let (metadata, file) = {
        let _guard = backend.store.commit_lock.lock().await;
        let metadata = match backend.metadata(&operation.key).await {
            Ok(value) => value,
            Err(error) => return backend_error_response(error),
        };
        let data_path = backend.store.data_path(&metadata.data_file);
        let file = match File::open(&data_path).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return StatusCode::NOT_FOUND.into_response();
            }
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        (metadata, file)
    };
    let content_type = operation
        .response_content_type
        .as_deref()
        .unwrap_or(&metadata.content_type);
    let last_modified = metadata.last_modified.to_rfc2822();

    // `If-Range` only lets the range through when it names this version.
    let range_header = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let if_range_matches = headers
        .get(header::IF_RANGE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| {
            let value = value.trim();
            value == metadata.etag || value == last_modified
        });
    let range = match range_header.filter(|_| if_range_matches) {
        Some(value) => parse_range_header(value, metadata.size),
        None => RangeRequest::Full,
    };
    let (status, start, length) = match range {
        RangeRequest::Full => (StatusCode::OK, 0, metadata.size),
        RangeRequest::Partial { start, end } => {
            (StatusCode::PARTIAL_CONTENT, start, end - start + 1)
        }
        RangeRequest::Unsatisfiable => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", metadata.size))
                .header(header::ACCEPT_RANGES, "bytes")
                .body(Body::empty())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };

    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, length.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, metadata.etag.clone())
        .header(header::LAST_MODIFIED, last_modified);
    if status == StatusCode::PARTIAL_CONTENT {
        response = response.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{}/{}", start + length - 1, metadata.size),
        );
    }
    if let Some(disposition) = operation.response_content_disposition {
        let Ok(value) = HeaderValue::from_str(&disposition) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        response = response.header(header::CONTENT_DISPOSITION, value);
    }
    if method == Method::HEAD {
        return response
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let mut file = file;
    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    response
        .body(Body::from_stream(ReaderStream::new(file.take(length))))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// What a `Range` request header asks of an object of a given size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeRequest {
    /// No usable range: answer 200 with the whole object. Also for a
    /// malformed header, another unit, and several ranges (`bytes=0-1,5-6`):
    /// RFC 9110 lets a server ignore Range, and players never ask for more
    /// than one.
    Full,
    /// `start..=end`, clamped to the object.
    Partial { start: u64, end: u64 },
    /// 416 with `Content-Range: bytes */size`: the range starts at or past
    /// the end, or is the empty suffix `bytes=-0` (any range of an empty
    /// object).
    Unsatisfiable,
}

/// Parse `bytes=a-b`, `bytes=a-` and `bytes=-n` against `size`.
pub fn parse_range_header(value: &str, size: u64) -> RangeRequest {
    let Some((unit, spec)) = value.trim().split_once('=') else {
        return RangeRequest::Full;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return RangeRequest::Full;
    }
    let spec = spec.trim();
    if spec.contains(',') {
        return RangeRequest::Full;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return RangeRequest::Full;
    };
    let (first, last) = (first.trim(), last.trim());
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match (first.is_empty(), last.is_empty()) {
        // `-n`: the last n bytes.
        (true, false) => {
            let Some(n) = digits(last).then(|| last.parse::<u64>().ok()).flatten() else {
                return RangeRequest::Full;
            };
            if n == 0 || size == 0 {
                return RangeRequest::Unsatisfiable;
            }
            RangeRequest::Partial {
                start: size.saturating_sub(n),
                end: size - 1,
            }
        }
        // `a-` or `a-b`.
        (false, _) => {
            let Some(start) = digits(first).then(|| first.parse::<u64>().ok()).flatten() else {
                return RangeRequest::Full;
            };
            let end = if last.is_empty() {
                None
            } else {
                match digits(last).then(|| last.parse::<u64>().ok()).flatten() {
                    Some(end) if end >= start => Some(end),
                    // `b < a` or garbage: an invalid range set is ignored.
                    _ => return RangeRequest::Full,
                }
            };
            if start >= size {
                return RangeRequest::Unsatisfiable;
            }
            RangeRequest::Partial {
                start,
                end: end.map_or(size - 1, |end| end.min(size - 1)),
            }
        }
        (true, true) => RangeRequest::Full,
    }
}

async fn put_upload_handler(
    State(state): State<LocalHttpState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(service) = active_service(&state).await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let operation = match service.verify_token(&query.token, SignedKind::Put) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    let backend = match service.resolve_backend(&operation.bucket) {
        Some(value) => value,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let content_type = operation
        .content_type
        .unwrap_or_else(|| "application/octet-stream".to_string());
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some(content_type.as_str())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match backend
        .stream_into_pending(
            body.into_data_stream(),
            operation.key,
            content_type,
            operation.max_size_bytes,
        )
        .await
    {
        Ok(stored) => upload_response(stored),
        Err(error) => backend_error_response(error),
    }
}

async fn post_upload_handler(
    State(state): State<LocalHttpState>,
    Query(query): Query<TokenQuery>,
    mut multipart: Multipart,
) -> Response {
    let Some(service) = active_service(&state).await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let operation = match service.verify_token(&query.token, SignedKind::Post) {
        Ok(value) => value,
        Err(status) => return status.into_response(),
    };
    let backend = match service.resolve_backend(&operation.bucket) {
        Some(value) => value,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let expected_content_type = operation
        .content_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let mut form_key = None;
    let mut form_content_type = None;
    let mut stored = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(value)) => value,
            Ok(None) => break,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "key" => match field.text().await {
                Ok(value) => form_key = Some(value),
                Err(_) => return StatusCode::BAD_REQUEST.into_response(),
            },
            "Content-Type" => match field.text().await {
                Ok(value) => form_content_type = Some(value),
                Err(_) => return StatusCode::BAD_REQUEST.into_response(),
            },
            "file" => {
                if stored.is_some() {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                if form_key.as_deref() != Some(operation.key.as_str())
                    || form_content_type.as_deref() != Some(expected_content_type.as_str())
                {
                    // Require policy fields before `file`, matching the fields
                    // order returned by presignPost and preventing an invalid
                    // multipart request from committing any bytes.
                    return StatusCode::BAD_REQUEST.into_response();
                }
                match backend
                    .stream_into_pending(
                        field,
                        operation.key.clone(),
                        expected_content_type.clone(),
                        operation.max_size_bytes,
                    )
                    .await
                {
                    Ok(value) => stored = Some(value),
                    Err(error) => return backend_error_response(error),
                }
            }
            _ => {}
        }
    }
    if form_key.as_deref() != Some(operation.key.as_str())
        || form_content_type.as_deref() != Some(expected_content_type.as_str())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match stored {
        Some(value) => upload_response(value),
        None => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn active_service(state: &LocalHttpState) -> Option<Arc<LocalService>> {
    let _gate = state.reconfigure_gate.read().await;
    state.active.read().await.clone()
}

fn upload_response(stored: StoredMetadata) -> Response {
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "etag": stored.etag,
            "size": stored.size,
            "key": stored.key,
        })),
    )
        .into_response()
}

fn backend_error_response(error: BackendError) -> Response {
    match error {
        BackendError::NotFound => StatusCode::NOT_FOUND.into_response(),
        BackendError::ObjectTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        BackendError::AuthFailed(_) => StatusCode::UNAUTHORIZED.into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

enum ListEntry {
    Prefix(String),
    Object(StoredMetadata),
}

fn scan_metadata(root: &Path) -> Result<Vec<StoredMetadata>, BackendError> {
    let mut out = Vec::new();
    let Ok(shards) = std::fs::read_dir(root) else {
        return Ok(out);
    };
    for shard in shards.flatten() {
        let Ok(kind) = shard.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(shard.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<StoredMetadata>(&bytes)
                        .map_err(|error| error.to_string())
                }) {
                Ok(metadata) => out.push(metadata),
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "skipping corrupt local object metadata")
                }
            }
        }
    }
    Ok(out)
}

fn encode_cursor(cursor: &(String, u8)) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).expect("cursor serializes"))
}

fn decode_cursor(cursor: &str) -> Result<(String, u8), BackendError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| provider_error("invalid local list cursor"))?;
    serde_json::from_slice(&bytes).map_err(|_| provider_error("invalid local list cursor"))
}

fn reject_version(version_id: Option<&str>) -> Result<(), BackendError> {
    if version_id.is_some() {
        return Err(BackendError::Provider {
            inner_code: Some("LOCAL_VERSIONS_UNSUPPORTED".into()),
            message: "the native local backend does not support object versions".into(),
        });
    }
    Ok(())
}

fn normalize_public_url(input: &str) -> Result<String, BackendError> {
    let parsed = Url::parse(input)
        .map_err(|error| provider_error(format!("invalid local public_url `{input}`: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(provider_error("local public_url must use http or https"));
    }
    Ok(input.trim_end_matches('/').to_string())
}

fn browser_address(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port())
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port())
        }
        _ => addr,
    }
}

fn random_event_id() -> String {
    format!("local-{:016x}", rand::random::<u64>())
}

fn provider_error(message: impl Into<String>) -> BackendError {
    BackendError::Provider {
        inner_code: Some("LOCAL_IO".into()),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::dispatcher::EventDispatcher;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingDispatcher(StdMutex<Vec<ObjectEventNormalized>>);

    #[async_trait::async_trait]
    impl EventDispatcher for RecordingDispatcher {
        async fn dispatch(&self, event: ObjectEventNormalized) -> bool {
            self.0.lock().unwrap().push(event);
            true
        }
    }

    async fn backend(http: bool) -> (tempfile::TempDir, Arc<dyn Backend>, PreparedLocalService) {
        let temp = tempfile::tempdir().unwrap();
        let config = LocalProviderConfig {
            data_dir: temp.path().to_string_lossy().to_string(),
            http: http.then(|| crate::config::LocalHttpConfig {
                bind_address: "127.0.0.1:0".into(),
                public_url: None,
            }),
        };
        let prepared = prepare(Some(&config), Arc::new(RecordingDispatcher::default()))
            .await
            .unwrap();
        let backend = build(&prepared.context, "scratch".into(), "scratch".into()).unwrap();
        (temp, backend, prepared)
    }

    #[tokio::test]
    async fn round_trip_and_directory_listing() {
        let (_temp, backend, _prepared) = backend(false).await;
        backend
            .put(PutReq {
                key: "reports/2026/a.txt".into(),
                body: b"hello".to_vec(),
                content_type: "text/plain".into(),
                cache_control: None,
                metadata: HashMap::new(),
            })
            .await
            .unwrap();
        let got = backend
            .get(GetReq {
                key: "reports/2026/a.txt".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(got.body, b"hello");
        let root = backend
            .list(ListReq {
                prefix: String::new(),
                delimiter: Some("/".into()),
                cursor: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert_eq!(root.common_prefixes, vec!["reports/"]);
        let reports = backend
            .list(ListReq {
                prefix: "reports/2026/".into(),
                delimiter: Some("/".into()),
                cursor: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert_eq!(reports.objects[0].key, "reports/2026/a.txt");
    }

    #[tokio::test]
    async fn signed_post_and_download_stream() {
        let (_temp, backend, mut prepared) = backend(true).await;
        let handle = start_http(&mut prepared).unwrap();
        let post = backend
            .presign_post(PresignPostReq {
                key: "large.bin".into(),
                content_type: "application/octet-stream".into(),
                expires_in_seconds: 60,
                max_size_bytes: None,
            })
            .await
            .unwrap();
        let form = reqwest::multipart::Form::new()
            .text("key", post.fields["key"].clone())
            .text("Content-Type", post.fields["Content-Type"].clone())
            .part(
                "file",
                reqwest::multipart::Part::bytes(b"streamed".to_vec())
                    .mime_str("application/octet-stream")
                    .unwrap(),
            );
        let upload = reqwest::Client::new()
            .post(post.url)
            .header(header::ORIGIN, "http://127.0.0.1:3113")
            .multipart(form)
            .send()
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::CREATED);
        assert_eq!(
            upload
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("*")
        );

        let download = backend
            .presign(PresignReq {
                key: "large.bin".into(),
                method: PresignMethod::Get,
                content_type: None,
                expires_in_seconds: 60,
                response_content_disposition: None,
                response_content_type: None,
            })
            .await
            .unwrap();
        let bytes = reqwest::get(download.url)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"streamed");
        handle.shutdown();
    }

    #[test]
    fn range_headers_parse_with_their_edge_cases() {
        use RangeRequest::*;
        assert_eq!(
            parse_range_header("bytes=0-9", 100),
            Partial { start: 0, end: 9 }
        );
        assert_eq!(
            parse_range_header("bytes=90-", 100),
            Partial { start: 90, end: 99 }
        );
        assert_eq!(
            parse_range_header("bytes=-10", 100),
            Partial { start: 90, end: 99 }
        );
        assert_eq!(
            parse_range_header("bytes=-1000", 100),
            Partial { start: 0, end: 99 }
        );
        assert_eq!(
            parse_range_header("bytes=50-1000", 100),
            Partial { start: 50, end: 99 }
        );
        assert_eq!(
            parse_range_header("bytes=99-99", 100),
            Partial { start: 99, end: 99 }
        );
        assert_eq!(
            parse_range_header(" Bytes = 1 - 2 ", 100),
            Partial { start: 1, end: 2 }
        );
        // Unsatisfiable: the empty suffix, a start at or past the end, any
        // range of an empty object.
        assert_eq!(parse_range_header("bytes=-0", 100), Unsatisfiable);
        assert_eq!(parse_range_header("bytes=100-", 100), Unsatisfiable);
        assert_eq!(parse_range_header("bytes=100-200", 100), Unsatisfiable);
        assert_eq!(parse_range_header("bytes=0-", 0), Unsatisfiable);
        assert_eq!(parse_range_header("bytes=-5", 0), Unsatisfiable);
        // Ignored (the whole object): several ranges, another unit, junk,
        // an end before the start.
        assert_eq!(parse_range_header("bytes=0-1,5-6", 100), Full);
        assert_eq!(parse_range_header("items=0-1", 100), Full);
        assert_eq!(parse_range_header("bytes=a-b", 100), Full);
        assert_eq!(parse_range_header("bytes=5-2", 100), Full);
        assert_eq!(parse_range_header("bytes=-", 100), Full);
        assert_eq!(parse_range_header("bytes=+1-2", 100), Full);
        assert_eq!(parse_range_header("bytes", 100), Full);
        assert_eq!(
            parse_range_header("bytes=99999999999999999999-", 100),
            Full,
            "an unparseable number is junk"
        );
    }

    async fn put_text(backend: &Arc<dyn Backend>, key: &str, body: &[u8]) {
        backend
            .put(PutReq {
                key: key.into(),
                body: body.to_vec(),
                content_type: "video/mp4".into(),
                cache_control: None,
                metadata: HashMap::new(),
            })
            .await
            .unwrap();
    }

    async fn download_url(backend: &Arc<dyn Backend>, key: &str) -> String {
        backend
            .presign(PresignReq {
                key: key.into(),
                method: PresignMethod::Get,
                content_type: None,
                expires_in_seconds: 60,
                response_content_disposition: Some("attachment; filename=\"v.mp4\"".into()),
                response_content_type: None,
            })
            .await
            .unwrap()
            .url
    }

    #[tokio::test]
    async fn signed_downloads_serve_ranges_and_head() {
        let (_temp, backend, mut prepared) = backend(true).await;
        let handle = start_http(&mut prepared).unwrap();
        put_text(&backend, "v.mp4", b"0123456789").await;
        let url = download_url(&backend, "v.mp4").await;
        let client = reqwest::Client::new();

        let full = client.get(&url).send().await.unwrap();
        assert_eq!(full.status(), StatusCode::OK);
        assert_eq!(full.headers()[header::ACCEPT_RANGES], "bytes");
        assert_eq!(&full.bytes().await.unwrap()[..], b"0123456789");

        let part = client
            .get(&url)
            .header(header::RANGE, "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(part.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(part.headers()[header::CONTENT_LENGTH], "4");
        assert_eq!(
            part.headers()[header::CONTENT_DISPOSITION],
            "attachment; filename=\"v.mp4\""
        );
        assert_eq!(&part.bytes().await.unwrap()[..], b"2345");

        let tail = client
            .get(&url)
            .header(header::RANGE, "bytes=-3")
            .send()
            .await
            .unwrap();
        assert_eq!(tail.headers()[header::CONTENT_RANGE], "bytes 7-9/10");
        assert_eq!(&tail.bytes().await.unwrap()[..], b"789");

        let open = client
            .get(&url)
            .header(header::RANGE, "bytes=8-")
            .send()
            .await
            .unwrap();
        assert_eq!(&open.bytes().await.unwrap()[..], b"89");

        let past = client
            .get(&url)
            .header(header::RANGE, "bytes=10-")
            .send()
            .await
            .unwrap();
        assert_eq!(past.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(past.headers()[header::CONTENT_RANGE], "bytes */10");

        let multi = client
            .get(&url)
            .header(header::RANGE, "bytes=0-1,4-5")
            .send()
            .await
            .unwrap();
        assert_eq!(
            multi.status(),
            StatusCode::OK,
            "several ranges get the whole object"
        );
        assert_eq!(multi.bytes().await.unwrap().len(), 10);

        // If-Range with another version's ETag: the whole (current) object.
        let stale = client
            .get(&url)
            .header(header::RANGE, "bytes=0-1")
            .header(header::IF_RANGE, "\"not-this-version\"")
            .send()
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::OK);
        let etag = stale.headers()[header::ETAG].to_str().unwrap().to_string();
        let fresh = client
            .get(&url)
            .header(header::RANGE, "bytes=0-1")
            .header(header::IF_RANGE, etag)
            .send()
            .await
            .unwrap();
        assert_eq!(fresh.status(), StatusCode::PARTIAL_CONTENT);

        let head = client.head(&url).send().await.unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::CONTENT_LENGTH], "10");
        assert_eq!(head.headers()[header::ACCEPT_RANGES], "bytes");
        assert!(head.bytes().await.unwrap().is_empty());
        let head_part = client
            .head(&url)
            .header(header::RANGE, "bytes=0-0")
            .send()
            .await
            .unwrap();
        assert_eq!(head_part.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(head_part.headers()[header::CONTENT_RANGE], "bytes 0-0/10");

        // An empty object: whole reads work, every range is unsatisfiable.
        put_text(&backend, "empty", b"").await;
        let url = download_url(&backend, "empty").await;
        let whole = client.get(&url).send().await.unwrap();
        assert_eq!(whole.status(), StatusCode::OK);
        let ranged = client
            .get(&url)
            .header(header::RANGE, "bytes=0-")
            .send()
            .await
            .unwrap();
        assert_eq!(ranged.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(ranged.headers()[header::CONTENT_RANGE], "bytes */0");

        // A CORS preflight may ask for Range.
        let preflight = client
            .request(Method::OPTIONS, &url)
            .header(header::ORIGIN, "http://127.0.0.1:3113")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "range")
            .send()
            .await
            .unwrap();
        assert!(preflight.status().is_success());
        handle.shutdown();
    }

    #[tokio::test]
    async fn ranged_gets_seek_and_respect_bounds() {
        let (_temp, backend, _prepared) = backend(false).await;
        put_text(&backend, "k", b"0123456789").await;
        let read = |offset: u64, length: Option<u64>, cap: Option<u64>| {
            let backend = backend.clone();
            async move {
                backend
                    .get(GetReq {
                        key: "k".into(),
                        version_id: None,
                        max_inline_bytes: cap,
                        range: Some(ByteRange { offset, length }),
                    })
                    .await
            }
        };
        let got = read(3, Some(4), None).await.unwrap();
        assert_eq!(
            (got.body.as_slice(), got.offset, got.size),
            (&b"3456"[..], 3, 10)
        );
        assert_eq!(read(8, Some(50), None).await.unwrap().body, b"89");
        assert_eq!(read(6, None, None).await.unwrap().body, b"6789");
        assert!(read(10, None, None).await.unwrap().body.is_empty());
        assert!(read(4, Some(0), None).await.unwrap().body.is_empty());
        assert!(matches!(
            read(11, None, None).await,
            Err(BackendError::InvalidRange {
                offset: 11,
                size: 10
            })
        ));
        // The cap is on the bytes returned.
        assert_eq!(read(0, Some(4), Some(4)).await.unwrap().body, b"0123");
        assert!(matches!(
            read(0, None, Some(4)).await,
            Err(BackendError::ObjectTooLarge {
                actual_size: 10,
                cap: 4
            })
        ));
    }

    #[tokio::test]
    async fn signed_post_enforces_streaming_size_cap_without_committing() {
        let (_temp, backend, mut prepared) = backend(true).await;
        let handle = start_http(&mut prepared).unwrap();
        let post = backend
            .presign_post(PresignPostReq {
                key: "limited.bin".into(),
                content_type: "application/octet-stream".into(),
                expires_in_seconds: 60,
                max_size_bytes: Some(4),
            })
            .await
            .unwrap();
        let form = reqwest::multipart::Form::new()
            .text("key", post.fields["key"].clone())
            .text("Content-Type", post.fields["Content-Type"].clone())
            .part(
                "file",
                reqwest::multipart::Part::bytes(b"too large".to_vec())
                    .mime_str("application/octet-stream")
                    .unwrap(),
            );
        let upload = reqwest::Client::new()
            .post(post.url)
            .multipart(form)
            .send()
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(matches!(
            backend
                .head(HeadReq {
                    key: "limited.bin".into(),
                    version_id: None,
                })
                .await,
            Err(BackendError::NotFound)
        ));
        handle.shutdown();
    }

    #[tokio::test]
    async fn runtime_keeps_socket_for_same_bind_and_updates_public_url() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let gate = Arc::new(RwLock::new(()));
        let runtime = LocalRuntime::new(Arc::new(RecordingDispatcher::default()), gate.clone());
        let first = LocalProviderConfig {
            data_dir: first_dir.path().to_string_lossy().to_string(),
            http: Some(crate::config::LocalHttpConfig {
                bind_address: "127.0.0.1:0".into(),
                public_url: None,
            }),
        };
        let prepared = runtime.prepare_update(Some(&first), true).await.unwrap();
        let backend = build(
            prepared.context().unwrap(),
            "scratch".into(),
            "scratch".into(),
        )
        .unwrap();
        {
            let _publish = gate.write().await;
            runtime.commit(prepared).await;
        }
        let first_addr = runtime.current_addr().await.unwrap();
        let first_url = backend
            .presign(PresignReq {
                key: "first.txt".into(),
                method: PresignMethod::Get,
                content_type: None,
                expires_in_seconds: 60,
                response_content_disposition: None,
                response_content_type: None,
            })
            .await
            .unwrap()
            .url;
        assert!(first_url.starts_with(&format!("http://{first_addr}/")));

        let second = LocalProviderConfig {
            data_dir: second_dir.path().to_string_lossy().to_string(),
            http: Some(crate::config::LocalHttpConfig {
                bind_address: "127.0.0.1:0".into(),
                public_url: Some("http://storage.vpn.test:49200".into()),
            }),
        };
        let prepared = runtime.prepare_update(Some(&second), true).await.unwrap();
        let backend = build(
            prepared.context().unwrap(),
            "scratch".into(),
            "scratch".into(),
        )
        .unwrap();
        assert!(prepared.keep_listener);
        {
            let _publish = gate.write().await;
            runtime.commit(prepared).await;
        }
        assert_eq!(runtime.current_addr().await, Some(first_addr));
        let second_url = backend
            .presign(PresignReq {
                key: "second.txt".into(),
                method: PresignMethod::Get,
                content_type: None,
                expires_in_seconds: 60,
                response_content_disposition: None,
                response_content_type: None,
            })
            .await
            .unwrap()
            .url;
        assert!(second_url.starts_with("http://storage.vpn.test:49200/"));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn failed_rebind_keeps_the_running_listener() {
        let data_dir = tempfile::tempdir().unwrap();
        let gate = Arc::new(RwLock::new(()));
        let runtime = LocalRuntime::new(Arc::new(RecordingDispatcher::default()), gate.clone());
        let initial = LocalProviderConfig {
            data_dir: data_dir.path().to_string_lossy().to_string(),
            http: Some(crate::config::LocalHttpConfig {
                bind_address: "127.0.0.1:0".into(),
                public_url: None,
            }),
        };
        let prepared = runtime.prepare_update(Some(&initial), true).await.unwrap();
        build(
            prepared.context().unwrap(),
            "scratch".into(),
            "scratch".into(),
        )
        .unwrap();
        {
            let _publish = gate.write().await;
            runtime.commit(prepared).await;
        }
        let running_addr = runtime.current_addr().await.unwrap();

        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let occupied_addr = occupied.local_addr().unwrap();
        let rejected = LocalProviderConfig {
            data_dir: data_dir.path().to_string_lossy().to_string(),
            http: Some(crate::config::LocalHttpConfig {
                bind_address: occupied_addr.to_string(),
                public_url: None,
            }),
        };
        assert!(runtime.prepare_update(Some(&rejected), true).await.is_err());
        assert_eq!(runtime.current_addr().await, Some(running_addr));
        runtime.shutdown().await;
    }
}
