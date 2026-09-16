use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct MemoryStore {
    value: Mutex<Option<SessionRecord>>,
    fail: AtomicBool,
    fail_write: AtomicBool,
    conflict_next_write: AtomicBool,
}

impl SessionStore for MemoryStore {
    fn load(&self) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionError>> {
        Box::pin(async {
            if self.fail.load(Ordering::SeqCst) {
                return Err(SessionError::storage());
            }
            Ok(self.value.lock().await.clone())
        })
    }
    fn compare_and_set<'a>(
        &'a self,
        expected: Option<&'a SessionRecord>,
        next: &'a SessionRecord,
    ) -> BoxFuture<'a, Result<bool, SessionError>> {
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) || self.fail_write.load(Ordering::SeqCst) {
                return Err(SessionError::storage());
            }
            let mut stored = self.value.lock().await;
            if stored.as_ref().map(|r| &r.revision) != expected.map(|r| &r.revision) {
                return Ok(false);
            }
            if self.conflict_next_write.swap(false, Ordering::SeqCst) {
                if let Some(record) = stored.as_mut() {
                    record.revision = uuid::Uuid::new_v4().to_string();
                }
                return Ok(false);
            }
            *stored = Some(next.clone());
            Ok(true)
        })
    }
}

#[derive(Default)]
struct FakeOAuth {
    refreshes: AtomicUsize,
    polled: AtomicUsize,
    deny_refresh: AtomicBool,
    refresh_started: tokio::sync::Notify,
    release_refresh: tokio::sync::Notify,
    pause_refresh: AtomicBool,
    pending: AtomicBool,
    transient_refresh: AtomicBool,
    preserve_access_token: AtomicBool,
    malformed_refresh: AtomicBool,
}

fn credential(account: &str, ttl: i64) -> Credential {
    Credential {
        access_token: format!("secret-{account}"),
        refresh_token: Some("refresh-secret".into()),
        id_token: Some("identity-secret".into()),
        account_id: account.into(),
        expires_at: now_seconds() + ttl,
    }
}

impl OAuthTransport for FakeOAuth {
    fn start(&self) -> BoxFuture<'_, Result<DeviceCode, OAuthError>> {
        Box::pin(async {
            Ok(DeviceCode {
                device_auth_id: "device-secret".into(),
                user_code: "ABCD-1234".into(),
                verification_uri: "https://auth.openai.com/codex/device".into(),
                interval: 1,
                expires_at: now_seconds() + 900,
            })
        })
    }
    fn poll<'a>(&'a self, _: &'a DeviceCode) -> BoxFuture<'a, Result<PollOutcome, OAuthError>> {
        Box::pin(async {
            self.polled.fetch_add(1, Ordering::SeqCst);
            if self.pending.load(Ordering::SeqCst) {
                return Ok(PollOutcome::Pending {
                    retry_after_secs: 1,
                });
            }
            Ok(PollOutcome::Authorized(credential("new-account", 3600)))
        })
    }
    fn refresh<'a>(&'a self, old: &'a Credential) -> BoxFuture<'a, Result<Credential, OAuthError>> {
        Box::pin(async move {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            self.refresh_started.notify_one();
            if self.pause_refresh.load(Ordering::SeqCst) {
                self.release_refresh.notified().await;
            }
            if self.transient_refresh.load(Ordering::SeqCst) {
                return Err(OAuthError {
                    code: "network_error".into(),
                    message: "Try again.".into(),
                    permanent: false,
                    retry_after_secs: None,
                });
            }
            if self.deny_refresh.load(Ordering::SeqCst) {
                return Err(OAuthError {
                    code: "refresh_token_revoked".into(),
                    message: "Sign in again.".into(),
                    permanent: true,
                    retry_after_secs: None,
                });
            }
            if self.malformed_refresh.load(Ordering::SeqCst) {
                return Err(OAuthError {
                    code: "malformed_response".into(),
                    message: "Could not read the token response. Try again.".into(),
                    permanent: true,
                    retry_after_secs: None,
                });
            }
            let mut fresh = credential(&old.account_id, 3600);
            fresh.access_token = if self.preserve_access_token.load(Ordering::SeqCst) {
                old.access_token.clone()
            } else {
                "rotated-access".into()
            };
            fresh.refresh_token = Some("rotated-refresh".into());
            Ok(fresh)
        })
    }
}

#[derive(Default)]
struct FakeLegacy {
    reads: AtomicUsize,
}
impl LegacySource for FakeLegacy {
    fn resolve(&self) -> BoxFuture<'_, Result<Option<ResolvedCredential>, SessionError>> {
        Box::pin(async {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(Some(ResolvedCredential {
                value: serde_json::to_value(credential("legacy", 3600)).unwrap(),
                source: CredentialSource::Local,
            }))
        })
    }
}

fn setup() -> (
    Arc<AuthManager>,
    Arc<MemoryStore>,
    Arc<FakeOAuth>,
    Arc<FakeLegacy>,
) {
    let store = Arc::new(MemoryStore::default());
    let oauth = Arc::new(FakeOAuth::default());
    let legacy = Arc::new(FakeLegacy::default());
    let manager = AuthManager::with_dependencies(store.clone(), oauth.clone(), legacy.clone());
    (manager, store, oauth, legacy)
}

#[tokio::test]
async fn managed_session_survives_new_manager_and_logout_disables_legacy() {
    let (manager, store, oauth, legacy) = setup();
    assert_eq!(
        manager.resolve(None).await.unwrap().unwrap().source,
        CredentialSource::Local
    );
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
    let restarted = AuthManager::with_dependencies(store.clone(), oauth, legacy.clone());
    assert_eq!(
        restarted.status().await.unwrap().account_id.as_deref(),
        Some("account")
    );
    manager.logout().await.unwrap();
    assert!(restarted.resolve(None).await.unwrap().is_none());
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn storage_outage_never_falls_back_to_another_account() {
    let (manager, store, _, legacy) = setup();
    store.fail.store(true, Ordering::SeqCst);
    assert!(manager.resolve(None).await.is_err());
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn concurrent_requests_refresh_only_once_and_store_rotation() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 10)));
    let (a, b) = tokio::join!(manager.resolve(None), manager.resolve(None));
    assert_eq!(a.unwrap().unwrap().value["access_token"], "rotated-access");
    assert_eq!(b.unwrap().unwrap().value["access_token"], "rotated-access");
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .value
            .lock()
            .await
            .as_ref()
            .unwrap()
            .credential
            .as_ref()
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("rotated-refresh")
    );
}

#[tokio::test]
async fn forced_refresh_preserving_access_token_does_not_refresh_twice() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
    oauth.preserve_access_token.store(true, Ordering::SeqCst);
    let resolved = manager
        .resolve(Some("secret-account"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.value["access_token"], "secret-account");
    assert_eq!(resolved.value["refresh_token"], "rotated-refresh");
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn canceling_a_caller_does_not_cancel_token_rotation() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 10)));
    oauth.pause_refresh.store(true, Ordering::SeqCst);
    let caller = tokio::spawn({
        let manager = manager.clone();
        async move { manager.resolve(None).await }
    });
    oauth.refresh_started.notified().await;
    caller.abort();
    assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
    oauth.release_refresh.notify_one();
    let resolved = manager.resolve(None).await.unwrap().unwrap();
    assert_eq!(resolved.value["refresh_token"], "rotated-refresh");
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn authorized_login_retries_a_concurrent_refresh_write() {
    let (manager, store, _, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("old-account", 3600)));
    let login = manager.start().await.unwrap();
    store.conflict_next_write.store(true, Ordering::SeqCst);
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..20 {
        if manager.poll(&login.login_id).status != LoginStatus::Pending {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(manager.poll(&login.login_id).status, LoginStatus::Ok);
    assert_eq!(
        manager.status().await.unwrap().account_id.as_deref(),
        Some("new-account")
    );
}

#[tokio::test]
async fn logout_wins_over_an_inflight_refresh() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 10)));
    oauth.pause_refresh.store(true, Ordering::SeqCst);
    let refresh = tokio::spawn({
        let manager = manager.clone();
        async move { manager.resolve(None).await }
    });
    oauth.refresh_started.notified().await;
    manager.logout().await.unwrap();
    oauth.release_refresh.notify_one();
    assert!(refresh.await.unwrap().unwrap().is_none());
    assert_eq!(
        manager.status().await.unwrap().status,
        AuthStatus::SignedOut
    );
}

#[tokio::test]
async fn revoked_session_requires_login_and_never_uses_legacy() {
    let (manager, store, oauth, legacy) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 10)));
    oauth.deny_refresh.store(true, Ordering::SeqCst);
    assert_eq!(manager.status().await.unwrap().status, AuthStatus::Expired);
    assert!(manager.resolve(None).await.is_err());
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn protocol_failure_does_not_permanently_invalidate_the_session() {
    let (manager, store, oauth, legacy) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 10)));
    oauth.malformed_refresh.store(true, Ordering::SeqCst);
    assert!(
        matches!(manager.resolve(None).await, Err(error) if error.code == "malformed_response")
    );
    assert!(!store.value.lock().await.as_ref().unwrap().needs_login);
    oauth.malformed_refresh.store(false, Ordering::SeqCst);
    assert_eq!(
        manager.status().await.unwrap().status,
        AuthStatus::Authenticated
    );
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn login_is_backend_owned_and_persists_before_reporting_success() {
    let (manager, store, _, _) = setup();
    let mut changed = manager.subscribe();
    let login = manager.start().await.unwrap();
    assert_eq!(
        manager.status().await.unwrap().login.unwrap().login_id,
        login.login_id
    );
    changed.changed().await.unwrap();
    assert_eq!(manager.poll(&login.login_id).status, LoginStatus::Ok);
    assert_eq!(
        store
            .value
            .lock()
            .await
            .as_ref()
            .unwrap()
            .credential
            .as_ref()
            .unwrap()
            .account_id,
        "new-account"
    );
    let public = serde_json::to_string(&manager.status().await.unwrap()).unwrap();
    assert!(!public.contains("secret"));
}

#[tokio::test(start_paused = true)]
async fn cancel_or_logout_cannot_later_complete_a_login() {
    let (manager, store, oauth, _) = setup();
    let login = manager.start().await.unwrap();
    manager.cancel(&login.login_id).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    assert_eq!(manager.poll(&login.login_id).status, LoginStatus::Canceled);
    assert!(store.value.lock().await.is_none());
    let login = manager.start().await.unwrap();
    manager.logout().await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    assert_eq!(manager.poll(&login.login_id).status, LoginStatus::Canceled);
    assert_eq!(oauth.polled.load(Ordering::SeqCst), 0);
    assert!(manager.resolve(None).await.unwrap().is_none());
}

#[tokio::test]
async fn rotated_refresh_is_retained_until_storage_recovers() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 10)));
    store.fail_write.store(true, Ordering::SeqCst);
    assert_eq!(
        manager.resolve(None).await.err().unwrap().code,
        "storage_unavailable"
    );
    store.fail_write.store(false, Ordering::SeqCst);
    assert_eq!(
        manager.resolve(None).await.unwrap().unwrap().value["access_token"],
        "rotated-access"
    );
    assert_eq!(
        oauth.refreshes.load(Ordering::SeqCst),
        1,
        "never reuse a consumed refresh token"
    );
}

#[tokio::test]
async fn transient_refresh_failure_preserves_session() {
    let (manager, store, oauth, legacy) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 10)));
    oauth.transient_refresh.store(true, Ordering::SeqCst);
    assert_eq!(
        manager.resolve(None).await.err().unwrap().code,
        "network_error"
    );
    assert!(!store.value.lock().await.as_ref().unwrap().needs_login);
    oauth.transient_refresh.store(false, Ordering::SeqCst);
    assert!(manager.resolve(None).await.unwrap().is_some());
    assert_eq!(legacy.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn switching_keeps_old_account_until_login_succeeds() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("original", 3600)));
    oauth.pending.store(true, Ordering::SeqCst);
    let old_attempt = manager.start().await.unwrap();
    assert_eq!(
        manager.status().await.unwrap().account_id.as_deref(),
        Some("original")
    );
    let mut changed = manager.subscribe();
    let new_attempt = manager.start().await.unwrap();
    assert_eq!(
        manager.poll(&old_attempt.login_id).status,
        LoginStatus::Canceled
    );
    oauth.pending.store(false, Ordering::SeqCst);
    changed.changed().await.unwrap();
    assert_eq!(manager.poll(&new_attempt.login_id).status, LoginStatus::Ok);
    assert_eq!(
        manager.status().await.unwrap().account_id.as_deref(),
        Some("new-account")
    );
}

#[tokio::test(start_paused = true)]
async fn login_write_failure_is_not_reported_as_success() {
    let (manager, store, _, _) = setup();
    let login = manager.start().await.unwrap();
    store.fail_write.store(true, Ordering::SeqCst);
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(manager.poll(&login.login_id).status, LoginStatus::Error);
    assert_eq!(
        manager.poll(&login.login_id).error.unwrap().code,
        "storage_unavailable"
    );
    assert!(store.value.lock().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn pending_login_expires_and_does_not_replace_the_old_session() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("original", 3600)));
    oauth.pending.store(true, Ordering::SeqCst);
    let login = manager.start().await.unwrap();
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_secs(901)).await;
    assert_eq!(manager.poll(&login.login_id).status, LoginStatus::Expired);
    assert_eq!(
        manager.status().await.unwrap().account_id.as_deref(),
        Some("original")
    );
}

#[tokio::test]
async fn a_forced_refresh_reuses_a_token_already_rotated_by_another_request() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
    let (first, second) = tokio::join!(
        manager.resolve(Some("secret-account")),
        manager.resolve(Some("secret-account"))
    );
    assert_eq!(
        first.unwrap().unwrap().value["access_token"],
        "rotated-access"
    );
    assert_eq!(
        second.unwrap().unwrap().value["access_token"],
        "rotated-access"
    );
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
}

const STREAM_OK: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n";
const UNAUTHORIZED: &str = "HTTP/1.1 401 Unauthorized\r\nconnection: close\r\n\r\n{}";

async fn stream_requests(
    manager: Arc<AuthManager>,
    replies: Vec<&'static str>,
) -> (
    Vec<llm_router::types::events::AssistantMessageEvent>,
    Vec<String>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let server = tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            seen.lock().await.push(String::from_utf8(request).unwrap());
            socket.write_all(reply.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    let args = crate::upstream::UpstreamArgs {
        api_url: format!("http://{addr}/responses"),
        model: "codex/test".into(),
        body: serde_json::json!({"stream":true}),
        warnings: vec![],
        headers: vec![
            ("authorization", "Bearer secret-account".into()),
            ("chatgpt-account-id", "account".into()),
        ],
    };
    let mut rx =
        crate::upstream::spawn_authenticated_upstream(reqwest::Client::new(), args, manager);
    let mut events = Vec::new();
    while let Some(event) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
    {
        events.push(event);
    }
    server.abort();
    let requests = requests.lock().await.clone();
    (events, requests)
}

#[tokio::test]
async fn unauthorized_before_streaming_rotates_and_retries_once() {
    use llm_router::types::events::AssistantMessageEvent;
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
    let (events, requests) = stream_requests(manager, vec![UNAUTHORIZED, STREAM_OK]).await;
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("Bearer secret-account"));
    assert!(requests[1].contains("Bearer rotated-access"));
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
    assert!(matches!(
        events.last(),
        Some(AssistantMessageEvent::Done { .. })
    ));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, AssistantMessageEvent::Start { .. }))
            .count(),
        1
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e, AssistantMessageEvent::Error { .. })));
}

#[tokio::test]
async fn repeated_unauthorized_is_terminal_after_one_retry() {
    use llm_router::types::events::AssistantMessageEvent;
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
    let (events, requests) = stream_requests(manager, vec![UNAUTHORIZED, UNAUTHORIZED]).await;
    assert_eq!(requests.len(), 2);
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events.first(),
        Some(AssistantMessageEvent::Error { .. })
    ));
}

#[tokio::test]
async fn streaming_auth_error_is_not_replayed_after_content() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("account", 3600)));
    let reply="HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"invalid_api_key\",\"message\":\"expired\"}}}\n\n";
    let (_, requests) = stream_requests(manager, vec![reply]).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unauthorized_never_replays_a_request_under_a_replacement_account() {
    let (manager, store, oauth, _) = setup();
    *store.value.lock().await = Some(SessionRecord::connected(credential("replacement", 3600)));
    let (events, requests) = stream_requests(manager, vec![UNAUTHORIZED]).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(oauth.refreshes.load(Ordering::SeqCst), 0);
    let output = serde_json::to_string(&events).unwrap();
    assert!(output.contains("Codex account changed"));
}
