//! `tmux::accounts::credential` (chat-projects spec §4.2.1, D20, D21,
//! D141): the access token of one account, for `provider-claude-code`,
//! renewed first when it expires within five minutes.
//!
//! The token is read from where the account's sign-in lives now: the slot
//! (`~/.claude`) for the global account, its folder for any other, through
//! the platform's [`SecretStore`](super::credentials::SecretStore) (the
//! Keychain on macOS). A read that a swap could have raced (a swap's
//! journal, or the registry changing under it) is made again.
//!
//! A renewal is the only place the worker renews a token. It runs in a task
//! of its own (Anthropic rotates refresh tokens: an answer must be kept
//! once it came), one per account at a time, and:
//!
//! 1. takes the swap gate, settles a swap left half-way, holds
//!    `swap.lock`, and only then decides which store holds the account, so
//!    no swap moves the sign-in under it;
//! 2. takes the store's three locks as a swap does (Claude Code's
//!    `.oauth_refresh.lock`, the legacy `realpath(dir).lock`,
//!    `.storage-write.lock`), touched while it holds them;
//! 3. reads the sign-in again: a Claude Code may have renewed it;
//! 4. renews it with its refresh token, as Claude Code does ([`oauth`]);
//! 5. writes it back to the same store, atomically, over the very sign-in
//!    it started from, reads it back, then lets go of the locks.
//!
//! Anything held for more than ten seconds (the gate, `swap.lock`, a lock
//! Claude Code holds) answers `busy`. Nothing here logs a token.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::credentials::{self, Credentials, OAUTH_KEY, Raw, Secret};
use super::lockdir::LockSet;
use super::oauth::{self, RenewError, Renewed};
use super::slot::{Place, SwapError, SwapHold};
use super::{Accounts, GLOBAL};
use crate::model::now_ms;

/// How long a renewal waits for the gate, `swap.lock` and Claude Code's
/// locks together before it answers `busy`.
pub const WAIT: Duration = Duration::from_secs(10);
/// How often the locks are touched while the token endpoint answers.
const TOUCH_EVERY: Duration = Duration::from_secs(2);
/// How many times a renewed sign-in is written before the renewal fails.
const WRITE_TRIES: u32 = 3;
/// A token that still works this long is answered when a renewal fails
/// for a reason that may pass.
const STILL_USABLE_MS: u64 = 10_000;

/// Why no token: one of `unknown_account`, `not_signed_in`,
/// `token_expired`, `busy` or `failed`, with one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialError {
    pub code: &'static str,
    pub message: String,
}

impl CredentialError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// One JSON object `{code, message}`, as the worker's other errors.
    pub fn to_wire(&self) -> String {
        serde_json::json!({"code": self.code, "message": self.message}).to_string()
    }

    fn unknown(id: &str) -> Self {
        if id == GLOBAL {
            return Self::new(
                "unknown_account",
                "Name the account itself: tmux::accounts::list's `global` says which one is global now.",
            );
        }
        Self::new("unknown_account", super::manage::no_account(id))
    }

    fn not_signed_in() -> Self {
        Self::new(
            "not_signed_in",
            "The account is not signed in; sign it in (tmux::accounts::login_start).",
        )
    }

    fn expired(why: &str) -> Self {
        Self::new(
            "token_expired",
            format!("{why} Sign the account in again (tmux::accounts::login_start)."),
        )
    }

    fn busy(message: &str) -> Self {
        Self::new("busy", message)
    }

    fn failed(message: impl Into<String>) -> Self {
        Self::new("failed", message)
    }
}

const BUSY: &str =
    "Claude Code or a switch of the global account holds the sign-in; try again in a moment.";

/// An account's access token, for the one caller that may have it.
#[derive(Debug, Clone)]
pub struct Credential {
    pub account: String,
    pub access_token: Secret,
    pub expires_at_ms: u64,
    pub email: Option<String>,
    pub plan: Option<String>,
}

/// One renewal per account at a time, and how long they wait.
pub struct Renewals {
    per_account: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    wait_ms: AtomicU64,
    /// Renewals that asked the token endpoint, for tests.
    asked: AtomicU64,
}

impl Default for Renewals {
    fn default() -> Self {
        Self {
            per_account: Mutex::new(HashMap::new()),
            wait_ms: AtomicU64::new(WAIT.as_millis() as u64),
            asked: AtomicU64::new(0),
        }
    }
}

impl Renewals {
    fn of(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.per_account
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(id.to_string())
            .or_default()
            .clone()
    }

    fn wait(&self) -> Duration {
        Duration::from_millis(self.wait_ms.load(Ordering::Relaxed))
    }

    /// How long a renewal waits before `busy` (tests shorten it).
    pub fn set_wait(&self, wait: Duration) {
        self.wait_ms
            .store(wait.as_millis() as u64, Ordering::Relaxed);
    }

    /// How many renewals asked the token endpoint.
    pub fn asked(&self) -> u64 {
        self.asked.load(Ordering::Relaxed)
    }
}

/// What a sign-in, read at `now`, allows.
enum Class {
    /// Signed out.
    NoSignIn,
    /// The token works for more than five minutes.
    Fresh { token: Secret, expires_at_ms: u64 },
    /// To renew: the refresh token, and the token while it still works.
    Due {
        refresh: Secret,
        usable: Option<(Secret, u64)>,
    },
    /// No refresh token and no token that works.
    Dead,
}

fn classify(credentials: &Credentials, now: u64) -> Class {
    if !credentials.signed_in() {
        return Class::NoSignIn;
    }
    let usable = credentials.token.clone().and_then(|token| {
        // A sign-in with no expiry (one made from a token alone) is asked
        // for again in five minutes.
        let expires = credentials
            .expires_at_ms
            .unwrap_or(now + oauth::RENEW_BEFORE_MS);
        (expires > now + STILL_USABLE_MS).then_some((token, expires))
    });
    match (&usable, &credentials.refresh) {
        (Some((token, expires)), _) if !oauth::due(credentials.expires_at_ms, now) => {
            Class::Fresh {
                token: token.clone(),
                expires_at_ms: *expires,
            }
        }
        (_, Some(refresh)) => Class::Due {
            refresh: refresh.clone(),
            usable,
        },
        // Expires within five minutes and nothing renews it: it works
        // until then.
        (Some((token, expires)), None) => Class::Fresh {
            token: token.clone(),
            expires_at_ms: *expires,
        },
        (None, None) => Class::Dead,
    }
}

/// The store and locks a renewal holds.
struct Held {
    _hold: Option<SwapHold>,
    locks: LockSet,
    place: Place,
    raw: Option<Raw>,
}

/// What a renewal did.
enum Done {
    /// It answered without asking the endpoint.
    Kept(Credential),
    /// It renewed and saved the sign-in.
    Renewed(Credential),
}

impl Accounts {
    /// Account `id`'s access token, renewed first when it expires within
    /// five minutes (see the module's description).
    pub async fn credential(self: &Arc<Self>, id: &str) -> Result<Credential, CredentialError> {
        let id = id.trim().to_string();
        if id == GLOBAL || !super::registry::valid_id(&id) {
            return Err(CredentialError::unknown(&id));
        }
        let deadline = Instant::now() + self.renewals.wait();
        let quick = {
            let this = self.clone();
            let id = id.clone();
            tokio::task::spawn_blocking(move || this.read_unlocked(&id))
                .await
                .map_err(|error| {
                    CredentialError::failed(format!("The worker failed ({error})."))
                })??
        };
        match quick {
            Some(Class::Fresh {
                token,
                expires_at_ms,
            }) => return Ok(self.answer(&id, token, expires_at_ms)),
            Some(Class::NoSignIn) => return Err(CredentialError::not_signed_in()),
            Some(Class::Dead) => {
                return Err(CredentialError::expired(
                    "Its sign-in expired and holds nothing to renew it with.",
                ));
            }
            Some(Class::Due { .. }) | None => {}
        }
        // To its end even when the caller goes away: a token renewed must
        // be kept, since the one it replaces no longer works.
        let this = self.clone();
        let renewed = tokio::spawn(async move { this.renew(&id, deadline).await })
            .await
            .map_err(|error| CredentialError::failed(format!("The worker failed ({error}).")))?;
        match renewed? {
            Done::Kept(credential) | Done::Renewed(credential) => Ok(credential),
        }
    }

    /// The sign-in of `id` read where it lives now, without the gate.
    /// `None` when a swap could have moved it while it was read (a journal,
    /// or another registry after the read): then only the gate tells.
    /// Blocking.
    fn read_unlocked(&self, id: &str) -> Result<Option<Class>, CredentialError> {
        let stores = self
            .stores()
            .ok_or_else(|| CredentialError::failed("This computer's user has no home folder."))?;
        for _ in 0..3 {
            let (registry, generation) = self.registry_at();
            if !registry.has(id) {
                return Err(CredentialError::unknown(id));
            }
            if stores.read_journal().is_some() {
                return Ok(None);
            }
            let place = Self::place_of(&registry, id);
            let raw = stores.read(&place).map_err(CredentialError::failed)?;
            if self.registry_at().1 == generation && stores.read_journal().is_none() {
                let credentials = raw
                    .as_ref()
                    .map(credentials::credentials_of)
                    .unwrap_or_default();
                return Ok(Some(classify(&credentials, now_ms())));
            }
        }
        Ok(None)
    }

    /// The answer for `id` with `token`: its email and plan from where its
    /// sign-in lives now.
    fn answer(&self, id: &str, token: Secret, expires_at_ms: u64) -> Credential {
        let (email, plan) = self.profile_of(id);
        Credential {
            account: id.to_string(),
            access_token: token,
            expires_at_ms,
            email,
            plan,
        }
    }

    fn profile_of(&self, id: &str) -> (Option<String>, Option<String>) {
        let Some(stores) = self.stores() else {
            return (None, None);
        };
        let place = Self::place_of(&self.registry(), id);
        let profile = std::fs::read_to_string(stores.json(&place))
            .ok()
            .and_then(|text| credentials::parse_profile(&text));
        let signed = stores
            .read(&place)
            .ok()
            .flatten()
            .as_ref()
            .map(credentials::credentials_of);
        (
            profile.as_ref().and_then(|p| p.email.clone()),
            credentials::plan(signed.as_ref(), profile.as_ref()),
        )
    }

    /// The renewal itself: one per account, under the gate, `swap.lock`
    /// and the store's locks (see the module's description).
    async fn renew(self: &Arc<Self>, id: &str, deadline: Instant) -> Result<Done, CredentialError> {
        let one = self.renewals.of(id);
        let _one = tokio::time::timeout_at(deadline.into(), one.lock_owned())
            .await
            .map_err(|_| CredentialError::busy(BUSY))?;
        let gate = tokio::time::timeout_at(deadline.into(), self.gate())
            .await
            .map_err(|_| CredentialError::busy(BUSY))?;
        // A swap left half-way is settled first; one that can't be yet
        // keeps its two accounts' sign-ins where nobody can tell them.
        if let Some(journal) = self.unsettled_swap().await
            && (journal.from == id || journal.to == id)
        {
            return Err(CredentialError::busy(super::switch::SWAP_UNSETTLED));
        }
        let held = {
            let this = self.clone();
            let id = id.to_string();
            tokio::task::spawn_blocking(move || this.hold_for_renewal(&id, deadline))
                .await
                .map_err(|error| {
                    CredentialError::failed(format!("The worker failed ({error})."))
                })??
        };
        let outcome = self.renew_held(id, held).await;
        drop(gate);
        if matches!(outcome, Ok(Done::Renewed(_))) {
            self.reread_credentials(id);
        }
        outcome
    }

    /// Steps 1 and 2 under the gate: `swap.lock`, the account's store as
    /// the registry says now, its three locks, the sign-in read again.
    /// Blocking.
    fn hold_for_renewal(&self, id: &str, deadline: Instant) -> Result<Held, CredentialError> {
        let stores = self
            .stores()
            .ok_or_else(|| CredentialError::failed("This computer's user has no home folder."))?;
        // Without `accounts.json` nothing swaps, and nothing is made in the
        // accounts folder.
        let hold = if self.managed() {
            loop {
                match stores.hold_swap() {
                    Ok(hold) => break Some(hold),
                    Err(SwapError::Busy) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(SwapError::Busy) => return Err(CredentialError::busy(BUSY)),
                    Err(error) => return Err(CredentialError::failed(error.sentence())),
                }
            }
        } else {
            None
        };
        if let Some(journal) = stores.read_journal()
            && (journal.from == id || journal.to == id)
        {
            return Err(CredentialError::busy(super::switch::SWAP_UNSETTLED));
        }
        let registry = self.registry();
        if !registry.has(id) {
            return Err(CredentialError::unknown(id));
        }
        let place = Self::place_of(&registry, id);
        let locks = stores
            .lock_place(&place, deadline)
            .map_err(|error| match error {
                SwapError::Busy => CredentialError::busy(BUSY),
                other => CredentialError::failed(other.sentence()),
            })?;
        let raw = stores.read(&place).map_err(CredentialError::failed)?;
        Ok(Held {
            _hold: hold,
            locks,
            place,
            raw,
        })
    }

    /// Steps 3 to 5, the locks held: decide again from the sign-in read
    /// under them, renew, write back. The locks go when `held` does.
    async fn renew_held(
        self: &Arc<Self>,
        id: &str,
        mut held: Held,
    ) -> Result<Done, CredentialError> {
        let now = now_ms();
        let credentials = held
            .raw
            .as_ref()
            .map(credentials::credentials_of)
            .unwrap_or_default();
        let (refresh, usable) = match classify(&credentials, now) {
            Class::NoSignIn => return Err(CredentialError::not_signed_in()),
            Class::Dead => {
                return Err(CredentialError::expired(
                    "Its sign-in expired and holds nothing to renew it with.",
                ));
            }
            // A Claude Code renewed it meanwhile.
            Class::Fresh {
                token,
                expires_at_ms,
            } => return Ok(Done::Kept(self.answer(id, token, expires_at_ms))),
            Class::Due { refresh, usable } => (refresh, usable),
        };
        let keep_usable = |why: String| match &usable {
            Some((token, expires)) => {
                tracing::warn!(account = %id, %why, "the sign-in was not renewed; its token still works");
                Ok(Done::Kept(self.answer(id, token.clone(), *expires)))
            }
            None => Err(CredentialError::failed(why)),
        };
        let Some(client) = self.oauth.clone() else {
            return keep_usable("This worker can't renew sign-ins.".into());
        };
        let entry = held
            .raw
            .as_ref()
            .and_then(|raw| raw.get(OAUTH_KEY))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let (scopes, fallback) = oauth::scopes_to_ask(&entry);
        let client_id = oauth::client_id(&entry);
        self.renewals.asked.fetch_add(1, Ordering::Relaxed);
        let mut outcome = touching(
            &mut held.locks,
            client.renew(&refresh, &client_id, &scopes, now),
        )
        .await;
        if outcome.as_ref().err() == Some(&RenewError::InvalidScope)
            && let Some(fallback) = fallback
        {
            outcome = touching(
                &mut held.locks,
                client.renew(&refresh, &client_id, &fallback, now_ms()),
            )
            .await;
        }
        let renewed = match outcome {
            Ok(renewed) => renewed,
            Err(error) if error.refused() || error == RenewError::InvalidScope => {
                tracing::warn!(account = %id, "Anthropic refused to renew the sign-in");
                return match &usable {
                    Some((token, expires)) => {
                        Ok(Done::Kept(self.answer(id, token.clone(), *expires)))
                    }
                    None => Err(CredentialError::expired(
                        "Anthropic refused to renew its sign-in.",
                    )),
                };
            }
            Err(error) => return keep_usable(error.sentence()),
        };
        // Step 5, off the async threads (a Keychain answers slowly).
        let this = self.clone();
        let written = tokio::task::spawn_blocking(move || {
            let written = this.write_back(&mut held, &renewed, &refresh);
            // The locks and `swap.lock` go as soon as the store is written.
            drop(held);
            written.map(|saved| (saved, renewed))
        })
        .await
        .map_err(|error| CredentialError::failed(format!("The worker failed ({error}).")))?;
        match written {
            Ok((Saved::Ours, renewed)) => {
                tracing::info!(account = %id, "sign-in renewed");
                Ok(Done::Renewed(self.answer(
                    id,
                    renewed.access_token,
                    renewed.expires_at_ms,
                )))
            }
            Ok((Saved::Theirs(credentials), _)) => match classify(&credentials, now_ms()) {
                Class::Fresh {
                    token,
                    expires_at_ms,
                } => Ok(Done::Kept(self.answer(id, token, expires_at_ms))),
                Class::NoSignIn => Err(CredentialError::not_signed_in()),
                _ => Err(CredentialError::busy(BUSY)),
            },
            Err(error) => {
                tracing::error!(account = %id, %error, "a renewed sign-in could not be saved");
                Err(CredentialError::failed(format!(
                    "The renewed sign-in could not be saved ({error})."
                )))
            }
        }
    }

    /// Write `renewed` over the sign-in the renewal started from
    /// (`posted`), in the store it was read from, the rest of the store as
    /// it is now, and read it back; tried again on a failed write. Another
    /// sign-in found there (something that ignored the locks), or none, is
    /// left as it is and answered instead, as Claude Code does. Blocking.
    fn write_back(
        &self,
        held: &mut Held,
        renewed: &Renewed,
        posted: &Secret,
    ) -> Result<Saved, String> {
        let stores = self
            .stores()
            .ok_or_else(|| "This computer's user has no home folder.".to_string())?;
        let mut last = String::new();
        for attempt in 0..WRITE_TRIES {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(200 * u64::from(attempt)));
            }
            held.locks.touch();
            let mut raw = match stores.read(&held.place) {
                Ok(Some(raw)) => raw,
                // Signed out meanwhile: the renewal is not brought back.
                Ok(None) => return Ok(Saved::Theirs(Credentials::default())),
                Err(error) => {
                    last = error;
                    continue;
                }
            };
            let entry = raw
                .get(OAUTH_KEY)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let stored = entry
                .get("refreshToken")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // A write of ours that was not read back last time.
            if stored == renewed.refresh_token.expose() && kept(&raw, renewed) {
                return Ok(Saved::Ours);
            }
            if stored != posted.expose() {
                return Ok(Saved::Theirs(credentials::credentials_of(&raw)));
            }
            raw.insert(
                OAUTH_KEY.to_string(),
                Value::Object(oauth::renewed_oauth(&entry, renewed)),
            );
            held.locks.touch();
            if let Err(error) = stores.write(&held.place, &raw) {
                last = error;
                continue;
            }
            // Read back before the locks go: a renewal that did not land
            // would leave the account with a refresh token Anthropic has
            // already replaced.
            match stores.read(&held.place) {
                Ok(Some(back)) if kept(&back, renewed) => return Ok(Saved::Ours),
                Ok(_) => last = "the store did not keep it".into(),
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}

/// Whether the sign-in `raw` holds the renewal `renewed`.
fn kept(raw: &Raw, renewed: &Renewed) -> bool {
    let back = credentials::credentials_of(raw);
    back.token.as_ref() == Some(&renewed.access_token)
        && back.refresh.as_ref() == Some(&renewed.refresh_token)
}

/// What [`Accounts::write_back`] found.
enum Saved {
    /// The renewal is in the store.
    Ours,
    /// Another sign-in was there; it stays.
    Theirs(Credentials),
}

/// Run `work`, touching `locks` every couple of seconds meanwhile, so none
/// grows stale while the token endpoint answers.
async fn touching<F: std::future::Future>(locks: &mut LockSet, work: F) -> F::Output {
    tokio::pin!(work);
    let mut tick = tokio::time::interval(TOUCH_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            out = &mut work => return out,
            _ = tick.tick() => locks.touch(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::credentials::KeychainStore;
    use crate::accounts::registry;
    use crate::accounts::slot::{self, Crash};
    use crate::accounts::test_support::Home;
    use serde_json::json;
    use std::path::Path;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const HOUR: u64 = 3_600_000;

    type Reply = Arc<dyn Fn(&Value) -> (u16, Value) + Send + Sync>;
    type During = Arc<dyn Fn() + Send + Sync>;

    /// Anthropic's token endpoint, faked on loopback: it notes each
    /// request, waits `delay`, runs `during` (what happens while a renewal
    /// waits for its answer), then answers with `reply`.
    struct TokenServer {
        url: String,
        asked: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl TokenServer {
        async fn start(delay: Duration, reply: Reply) -> Self {
            Self::with(delay, reply, Arc::new(|| {})).await
        }

        async fn with(delay: Duration, reply: Reply, during: During) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let asked = Arc::new(Mutex::new(Vec::new()));
            let noted = asked.clone();
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let (noted, reply, during) = (noted.clone(), reply.clone(), during.clone());
                    tokio::spawn(async move {
                        let Some((path, body)) = read_request(&mut socket).await else {
                            return;
                        };
                        noted.lock().unwrap().push((path, body.clone()));
                        tokio::time::sleep(delay).await;
                        tokio::task::spawn_blocking(move || during()).await.unwrap();
                        let (status, answer) = reply(&body);
                        let text = answer.to_string();
                        let response = format!(
                            "HTTP/1.1 {status} Answer\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                            text.len()
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                    });
                }
            });
            Self { url, asked }
        }

        fn asked(&self) -> Vec<(String, Value)> {
            self.asked.lock().unwrap().clone()
        }
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<(String, Value)> {
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        let head_end = loop {
            if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            let n = socket.read(&mut buf).await.ok()?;
            if n == 0 {
                return None;
            }
            request.extend_from_slice(&buf[..n]);
        };
        let head = String::from_utf8_lossy(&request[..head_end]).to_string();
        let path = head.split_whitespace().nth(1)?.to_string();
        let length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .unwrap_or(0);
        while request.len() < head_end + length {
            let n = socket.read(&mut buf).await.ok()?;
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let body = serde_json::from_slice(&request[head_end..]).unwrap_or(Value::Null);
        Some((path, body))
    }

    /// Renews every sign-in as `<old>-2`, for eight hours.
    fn renews() -> Reply {
        Arc::new(|body| {
            let refresh = body["refresh_token"].as_str().unwrap_or_default();
            let account = refresh.trim_start_matches("rt-");
            (
                200,
                json!({"access_token": format!("at-{account}-2"), "refresh_token": format!("rt-{account}-2"),
                    "expires_in": 28_800, "scope": "user:inference user:profile", "token_type": "Bearer"}),
            )
        })
    }

    fn answers(status: u16, error: &'static str) -> Reply {
        Arc::new(move |_| (status, json!({"error": error})))
    }

    /// A sign-in as Claude Code writes it, for account `name`.
    fn sign_in(name: &str, refresh: bool, expires_at_ms: u64) -> Value {
        let mut oauth = json!({"accessToken": format!("at-{name}"), "expiresAt": expires_at_ms,
            "scopes": ["user:inference", "user:profile"], "subscriptionType": "max",
            "rateLimitTier": "default_claude_max_20x"});
        if refresh {
            oauth["refreshToken"] = json!(format!("rt-{name}"));
        }
        json!({"claudeAiOauth": oauth})
    }

    fn write(path: &Path, value: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn slot_file(home: &Home) -> std::path::PathBuf {
        home.paths.claude_home.join(".credentials.json")
    }

    fn folder_file(home: &Home, id: &str) -> std::path::PathBuf {
        home.paths.account_dir(id).join(".credentials.json")
    }

    /// Work signed in on the slot (`default`, global), Personal in its
    /// folder, the token endpoint at `oauth_url`.
    fn two_accounts(
        name: &str,
        oauth_url: &str,
        work: Value,
        personal: Value,
    ) -> (Home, Arc<Accounts>) {
        let home = Home::with_urls(name, "http://127.0.0.1:9", oauth_url);
        write(&slot_file(&home), &work);
        write(
            &home.paths.default_json,
            &json!({"oauthAccount": {"accountUuid": "uuid-work", "emailAddress": "work@x.dev", "displayName": "Work"}}),
        );
        let accounts = home.accounts();
        accounts
            .update_registry(|registry| {
                registry.accounts.push(registry::Entry {
                    id: "personal".into(),
                    name: "Personal".into(),
                    color: "green".into(),
                    shared: vec![],
                    created_at_ms: 1,
                });
                Ok(())
            })
            .unwrap();
        write(&folder_file(&home, "personal"), &personal);
        write(
            &home.paths.account_json("personal"),
            &json!({"oauthAccount": {"accountUuid": "uuid-personal", "emailAddress": "me@x.dev", "displayName": "Personal"}}),
        );
        accounts.renewals.set_wait(Duration::from_secs(5));
        (home, accounts)
    }

    fn token(raw: &Value) -> &str {
        raw["claudeAiOauth"]["accessToken"]
            .as_str()
            .unwrap_or_default()
    }

    /// No lock and no temporary file left in `dir`, nor the legacy lock
    /// beside it.
    fn nothing_left(dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.ends_with(".lock") && !name.contains("tmp"),
                "{name} left in {}",
                dir.display()
            );
        }
        assert!(!super::super::lockdir::legacy_path(dir).exists());
    }

    #[tokio::test]
    async fn a_token_that_works_is_answered_as_it_is() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let now = now_ms();
        let (_home, accounts) = two_accounts(
            "cred-fresh",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 2 * HOUR),
        );
        let work = accounts.credential("default").await.unwrap();
        assert_eq!(work.access_token.expose(), "at-work");
        assert_eq!(work.expires_at_ms, now + HOUR);
        assert_eq!(work.email.as_deref(), Some("work@x.dev"));
        assert_eq!(work.plan.as_deref(), Some("max"));
        let personal = accounts.credential(" personal ").await.unwrap();
        assert_eq!(personal.account, "personal");
        assert_eq!(personal.access_token.expose(), "at-personal");
        assert_eq!(personal.email.as_deref(), Some("me@x.dev"));
        assert!(server.asked().is_empty());
        assert_eq!(accounts.renewals.asked(), 0);
        for id in ["global", "nobody", "../personal", ""] {
            let error = accounts.credential(id).await.unwrap_err();
            assert_eq!(error.code, "unknown_account", "{id}");
        }
        let wire: Value =
            serde_json::from_str(&accounts.credential("global").await.unwrap_err().to_wire())
                .unwrap();
        assert_eq!(wire["code"], "unknown_account");
        assert!(wire["message"].as_str().unwrap().contains("global"));
    }

    #[tokio::test]
    async fn a_token_about_to_expire_is_renewed_in_its_own_store() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let now = now_ms();
        let mut personal = sign_in("personal", true, now + 60_000);
        personal["claudeAiOauth"]["somethingNew"] = json!("kept");
        personal["mcpOAuth"] = json!({"server": {"accessToken": "mcp-token"}});
        let (home, accounts) = two_accounts(
            "cred-renew",
            &server.url,
            sign_in("work", true, now + HOUR),
            personal,
        );
        let renewed = accounts.credential("personal").await.unwrap();
        assert_eq!(renewed.access_token.expose(), "at-personal-2");
        assert!(renewed.expires_at_ms >= now + 28_800_000);
        assert!(renewed.expires_at_ms <= now_ms() + 28_800_000);
        assert_eq!(renewed.email.as_deref(), Some("me@x.dev"));
        assert_eq!(renewed.plan.as_deref(), Some("max"));
        // Asked once, as Claude Code asks.
        let asked = server.asked();
        assert_eq!(asked.len(), 1);
        let (path, body) = &asked[0];
        assert_eq!(path, "/v1/oauth/token");
        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "rt-personal");
        assert_eq!(body["client_id"], oauth::CLIENT_ID);
        assert_eq!(body["scope"], oauth::DEFAULT_SCOPES.join(" "));
        // Written back in its folder, everything else kept.
        let stored = read(&folder_file(&home, "personal"));
        let entry = &stored["claudeAiOauth"];
        assert_eq!(entry["accessToken"], "at-personal-2");
        assert_eq!(entry["refreshToken"], "rt-personal-2");
        assert_eq!(entry["expiresAt"], renewed.expires_at_ms);
        assert_eq!(entry["scopes"], json!(["user:inference", "user:profile"]));
        assert_eq!(entry["subscriptionType"], "max");
        assert_eq!(entry["rateLimitTier"], "default_claude_max_20x");
        assert_eq!(entry["somethingNew"], "kept");
        assert_eq!(stored["mcpOAuth"]["server"]["accessToken"], "mcp-token");
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(folder_file(&home, "personal"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "private: {mode:o}");
        }
        nothing_left(&home.paths.account_dir("personal"));
        // The global account's sign-in is not touched.
        assert_eq!(token(&read(&slot_file(&home))), "at-work");
        // Renewed once: the next ask is answered from the store.
        let again = accounts.credential("personal").await.unwrap();
        assert_eq!(again.access_token.expose(), "at-personal-2");
        assert_eq!(server.asked().len(), 1);
    }

    #[tokio::test]
    async fn the_global_account_is_renewed_on_the_slot() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let now = now_ms();
        let (home, accounts) = two_accounts(
            "cred-slot",
            &server.url,
            sign_in("work", true, now + 1_000),
            sign_in("personal", true, now + HOUR),
        );
        let work = accounts.credential("default").await.unwrap();
        assert_eq!(work.access_token.expose(), "at-work-2");
        assert_eq!(
            read(&slot_file(&home))["claudeAiOauth"]["refreshToken"],
            "rt-work-2"
        );
        assert!(!folder_file(&home, "default").exists(), "not in its folder");
        nothing_left(&home.paths.claude_home);
        assert_eq!(token(&read(&folder_file(&home, "personal"))), "at-personal");
    }

    #[tokio::test]
    async fn without_accounts_json_the_slot_is_renewed_and_nothing_is_made() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let home = Home::with_urls("cred-plain", "http://127.0.0.1:9", &server.url);
        write(&slot_file(&home), &sign_in("work", true, now_ms() - 1_000));
        let accounts = home.accounts();
        assert!(!accounts.managed());
        let work = accounts.credential("default").await.unwrap();
        assert_eq!(work.access_token.expose(), "at-work-2");
        assert_eq!(token(&read(&slot_file(&home))), "at-work-2");
        assert!(!home.paths.root.exists(), "no accounts folder made");
        assert_eq!(
            accounts.credential("personal").await.unwrap_err().code,
            "unknown_account"
        );
    }

    #[tokio::test]
    async fn one_renewal_at_a_time_per_account() {
        let server = TokenServer::start(Duration::from_millis(200), renews()).await;
        let now = now_ms();
        let (_home, accounts) = two_accounts(
            "cred-once",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 60_000),
        );
        let asks: Vec<_> = (0..6)
            .map(|_| {
                let accounts = accounts.clone();
                tokio::spawn(async move { accounts.credential("personal").await })
            })
            .collect();
        for ask in asks {
            let credential = ask.await.unwrap().unwrap();
            assert_eq!(credential.access_token.expose(), "at-personal-2");
        }
        assert_eq!(server.asked().len(), 1, "one renewal for six asks");
        assert_eq!(accounts.renewals.asked(), 1);
    }

    #[tokio::test]
    async fn a_claude_code_that_renewed_first_is_taken_as_it_is() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let now = now_ms();
        let (home, accounts) = two_accounts(
            "cred-theirs",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 60_000),
        );
        // Another process (a Claude Code on Personal) holds the refresh
        // lock, renews, writes, and lets go.
        let folder = home.paths.account_dir("personal");
        let lock = folder.join(super::super::lockdir::REFRESH);
        std::fs::create_dir(&lock).unwrap();
        let mut theirs = sign_in("personal-theirs", true, now + 8 * HOUR);
        theirs["claudeAiOauth"]["refreshToken"] = json!("rt-theirs");
        let mut other = std::process::Command::new("sh")
            .arg("-c")
            .arg(r#"sleep 0.4 && printf '%s' "$1" > "$2.new" && mv "$2.new" "$2" && rmdir "$3""#)
            .arg("claude")
            .arg(theirs.to_string())
            .arg(folder_file(&home, "personal"))
            .arg(&lock)
            .spawn()
            .unwrap();
        let credential = accounts.credential("personal").await.unwrap();
        assert!(other.wait().unwrap().success());
        assert_eq!(credential.access_token.expose(), "at-personal-theirs");
        assert_eq!(credential.expires_at_ms, now + 8 * HOUR);
        assert!(server.asked().is_empty(), "never renewed twice");
        assert_eq!(
            read(&folder_file(&home, "personal"))["claudeAiOauth"]["refreshToken"],
            "rt-theirs"
        );
        nothing_left(&folder);
    }

    #[tokio::test]
    async fn no_swap_runs_while_a_renewal_does_and_the_next_one_moves_it() {
        let seen: Arc<Mutex<Option<(bool, bool)>>> = Arc::new(Mutex::new(None));
        let under: Arc<Mutex<Option<Arc<Accounts>>>> = Arc::new(Mutex::new(None));
        let during: During = {
            let (seen, under) = (seen.clone(), under.clone());
            Arc::new(move || {
                let accounts = under.lock().unwrap().clone().unwrap();
                // This worker's own switch waits for the gate; another
                // worker process's finds swap.lock held.
                let gate_free = accounts.try_gate().is_some();
                let swapped = slot::swap(
                    &accounts.stores().unwrap(),
                    "default",
                    "personal",
                    None,
                    &|_| Ok(()),
                    Crash::None,
                );
                *seen.lock().unwrap() = Some((gate_free, matches!(swapped, Err(SwapError::Busy))));
            })
        };
        let server = TokenServer::with(Duration::from_millis(100), renews(), during).await;
        let now = now_ms();
        let (home, accounts) = two_accounts(
            "cred-swap-after",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 60_000),
        );
        *under.lock().unwrap() = Some(accounts.clone());
        let renewed = accounts.credential("personal").await.unwrap();
        assert_eq!(renewed.access_token.expose(), "at-personal-2");
        assert_eq!(
            *seen.lock().unwrap(),
            Some((false, true)),
            "the gate and swap.lock were held"
        );
        assert_eq!(accounts.registry().global, "default");
        // The switch that waited runs now and moves the renewed sign-in.
        let gate = accounts.gate().await;
        slot::swap(
            &accounts.stores().unwrap(),
            "default",
            "personal",
            None,
            &|journal| accounts.commit_swap(journal),
            Crash::None,
        )
        .unwrap();
        drop(gate);
        let slot = read(&slot_file(&home));
        assert_eq!(token(&slot), "at-personal-2");
        assert_eq!(slot["claudeAiOauth"]["refreshToken"], "rt-personal-2");
        let personal = accounts.credential("personal").await.unwrap();
        assert_eq!(personal.access_token.expose(), "at-personal-2");
        assert_eq!(server.asked().len(), 1);
    }

    #[tokio::test]
    async fn a_swap_first_decides_where_the_renewal_reads_and_writes() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let now = now_ms();
        let (home, accounts) = two_accounts(
            "cred-swap-first",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 60_000),
        );
        // A switch to Personal holds the gate when the renewal is asked.
        let gate = accounts.gate().await;
        let asking = {
            let accounts = accounts.clone();
            tokio::spawn(async move { accounts.credential("personal").await })
        };
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!asking.is_finished(), "it waits for the gate");
        {
            let accounts = accounts.clone();
            tokio::task::spawn_blocking(move || {
                slot::swap(
                    &accounts.stores().unwrap(),
                    "default",
                    "personal",
                    None,
                    &|journal| accounts.commit_swap(journal),
                    Crash::None,
                )
            })
            .await
            .unwrap()
            .unwrap();
        }
        drop(gate);
        let renewed = asking.await.unwrap().unwrap();
        assert_eq!(renewed.access_token.expose(), "at-personal-2");
        assert_eq!(accounts.registry().global, "personal");
        // Renewed where Personal lives now: the slot.
        let slot = read(&slot_file(&home));
        assert_eq!(token(&slot), "at-personal-2");
        assert_eq!(slot["claudeAiOauth"]["refreshToken"], "rt-personal-2");
        assert_eq!(token(&read(&folder_file(&home, "default"))), "at-work");
        let left = std::fs::read_to_string(folder_file(&home, "personal")).unwrap_or_default();
        assert!(
            !left.contains("personal"),
            "nothing of Personal's left or written where it was: {left}"
        );
        assert_eq!(server.asked().len(), 1);
    }

    #[tokio::test]
    async fn a_keychain_sign_in_is_renewed_in_the_keychain() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let home = Home::with_urls("cred-keychain", "http://127.0.0.1:9", &server.url);
        let program = credentials::tests::fake_security(&home.dir.join("keychain"));
        let accounts = Arc::new(Accounts::new(
            Some(home.paths.clone()),
            Box::new(KeychainStore {
                program,
                account: "tester".into(),
            }),
            true,
        ));
        accounts.load_blocking();
        accounts
            .update_registry(|registry| {
                registry.accounts.push(registry::Entry {
                    id: "personal".into(),
                    name: "Personal".into(),
                    color: "green".into(),
                    shared: vec![],
                    created_at_ms: 1,
                });
                Ok(())
            })
            .unwrap();
        let place = Place::Folder("personal".into());
        let raw: Raw =
            serde_json::from_value(sign_in("personal", true, now_ms() + 60_000)).unwrap();
        accounts.stores().unwrap().write(&place, &raw).unwrap();
        let renewed = accounts.credential("personal").await.unwrap();
        assert_eq!(renewed.access_token.expose(), "at-personal-2");
        let stored = accounts.stores().unwrap().read(&place).unwrap().unwrap();
        let back = credentials::credentials_of(&stored);
        assert_eq!(back.token.unwrap().expose(), "at-personal-2");
        assert_eq!(back.refresh.unwrap().expose(), "rt-personal-2");
        assert!(
            !folder_file(&home, "personal").exists(),
            "no plaintext file is made"
        );
        let log = std::fs::read_to_string(home.dir.join("keychain/calls.log")).unwrap();
        assert!(log.contains("add-stdin tester"), "{log}");
        assert!(!log.contains("at-personal") && !log.contains("rt-personal"));
        assert_eq!(server.asked().len(), 1);
    }

    #[tokio::test]
    async fn a_refusal_or_nothing_to_renew_with_is_token_expired() {
        let server = TokenServer::start(Duration::ZERO, answers(400, "invalid_grant")).await;
        let now = now_ms();
        let (home, accounts) = two_accounts(
            "cred-refused",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now - 1_000),
        );
        let error = accounts.credential("personal").await.unwrap_err();
        assert_eq!(error.code, "token_expired");
        assert_eq!(server.asked().len(), 1);
        assert_eq!(
            read(&folder_file(&home, "personal"))["claudeAiOauth"]["refreshToken"],
            "rt-personal",
            "a refusal changes nothing in the store"
        );
        // Refused while the token still works for a minute: that token.
        write(
            &folder_file(&home, "personal"),
            &sign_in("personal", true, now_ms() + 60_000),
        );
        let kept = accounts.credential("personal").await.unwrap();
        assert_eq!(kept.access_token.expose(), "at-personal");
        assert_eq!(server.asked().len(), 2);
        // Expired, nothing to renew it with: not asked.
        write(
            &folder_file(&home, "personal"),
            &sign_in("personal", false, now_ms() - 1_000),
        );
        let error = accounts.credential("personal").await.unwrap_err();
        assert_eq!(error.code, "token_expired");
        assert_eq!(server.asked().len(), 2);
        // Signed out.
        std::fs::remove_file(folder_file(&home, "personal")).unwrap();
        let error = accounts.credential("personal").await.unwrap_err();
        assert_eq!(error.code, "not_signed_in");
        assert_eq!(server.asked().len(), 2);
    }

    #[tokio::test]
    async fn a_failure_that_may_pass_keeps_a_token_that_still_works() {
        let server = TokenServer::start(Duration::ZERO, answers(503, "overloaded")).await;
        let now = now_ms();
        let (home, accounts) = two_accounts(
            "cred-failed",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 60_000),
        );
        let kept = accounts.credential("personal").await.unwrap();
        assert_eq!(kept.access_token.expose(), "at-personal");
        write(
            &folder_file(&home, "personal"),
            &sign_in("personal", true, now_ms() - 1_000),
        );
        let error = accounts.credential("personal").await.unwrap_err();
        assert_eq!(error.code, "failed");
        assert_eq!(server.asked().len(), 2);
        assert_eq!(
            read(&folder_file(&home, "personal"))["claudeAiOauth"]["refreshToken"],
            "rt-personal"
        );
    }

    #[tokio::test]
    async fn a_scope_refused_is_asked_again_with_the_stored_ones() {
        let reply: Reply = Arc::new(|body| {
            if body["scope"]
                .as_str()
                .unwrap_or_default()
                .contains("user:plugins")
            {
                (400, json!({"error": "invalid_scope"}))
            } else {
                (
                    200,
                    json!({"access_token": "at-narrow", "refresh_token": "rt-narrow", "expires_in": 3600}),
                )
            }
        });
        let server = TokenServer::start(Duration::ZERO, reply).await;
        let now = now_ms();
        let (_home, accounts) = two_accounts(
            "cred-scope",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 60_000),
        );
        let renewed = accounts.credential("personal").await.unwrap();
        assert_eq!(renewed.access_token.expose(), "at-narrow");
        let asked = server.asked();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[1].1["scope"], "user:inference user:profile");
    }

    #[tokio::test]
    async fn anything_held_too_long_is_busy() {
        let server = TokenServer::start(Duration::ZERO, renews()).await;
        let now = now_ms();
        let (home, accounts) = two_accounts(
            "cred-busy",
            &server.url,
            sign_in("work", true, now + HOUR),
            sign_in("personal", true, now + 60_000),
        );
        accounts.renewals.set_wait(Duration::from_millis(300));
        // Another worker process swapping.
        let held = accounts.stores().unwrap().hold_swap().unwrap();
        assert_eq!(
            accounts.credential("personal").await.unwrap_err().code,
            "busy"
        );
        drop(held);
        // This worker's own switch.
        let gate = accounts.gate().await;
        assert_eq!(
            accounts.credential("personal").await.unwrap_err().code,
            "busy"
        );
        drop(gate);
        // A Claude Code writing Personal's sign-in.
        let lock = home
            .paths
            .account_dir("personal")
            .join(super::super::lockdir::STORAGE);
        std::fs::create_dir(&lock).unwrap();
        assert_eq!(
            accounts.credential("personal").await.unwrap_err().code,
            "busy"
        );
        std::fs::remove_dir(&lock).unwrap();
        assert!(server.asked().is_empty());
        nothing_left(&home.paths.account_dir("personal"));
        // Nothing held: renewed.
        let renewed = accounts.credential("personal").await.unwrap();
        assert_eq!(renewed.access_token.expose(), "at-personal-2");
    }
}
