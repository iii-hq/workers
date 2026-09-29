//! Real bus/storage/router integration; only OAuth and legacy lookup are fakes.
//! Never render credential-bearing RPC values or captured requests on failure.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, ensure, Context};
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{register_worker, IIIClient, RegisterFunction};
use provider_openai_codex::credential_store::{PrivateStateStore, SessionStore, AUTH_SCOPE};
use provider_openai_codex::oauth::{Credential, DeviceCode, OAuthError, PollOutcome};
use provider_openai_codex::session::{
    AuthManager, LegacySource, OAuthTransport, ResolvedCredential, SessionError,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Semaphore};

use crate::case::{enabled_cases, ProviderCase};
use crate::contract::{chat, configure, wait_for_provider, wait_for_registration_token};
use crate::protocol::happy_sse;
use crate::runtime::{
    call, provider_init_options, test_init_options, wait_for_codex_state, Engine,
};
use crate::stub::{StubResponse, StubUpstream};

const ACCESS: &str = "codex-login-synthetic-access";
const REFRESH: &str = "codex-login-synthetic-refresh";
const ID_TOKEN: &str = "codex-login-synthetic-id-token";
const ACCOUNT: &str = "codex-login-synthetic-account";
const DEVICE_ID: &str = "codex-login-synthetic-device";
const PUBLIC_SCOPE: &str = "codex-login-public-probe";

type Reply<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Default)]
struct EmptyLegacy(AtomicUsize);

impl LegacySource for EmptyLegacy {
    fn resolve(&self) -> Reply<'_, Result<Option<ResolvedCredential>, SessionError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(None) })
    }
}

struct FakeOAuth {
    authorized: Semaphore,
    polling: Semaphore,
    /// One permit per in-flight poll that returned or was dropped.
    settled: Semaphore,
}

impl Default for FakeOAuth {
    fn default() -> Self {
        Self {
            authorized: Semaphore::new(0),
            polling: Semaphore::new(0),
            settled: Semaphore::new(0),
        }
    }
}

struct Settled<'a>(&'a Semaphore);

impl Drop for Settled<'_> {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}

impl OAuthTransport for FakeOAuth {
    fn start(&self) -> Reply<'_, Result<DeviceCode, OAuthError>> {
        Box::pin(async {
            Ok(DeviceCode {
                device_auth_id: DEVICE_ID.into(),
                user_code: "TEST-CODE".into(),
                verification_uri: "https://login.invalid/device".into(),
                interval: 1,
                expires_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 60,
            })
        })
    }

    fn poll<'a>(&'a self, device: &'a DeviceCode) -> Reply<'a, Result<PollOutcome, OAuthError>> {
        Box::pin(async move {
            assert!(
                device.device_auth_id == DEVICE_ID,
                "unexpected device ceremony"
            );
            let _settled = Settled(&self.settled);
            self.polling.add_permits(1);
            self.authorized
                .acquire()
                .await
                .expect("OAuth fixture open")
                .forget();
            Ok(PollOutcome::Authorized(Credential {
                access_token: ACCESS.into(),
                refresh_token: Some(REFRESH.into()),
                id_token: Some(ID_TOKEN.into()),
                account_id: ACCOUNT.into(),
                expires_at: 4_102_444_800,
            }))
        })
    }

    fn refresh<'a>(&'a self, _: &'a Credential) -> Reply<'a, Result<Credential, OAuthError>> {
        Box::pin(async {
            Err(OAuthError {
                code: "unexpected_refresh".into(),
                message: "Fresh synthetic credentials must not be refreshed".into(),
                permanent: true,
                retry_after_secs: None,
            })
        })
    }
}

#[derive(Default)]
struct Clients(Vec<IIIClient>);

impl Clients {
    fn connect(&mut self, url: &str, provider: bool) -> IIIClient {
        let options = if provider {
            provider_init_options("openai-codex")
        } else {
            test_init_options()
        };
        let client = register_worker(url, options);
        self.0.push(client.clone());
        client
    }
}

impl Drop for Clients {
    fn drop(&mut self) {
        for client in &self.0 {
            client.shutdown();
        }
    }
}

struct Fixture {
    // Shut down SDK connections before dropping the engine process.
    _clients: Clients,
    engine: Engine,
    operator: IIIClient,
    provider: IIIClient,
    store: Arc<PrivateStateStore>,
    oauth: Arc<FakeOAuth>,
    legacy: Arc<EmptyLegacy>,
    stub: StubUpstream,
    case: ProviderCase,
}

impl Fixture {
    async fn start() -> anyhow::Result<Self> {
        let case = enabled_cases()
            .into_iter()
            .find(|case| case.id == "openai-codex")
            .context("enable provider-openai-codex")?;
        let engine = Engine::start().await?;
        let mut clients = Clients::default();
        let operator = clients.connect(&engine.url, false);
        let router = clients.connect(&engine.url, false);
        llm_router::register::register_router(router.clone()).await?;
        let stub = StubUpstream::start(case).await?;
        configure(&router, case, &stub.endpoint(case.generation_path)).await?;
        let provider = clients.connect(&engine.url, true);
        let store = Arc::new(PrivateStateStore::new(provider.clone()));
        wait_for_codex_state(&provider).await?;
        ensure!(
            store.load().await?.is_none(),
            "fresh engine already has an auth record"
        );
        let oauth = Arc::new(FakeOAuth::default());
        let legacy = Arc::new(EmptyLegacy::default());
        let auth = AuthManager::with_dependencies(store.clone(), oauth.clone(), legacy.clone());
        provider_openai_codex::register::register_provider_with_auth(provider.clone(), auth)
            .await?;
        wait_for_provider(&router, case.id).await?;
        wait_for_registration_token(&provider, case.id).await?;
        Ok(Self {
            _clients: clients,
            engine,
            operator,
            provider,
            store,
            oauth,
            legacy,
            stub,
            case,
        })
    }

    async fn rpc(&self, id: &str, arguments: Value) -> anyhow::Result<Value> {
        let value = call(&self.operator, id, arguments)
            .await
            .map_err(|_| anyhow!("{id} RPC failed"))?;
        ensure!(
            !contains_secret(&value),
            "{id} returned credential material"
        );
        Ok(value)
    }

    async fn start_login(&self) -> anyhow::Result<String> {
        let start = self
            .rpc("provider::openai-codex::login::start", json!({}))
            .await?;
        ensure!(
            start["user_code"] == "TEST-CODE",
            "login returned the wrong user code"
        );
        ensure!(
            start["verification_uri"] == "https://login.invalid/device",
            "wrong verification URI"
        );
        let id = start["login_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .context("missing login id")?;
        let pending = self
            .rpc(
                "provider::openai-codex::login::poll",
                json!({ "login_id": id }),
            )
            .await?;
        ensure!(
            pending["status"] == "pending",
            "login did not begin pending"
        );
        Ok(id.into())
    }

    async fn finish_login(&self, id: &str) -> anyhow::Result<()> {
        self.oauth.authorized.add_permits(1);
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let poll = self
                    .rpc(
                        "provider::openai-codex::login::poll",
                        json!({ "login_id": id }),
                    )
                    .await?;
                if poll["status"] == "ok" {
                    return Ok::<_, anyhow::Error>(());
                }
                ensure!(
                    poll["status"] == "pending",
                    "login failed before completion"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .context("login completion timed out")??;
        Ok(())
    }

    async fn assert_public_state_is_private(&self) -> anyhow::Result<()> {
        for id in ["state::get", "state::list", "state::list_keys"] {
            match call(
                &self.operator,
                id,
                json!({ "scope": AUTH_SCOPE, "key": "session" }),
            )
            .await
            {
                Ok(_) => anyhow::bail!("{id} exposed the private auth scope"),
                Err(error) => {
                    // Timeouts, missing endpoints, and unrelated errors are
                    // not denial; only the reserved-scope rejection counts.
                    ensure!(
                        error.to_string().contains("RESERVED_SCOPE"),
                        "{id} failed for a reason other than private-scope denial"
                    );
                }
            }
        }
        let groups = self.rpc("state::list_groups", json!({})).await?;
        ensure!(
            !groups["groups"]
                .as_array()
                .context("missing state groups")?
                .contains(&json!(AUTH_SCOPE)),
            "private scope appeared in public groups"
        );
        Ok(())
    }
}

fn contains_secret(value: &Value) -> bool {
    let rendered = value.to_string();
    [
        ACCESS,
        REFRESH,
        ID_TOKEN,
        DEVICE_ID,
        "access_token",
        "refresh_token",
        "id_token",
    ]
    .iter()
    .any(|secret| rendered.contains(secret))
}

struct ObservedEvent {
    scope: String,
    key: String,
    secret: bool,
}

async fn observe_state(
    client: &IIIClient,
) -> anyhow::Result<(
    iii_sdk::trigger::Trigger,
    mpsc::UnboundedReceiver<ObservedEvent>,
)> {
    let (sender, receiver) = mpsc::unbounded_channel();
    client.register_function(
        "contract::codex-state-event",
        RegisterFunction::new_async(move |event: Value| {
            let _ = sender.send(ObservedEvent {
                scope: event["scope"].as_str().unwrap_or_default().into(),
                key: event["key"].as_str().unwrap_or_default().into(),
                secret: contains_secret(&event),
            });
            async { Ok::<_, Error>(json!({ "ok": true })) }
        }),
    );
    let trigger = client.register_trigger(RegisterTriggerInput::new(
        "state",
        "contract::codex-state-event",
        json!({}),
    ))?;
    Ok((trigger, receiver))
}

async fn public_event_barrier(
    client: &IIIClient,
    events: &mut mpsc::UnboundedReceiver<ObservedEvent>,
    key: &str,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            call(
                client,
                "state::set",
                json!({ "scope": PUBLIC_SCOPE, "key": key, "value": "public" }),
            )
            .await?;
            // Repeated marker writes also handle asynchronous trigger registration.
            while let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(100), events.recv()).await
            {
                ensure!(
                    event.scope != AUTH_SCOPE && !event.secret,
                    "private auth state emitted an event"
                );
                if event.scope == PUBLIC_SCOPE && event.key == key {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        }
    })
    .await
    .context("public state event listener did not become ready")??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires III_ENGINE_BIN and III_STATE_BIN; hermetic Codex login integration"]
async fn codex_login_persists_discovers_streams_and_logs_out() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let (_trigger, mut events) = observe_state(&fixture.operator).await?;
    public_event_barrier(&fixture.operator, &mut events, "before-login").await?;
    let status = fixture
        .rpc("provider::openai-codex::auth::status", json!({}))
        .await?;
    ensure!(
        status["status"] == "signed_out",
        "fresh provider was not signed out"
    );
    ensure!(
        fixture.stub.requests().is_empty(),
        "signed-out provider contacted upstream"
    );
    let id = fixture.start_login().await?;
    fixture.finish_login(&id).await?;

    let stored = fixture
        .store
        .load()
        .await?
        .context("login did not persist a session")?;
    let credential = stored
        .credential
        .context("persisted session has no credential")?;
    ensure!(
        credential.access_token == ACCESS
            && credential.refresh_token.as_deref() == Some(REFRESH)
            && credential.id_token.as_deref() == Some(ID_TOKEN)
            && credential.account_id == ACCOUNT,
        "persisted credential differs from the synthetic OAuth account"
    );
    let status = fixture
        .rpc("provider::openai-codex::auth::status", json!({}))
        .await?;
    ensure!(
        status["status"] == "authenticated"
            && status["source"] == "managed"
            && status["account_id"] == ACCOUNT,
        "operator status did not report the managed account"
    );
    fixture.assert_public_state_is_private().await?;

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let catalog = fixture
                .rpc(
                    "router::models::list",
                    json!({ "provider": "openai-codex" }),
                )
                .await?;
            if catalog["models"]
                .as_array()
                .is_some_and(|models| models.iter().any(|m| m["id"] == fixture.case.model))
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("login did not populate the router catalog")??;
    let requests = fixture.stub.requests();
    let models = requests
        .iter()
        .find(|r| r.method == "GET" && r.path == "/backend-api/codex/models")
        .context("catalog did not fetch the Codex models endpoint")?;
    ensure!(
        models.header("authorization") == Some(format!("Bearer {ACCESS}").as_str())
            && models.header("chatgpt-account-id") == Some(ACCOUNT),
        "catalog used the wrong account"
    );

    // A new manager reads the durable record through a new store instance.
    let reloaded = AuthManager::with_dependencies(
        Arc::new(PrivateStateStore::new(fixture.provider.clone())),
        fixture.oauth.clone(),
        fixture.legacy.clone(),
    );
    let resolved = reloaded
        .resolve(None)
        .await?
        .context("new manager lost the persisted session")?;
    ensure!(
        resolved.value["access_token"] == ACCESS,
        "new manager resolved the wrong credential"
    );
    fixture
        .stub
        .respond([StubResponse::sse(happy_sse(fixture.case.family))]);
    let streamed = chat(
        &fixture.engine.url,
        fixture.case,
        fixture.case.model,
        "codex-login-stream",
    )
    .await?;
    ensure!(
        streamed.response["ok"] == true
            && streamed
                .frames
                .last()
                .is_some_and(|frame| frame["type"] == "done"),
        "authenticated router stream did not complete"
    );
    ensure!(
        !contains_secret(&streamed.response) && !streamed.frames.iter().any(contains_secret),
        "stream leaked credentials"
    );
    let requests = fixture.stub.post_requests();
    ensure!(requests.len() == 1, "unexpected stream request count");
    ensure!(
        requests[0].header("authorization") == Some(format!("Bearer {ACCESS}").as_str())
            && requests[0].header("chatgpt-account-id") == Some(ACCOUNT),
        "stream used the wrong account"
    );

    let legacy_calls = fixture.legacy.0.load(Ordering::SeqCst);
    ensure!(
        legacy_calls > 0,
        "fresh signed-out session never checked the empty legacy source"
    );
    fixture
        .rpc("provider::openai-codex::auth::logout", json!({}))
        .await?;
    let status = fixture
        .rpc("provider::openai-codex::auth::status", json!({}))
        .await?;
    ensure!(
        status["status"] == "signed_out" && status["account_id"].is_null(),
        "logout left an account active"
    );
    ensure!(
        fixture
            .store
            .load()
            .await?
            .is_some_and(|record| record.credential.is_none()),
        "logout did not persist its tombstone"
    );
    ensure!(
        reloaded.resolve(None).await?.is_none(),
        "another manager ignored logout"
    );
    fixture.stub.clear_requests();
    let chat = chat(
        &fixture.engine.url,
        fixture.case,
        fixture.case.model,
        "codex-after-logout",
    )
    .await?;
    ensure!(
        chat.response["ok"] == false,
        "stream succeeded after logout"
    );
    ensure!(
        fixture.stub.post_requests().is_empty(),
        "logout still sent an upstream request"
    );
    ensure!(
        fixture.legacy.0.load(Ordering::SeqCst) == legacy_calls,
        "logout fell back to legacy credentials"
    );
    fixture.assert_public_state_is_private().await?;
    public_event_barrier(&fixture.operator, &mut events, "after-logout").await?;
    // Drain late fan-out too; never retain credential-bearing event payloads.
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(250), events.recv()).await
    {
        ensure!(
            event.scope != AUTH_SCOPE && !event.secret,
            "private auth state emitted an event"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires III_ENGINE_BIN and III_STATE_BIN; hermetic Codex login integration"]
async fn codex_login_cancel_cannot_persist_a_late_authorization() -> anyhow::Result<()> {
    let fixture = Fixture::start().await?;
    let id = fixture.start_login().await?;
    tokio::time::timeout(Duration::from_secs(5), fixture.oauth.polling.acquire())
        .await
        .context("OAuth polling did not start")?
        .context("OAuth fixture closed")?
        .forget();
    fixture
        .rpc(
            "provider::openai-codex::login::cancel",
            json!({ "login_id": id }),
        )
        .await?;
    fixture.oauth.authorized.add_permits(1);
    // The in-flight poll either observes cancellation (dropped) or delivers the
    // late authorization (returned); wait for whichever happened.
    tokio::time::timeout(Duration::from_secs(5), fixture.oauth.settled.acquire())
        .await
        .context("in-flight OAuth poll never settled")?
        .context("OAuth fixture closed")?
        .forget();
    let poll = fixture
        .rpc(
            "provider::openai-codex::login::poll",
            json!({ "login_id": id }),
        )
        .await?;
    ensure!(
        poll["status"] == "canceled",
        "canceled login accepted late authorization"
    );
    ensure!(
        fixture.store.load().await?.is_none(),
        "canceled login persisted credentials"
    );
    ensure!(
        fixture.stub.requests().is_empty(),
        "canceled login contacted the backend"
    );
    Ok(())
}
