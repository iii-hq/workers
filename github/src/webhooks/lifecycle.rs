use super::*;
use sha2::Digest;

// Hash both components so the longest valid identities fit the 128-byte limit.
pub(super) fn consumer_id(installation: &str, watch_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(installation.as_bytes());
    hash.update([0]);
    hash.update(watch_id.as_bytes());
    format!("github:{}", hex::encode(hash.finalize()))
}

pub(super) const TUNNEL_JOB_PREFIX: &str = "lifecycle:tunnel:";

fn terminal_state(w: &Watch) -> WatchState {
    if w.snapshot.merged
        || (w.spec.stop_on == StopOn::Closed && w.snapshot.state.as_deref() == Some("closed"))
    {
        WatchState::Completed
    } else if w.spec.expires_at <= Utc::now() {
        WatchState::Expired
    } else {
        WatchState::Stopped
    }
}

impl Service {
    pub(super) async fn watch(&self, req: WatchRequest) -> Result<WatchResponse> {
        let _guard = self.operations.lock().await;
        let spec = req.validate()?;
        self.store()?.change(|d| {
            if let Some(w) = d.watches.get(&spec.watch_id) {
                return if w.spec == spec {
                    Ok(())
                } else {
                    Err(Failure::Conflict)
                };
            }
            let now = Utc::now();
            if spec.expires_at <= now || spec.expires_at > now + chrono::Duration::days(30) {
                return Err(Failure::Invalid(
                    "expires_at must be in the future and no more than 30 days away (quick-tunnel lease limit)".into(),
                ));
            }
            d.repos
                .entry(spec.repo.clone())
                .or_insert_with(|| RepoHook {
                    endpoint_id: store::random_id(),
                    secret: store::random_id(),
                    hook_id: None,
                    url: None,
                    pending_url: None,
                    generation: None,
                    create_started: false,
                    cleanup_attempts: 0,
                    error: None,
                });
            d.watches.insert(
                spec.watch_id.clone(),
                Watch {
                    spec: spec.clone(),
                    status: WatchState::Preparing,
                    snapshot: Snapshot::default(),
                    lease_id: None,
                    error: None,
                    seen: Default::default(),
                },
            );
            Ok(())
        })?;
        if self
            .store()?
            .read()?
            .watches
            .get(&spec.watch_id)
            .is_some_and(Watch::live)
        {
            if let Err(e) = self.prepare_watch(&spec.watch_id).await {
                // Even an unavailable tunnel must not disguise an already-merged PR.
                let reconciliation = self
                    .reconcile_watch(&spec.watch_id, "initial-fallback")
                    .await;
                // Run both even when publication fails; retain the original
                // preparation error in health, never disguise a failed setup.
                let publication = self.publish_pending().await;
                let cleanup = self.cleanup().await;
                self.watch_error(&spec.watch_id, &e)?;
                self.record_failure(&e)?;
                reconciliation?;
                publication?;
                cleanup?;
                return Err(e);
            }
        }
        self.status(&spec.watch_id)
    }
    fn watch_error(&self, id: &str, e: &Failure) -> Result<()> {
        self.store()?.change(|d| {
            if let Some(w) = d.watches.get_mut(id) {
                w.error = Some(e.to_string());
            }
            Ok(())
        })
    }
    async fn acquire_lease(&self, id: &str) -> Result<()> {
        let data = self.store()?.read()?;
        let w = data.watches.get(id).ok_or(Failure::NotFound)?;
        let lease = self.invoke("quick-tunnel::acquire", json!({"consumer_id": consumer_id(&data.installation, id), "tunnel_id": self.config.tunnel_id, "expires_at": w.spec.expires_at.to_rfc3339()})).await?;
        let lease_id = lease["lease_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| Failure::Invalid("tunnel response missing lease_id".into()))?
            .to_owned();
        // Keep the previous lease reachable for cleanup until acquire succeeds.
        self.store()?.change(|d| {
            let w = d.watches.get_mut(id).ok_or(Failure::NotFound)?;
            w.lease_id = Some(lease_id);
            Ok(())
        })
    }
    async fn prepare_watch(&self, id: &str) -> Result<()> {
        let data = self.store()?.read()?;
        let w = data.watches.get(id).ok_or(Failure::NotFound)?;
        if w.lease_id.is_none() {
            self.acquire_lease(id).await?;
        }
        // Registration was submitted before acquire; the SDK does not expose
        // binding acknowledgement. One status read closes the ready-event race.
        let status = self
            .invoke(
                "quick-tunnel::status",
                json!({"tunnel_id":self.config.tunnel_id}),
            )
            .await?;
        let status: TunnelSnapshot = serde_json::from_value(status)?;
        self.apply_tunnel(&status).await?;
        self.reconcile_watch(id, "initial").await?;
        let publication = self.publish_pending().await;
        let cleanup = self.cleanup().await;
        publication.and(cleanup)
    }
    pub(super) async fn tunnel_changed(&self, status: TunnelSnapshot) -> Result<OperationResponse> {
        if status.tunnel_id != self.config.tunnel_id {
            return self.operation_response();
        }
        // No operation lock or external I/O inside the tunnel's 5-second callback.
        // Distinct durable jobs prevent a new arrival being removed by an older ack.
        self.store()?.change(|d| {
            if d.jobs.len() >= self.config.max_pending {
                return Err(Failure::Capacity);
            }
            // A queued change is not proof that the previous URL is healthy.
            d.tunnel_status = "pending".into();
            d.jobs.insert(
                format!("{TUNNEL_JOB_PREFIX}{}", store::random_id()),
                Job::Inbox(Inbox {
                    repo: String::new(),
                    event: "tunnel-lifecycle".into(),
                    delivery: status.generation.clone(),
                    body: serde_json::to_value(status)?,
                }),
            );
            Ok(())
        })?;
        self.operation_response()
    }
    pub(super) async fn process_tunnel(&self, event: &TunnelSnapshot) -> Result<()> {
        // UUID generations are identities, not sortable versions. Always use
        // this ONE authoritative read, including for delayed failure/ready events.
        let current = self
            .invoke(
                "quick-tunnel::status",
                json!({"tunnel_id":self.config.tunnel_id}),
            )
            .await;
        let current: TunnelSnapshot = match current.and_then(|v| Ok(serde_json::from_value(v)?)) {
            Ok(current) => current,
            Err(e) => {
                self.store()?.change(|d| {
                    d.tunnel_status = "unknown".into();
                    Ok(())
                })?;
                self.record_failure(&e)?;
                return Err(e);
            }
        };
        if current.generation != event.generation {
            tracing::debug!("discarding stale tunnel event; applying current snapshot");
        }
        self.apply_tunnel(&current).await
    }
    pub(super) async fn apply_tunnel(&self, status: &TunnelSnapshot) -> Result<()> {
        if status.tunnel_id != self.config.tunnel_id {
            return Ok(());
        }
        self.store()?.change(|d| {
            d.tunnel_status = if status.status == "ready" && status.error.is_some() {
                "failed".into()
            } else {
                status.status.clone()
            };
            if status.status != "ready" || status.error.is_some() {
                for w in d.watches.values_mut().filter(|w| w.live()) {
                    w.status = WatchState::Preparing;
                    w.error = status.error.clone();
                }
            }
            Ok(())
        })?;
        if status.status != "ready" || status.error.is_some() {
            return Ok(());
        }
        let url = status
            .public_url
            .as_deref()
            .filter(|s| s.starts_with("https://") && !s.contains(['?', '#', '@']))
            .ok_or_else(|| Failure::Invalid("ready tunnel lacks safe HTTPS public_url".into()))?;
        let repos: Vec<_> = self.store()?.read()?.repos.keys().cloned().collect();
        let mut first_error = None;
        for repo in repos {
            if !self
                .store()?
                .read()?
                .watches
                .values()
                .any(|w| w.live() && w.spec.repo == repo)
            {
                continue;
            }
            let result = async {
                let changed = self.ensure_hook(&repo, url, &status.generation).await?;
                if changed
                    || self.store()?.read()?.watches.values().any(|w| {
                        w.live() && w.spec.repo == repo && w.status == WatchState::Preparing
                    })
                {
                    let ids: Vec<_> = self
                        .store()?
                        .read()?
                        .watches
                        .values()
                        .filter(|w| w.live() && w.spec.repo == repo)
                        .map(|w| w.spec.watch_id.clone())
                        .collect();
                    for id in ids {
                        self.reconcile_watch(&id, "url-ready").await?;
                    }
                    self.redeliver(&repo).await?;
                }
                Ok(())
            }
            .await;
            if let Err(e) = result {
                // Persist reconciliation/redelivery failures too, so a retry
                // does not mistake a previously PATCHed hook for finished work.
                self.repo_error(&repo, &e)?;
                first_error.get_or_insert(e);
            }
        }
        let publication = self.publish_pending().await;
        let cleanup = self.cleanup().await;
        first_error
            .map_or(Ok(()), Err)
            .and(publication)
            .and(cleanup)
    }
    fn repo_error(&self, repo: &str, e: &Failure) -> Result<()> {
        self.store()?.change(|d| {
            if let Some(h) = d.repos.get_mut(repo) {
                h.error = Some(e.to_string());
            }
            Ok(())
        })
    }
    pub(super) async fn ensure_hook(
        &self,
        repo: &str,
        public_url: &str,
        generation: &str,
    ) -> Result<bool> {
        self.reconcile_create(repo).await?;
        let data = self.store()?.read()?;
        let h = data.repos.get(repo).ok_or(Failure::NotFound)?;
        let url = format!(
            "{}/webhooks/github/{}",
            public_url.trim_end_matches('/'),
            h.endpoint_id
        );
        if h.url.as_deref() == Some(&url)
            && h.generation.as_deref() == Some(generation)
            && h.hook_id.is_some()
            && h.error.is_none()
            && h.pending_url.is_none()
        {
            return Ok(false);
        }
        let config =
            json!({"url": url, "content_type":"json", "insecure_ssl":"0", "secret":h.secret});
        let id = if let Some(id) = h.hook_id {
            self.patch_owned(repo, h, &url, config).await?;
            id
        } else {
            self.store()?.change(|d| {
                let h = d.repos.get_mut(repo).ok_or(Failure::NotFound)?;
                if h.create_started {
                    return Err(Failure::AmbiguousHook);
                }
                // Commit the exact random endpoint BEFORE the side effect. A
                // crash or a rejected/lost response must be reconcilable later.
                h.url = Some(url.clone());
                h.generation = None;
                h.create_started = true;
                Ok(())
            })?;
            let created = self
                .api(
                    "POST",
                    &format!("repos/{repo}/hooks"),
                    Some(
                        json!({"name":"web","active":true,"events":hook_events(),"config":config}),
                    ),
                )
                .await
                .and_then(|created| created["id"].as_u64().ok_or(Failure::AmbiguousHook));
            match created {
                Ok(id) => id,
                Err(error) => {
                    self.reconcile_create(repo).await?;
                    let data = self.store()?.read()?;
                    let h = data.repos.get(repo).ok_or(Failure::NotFound)?;
                    let id = h.hook_id.ok_or(error)?;
                    // GET/list masks the secret: reapply our configuration only
                    // after verifying exact ownership, retaining PATCH intent.
                    self.patch_owned(repo, h, &url, config).await?;
                    id
                }
            }
        };
        self.store()?.change(|d| {
            let h = d.repos.get_mut(repo).ok_or(Failure::NotFound)?;
            h.hook_id = Some(id);
            h.url = Some(url);
            h.pending_url = None;
            h.generation = Some(generation.into());
            h.error = None;
            Ok(())
        })?;
        Ok(true)
    }
    async fn reconcile_create(&self, repo: &str) -> Result<()> {
        let data = self.store()?.read()?;
        let hook = data.repos.get(repo).ok_or(Failure::NotFound)?;
        if hook.hook_id.is_some() || !hook.create_started {
            return Ok(());
        }
        // Legacy intents lack the original tunnel URL. Neither the current URL
        // nor an endpoint suffix proves their absence/ownership: fail closed.
        let url = hook
            .url
            .as_deref()
            .filter(|url| {
                !hook.endpoint_id.is_empty()
                    && url.starts_with("https://")
                    && !url.contains(['?', '#', '@'])
                    && url.ends_with(&format!("/webhooks/github/{}", hook.endpoint_id))
            })
            .ok_or(Failure::AmbiguousHook)?;
        let base = format!("repos/{repo}/hooks");
        let mut endpoint = format!("{base}?per_page=100");
        let mut seen = std::collections::BTreeSet::new();
        let mut owned = std::collections::BTreeSet::new();
        for _ in 0..5 {
            if !seen.insert(endpoint.clone()) {
                return Err(Failure::AmbiguousHook);
            }
            let output = self.api_output("GET", &endpoint, None, true).await?;
            let (hooks, next) = hook_page(&output, &base)?;
            let hooks = hooks.as_array().ok_or(Failure::AmbiguousHook)?;
            for actual in hooks {
                if actual.pointer("/config/url").and_then(Value::as_str) == Some(url) {
                    owned.insert(actual["id"].as_u64().ok_or(Failure::AmbiguousHook)?);
                }
            }
            if let Some(next) = next {
                endpoint = next;
                continue;
            }
            if owned.len() > 1 {
                return Err(Failure::AmbiguousHook);
            }
            // Only a COMPLETE listing establishes absence or a unique match.
            return self.store()?.change(|d| {
                let h = d.repos.get_mut(repo).ok_or(Failure::NotFound)?;
                if let Some(id) = owned.first() {
                    h.hook_id = Some(*id);
                    h.generation = None;
                    h.error = Some("recovered hook requires secret/configuration PATCH".into());
                } else {
                    h.create_started = false;
                    h.error = None;
                }
                Ok(())
            });
        }
        Err(Failure::AmbiguousHook)
    }
    /// Persist both the observed URL and new intent before an owned hook PATCH.
    async fn patch_owned(
        &self,
        repo: &str,
        hook: &RepoHook,
        url: &str,
        config: Value,
    ) -> Result<()> {
        let observed_url = self.verify_owned(repo, hook).await?;
        let id = hook.hook_id.ok_or(Failure::Ownership)?;
        self.store()?.change(|d| {
            let h = d.repos.get_mut(repo).ok_or(Failure::NotFound)?;
            h.url = Some(observed_url);
            h.pending_url = Some(url.to_owned());
            h.generation = None;
            h.error = Some("hook update requires configuration confirmation".into());
            Ok(())
        })?;
        self.api(
            "PATCH",
            &format!("repos/{repo}/hooks/{id}"),
            Some(json!({"active":true,"config":config,"events":hook_events()})),
        )
        .await?;
        Ok(())
    }
    /// Verify the hook ID and an exact confirmed or durably intended URL.
    async fn verify_owned(&self, repo: &str, hook: &RepoHook) -> Result<String> {
        let id = hook.hook_id.ok_or(Failure::Ownership)?;
        let actual = self
            .api("GET", &format!("repos/{repo}/hooks/{id}"), None)
            .await?;
        let url = actual
            .pointer("/config/url")
            .and_then(Value::as_str)
            .ok_or(Failure::Ownership)?;
        if actual["id"].as_u64() != Some(id)
            || (Some(url) != hook.url.as_deref() && Some(url) != hook.pending_url.as_deref())
        {
            return Err(Failure::Ownership);
        }
        Ok(url.to_owned())
    }
    pub(super) async fn reconcile_watch(&self, id: &str, source: &str) -> Result<()> {
        let data = self.store()?.read()?;
        let w = data.watches.get(id).ok_or(Failure::NotFound)?;
        if !w.live() {
            return Ok(());
        }
        // Bindings were submitted, not acknowledged. A starting tunnel can
        // resolve an already-merged PR, but an open watch must stay Preparing.
        let pr = self
            .api(
                "GET",
                &format!("repos/{}/pulls/{}", w.spec.repo, w.spec.number),
                None,
            )
            .await?;
        let snapshot = normalize::snapshot(&pr, &w.snapshot)?;
        // A one-time baseline covers CI that completed before the webhook was
        // installed. Do not let an unrelated CI read block terminal cleanup.
        let terminal = snapshot.merged
            || (w.spec.stop_on == StopOn::Closed && snapshot.state.as_deref() == Some("closed"));
        let snapshot = if !terminal && w.spec.events.contains(&Category::Ci) {
            self.reconcile_ci(&w.spec.repo, snapshot).await?
        } else {
            snapshot
        };
        let policy = self.cell.read().await.webhooks.notifications.clone();
        self.store()?.change(|d| {
            let w = d.watches.get_mut(id).ok_or(Failure::NotFound)?;
            let changed = w.snapshot != snapshot || w.status == WatchState::Preparing;
            w.snapshot = snapshot;
            w.status = if d.tunnel_status == "ready"
                && d.repos.get(&w.spec.repo).is_some_and(|h| {
                    h.hook_id.is_some() && h.error.is_none() && h.pending_url.is_none()
                }) {
                WatchState::Active
            } else {
                WatchState::Preparing
            };
            w.error = None;
            let final_event = normalize::finish_if_needed(w);
            if changed || final_event {
                let kind = if w.snapshot.merged {
                    "merged"
                } else if final_event {
                    "closed"
                } else {
                    "snapshot"
                };
                let event =
                    normalize::make_event(w, Category::Pr, kind, "snapshot", 0, final_event);
                normalize::persist_event_with_policy(d, event, source, &policy);
            }
            Ok(())
        })
    }
    pub(super) async fn unwatch(&self, id: &str) -> Result<WatchResponse> {
        let _guard = self.operations.lock().await;
        self.store()?.change(|d| {
            let w = d.watches.get_mut(id).ok_or(Failure::NotFound)?;
            if w.live() {
                w.status = WatchState::Stopped;
            }
            Ok(())
        })?;
        if let Err(e) = self.cleanup().await {
            self.watch_error(id, &e)?;
        }
        self.status(id)
    }
    pub(super) async fn cleanup(&self) -> Result<()> {
        let data = self.store()?.read()?;
        let mut first_error = None;
        for (repo, hook) in &data.repos {
            if data
                .watches
                .values()
                .any(|w| w.live() && &w.spec.repo == repo)
                || hook.cleanup_attempts >= 5
            {
                continue;
            }
            let result = async {
                self.reconcile_create(repo).await?;
                let current = self.store()?.read()?;
                let hook = current.repos.get(repo).ok_or(Failure::NotFound)?;
                if let Some(id) = hook.hook_id {
                    self.verify_owned(repo, hook).await?;
                    self.api("DELETE", &format!("repos/{repo}/hooks/{id}"), None)
                        .await?;
                } else if hook.create_started {
                    return Err(Failure::AmbiguousHook);
                }
                Ok(())
            }
            .await;
            match result {
                Err(e) => {
                    self.store()?.change(|d| {
                        if let Some(h) = d.repos.get_mut(repo) {
                            h.cleanup_attempts += 1;
                            h.error = Some(e.to_string());
                        }
                        for w in d.watches.values_mut().filter(|w| &w.spec.repo == repo) {
                            w.status = WatchState::CleanupPending;
                            w.error = Some(e.to_string());
                        }
                        Ok(())
                    })?;
                    first_error.get_or_insert(e);
                }
                Ok(()) => {
                    self.store()?.change(|d| {
                        d.repos.remove(repo);
                        Ok(())
                    })?;
                }
            }
        }
        let data = self.store()?.read()?;
        for (id, w) in &data.watches {
            if w.live() {
                continue;
            }
            // Only a failed cleanup in THIS repo blocks this lease. A different
            // repository must not prevent independent cleanup/release work.
            if data.repos.contains_key(&w.spec.repo)
                && !data
                    .watches
                    .values()
                    .any(|x| x.live() && x.spec.repo == w.spec.repo)
            {
                continue;
            }
            if let Some(lease_id) = &w.lease_id {
                if let Err(e) = self
                    .invoke("quick-tunnel::release", json!({"lease_id":lease_id}))
                    .await
                {
                    self.store()?.change(|d| {
                        let w = d.watches.get_mut(id).ok_or(Failure::NotFound)?;
                        w.status = WatchState::CleanupPending;
                        w.error = Some(e.to_string());
                        Ok(())
                    })?;
                    first_error.get_or_insert(e);
                    continue;
                }
            }
            self.store()?.change(|d| {
                let w = d.watches.get_mut(id).ok_or(Failure::NotFound)?;
                w.lease_id = None;
                if w.status == WatchState::CleanupPending {
                    w.status = terminal_state(w);
                    w.error = None;
                }
                Ok(())
            })?;
        }
        first_error.map_or(Ok(()), Err)
    }
    /// Maintenance only expires deadlines and drains/retries pending local work.
    /// It NEVER reads a PR or lists GitHub deliveries on an interval.
    pub(super) async fn maintain(&self) -> Result<OperationResponse> {
        let _guard = self.operations.lock().await;
        self.maintain_locked().await
    }
    async fn maintain_locked(&self) -> Result<OperationResponse> {
        let policy = self.cell.read().await.webhooks.notifications.clone();
        self.store()?.change(|d| {
            let mut expired = Vec::new();
            for w in d
                .watches
                .values_mut()
                .filter(|w| w.live() && w.spec.expires_at <= Utc::now())
            {
                w.status = WatchState::Expired;
                expired.push(normalize::make_event(
                    w,
                    Category::Pr,
                    "expired",
                    "lifecycle",
                    0,
                    true,
                ));
            }
            for event in expired {
                normalize::persist_event_with_policy(d, event, "expiry", &policy);
            }
            Ok(())
        })?;
        let publication = self.publish_pending().await;
        let cleanup = self.cleanup().await;
        publication.and(cleanup)?;
        self.operation_response()
    }
    pub(super) async fn recover(&self, repo: Option<&str>) -> Result<OperationResponse> {
        let _guard = self.operations.lock().await;
        if let Some(repo) = repo {
            validate_repo(repo)?;
        }
        self.store()?.change(|d| {
            d.publications.clear();
            d.last_error = None;
            for h in d.repos.values_mut() {
                h.cleanup_attempts = 0;
            }
            Ok(())
        })?;
        let mut first_error = self.maintain_locked().await.err();
        let data = self.store()?.read()?;
        let ids: Vec<_> = data
            .watches
            .values()
            .filter(|w| w.live() && repo.is_none_or(|r| r.eq_ignore_ascii_case(&w.spec.repo)))
            .map(|w| w.spec.watch_id.clone())
            .collect();
        for id in ids {
            // Force the idempotent acquire to repair a restarted backend, but
            // do not discard the old lease if that call fails or is malformed.
            let result = async {
                self.acquire_lease(&id).await?;
                self.prepare_watch(&id).await
            }
            .await;
            if let Err(e) = result {
                self.watch_error(&id, &e)?;
                first_error.get_or_insert(e);
            }
        }
        let data = self.store()?.read()?;
        for name in data
            .repos
            .keys()
            .filter(|r| repo.is_none_or(|x| x.eq_ignore_ascii_case(r)))
        {
            let result = async {
                self.reconcile_create(name).await?;
                self.redeliver(name).await
            }
            .await;
            if let Err(e) = result {
                self.repo_error(name, &e)?;
                first_error.get_or_insert(e);
            }
        }
        // Both phases run even after any watch/repository failed.
        for result in [self.publish_pending().await, self.cleanup().await] {
            if let Err(e) = result {
                first_error.get_or_insert(e);
            }
        }
        if let Some(e) = first_error {
            self.record_failure(&e)?;
            return Err(e);
        }
        self.operation_response()
    }
    pub(super) async fn redeliver(&self, repo: &str) -> Result<()> {
        let data = self.store()?.read()?;
        let Some(h) = data.repos.get(repo) else {
            return Ok(());
        };
        let Some(id) = h.hook_id else {
            return Ok(());
        };
        self.verify_owned(repo, h).await?;
        // GitHub deliveries use opaque Link cursors, not numeric page offsets.
        // Follow at most five pages, and only within this owned hook's endpoint.
        let base = format!("repos/{repo}/hooks/{id}/deliveries");
        let mut endpoint = format!("{base}?per_page=100");
        let mut seen = std::collections::BTreeSet::new();
        let mut rows = Vec::new();
        let mut complete = false;
        for _ in 0..5 {
            if !seen.insert(endpoint.clone()) {
                return Err(Failure::Invalid("repeated delivery cursor".into()));
            }
            let output = self.api_output("GET", &endpoint, None, true).await?;
            let (deliveries, next) = delivery_page(&output, &base)?;
            rows.extend(
                deliveries
                    .as_array()
                    .ok_or_else(|| Failure::Invalid("invalid delivery list".into()))?
                    .iter()
                    .cloned(),
            );
            match next {
                Some(next) => endpoint = next,
                None => {
                    complete = true;
                    break;
                }
            }
        }
        // A successful retry can appear after an older failure, even on a later
        // page. Gather the bounded history before selecting failed GUIDs.
        let mut guids = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for row in &rows {
            if row["status_code"]
                .as_u64()
                .is_some_and(|s| (200..300).contains(&s))
            {
                if let Some(guid) = row["guid"].as_str().filter(|g| !g.is_empty()) {
                    guids.insert(guid.to_owned());
                }
                if let Some(id) = row["id"].as_u64() {
                    ids.insert(id);
                }
            }
        }
        for row in rows {
            let Some(delivery_id) = row["id"].as_u64() else {
                continue;
            };
            let guid = row["guid"].as_str().filter(|g| !g.is_empty());
            if ids.contains(&delivery_id) || guid.is_some_and(|g| guids.contains(g)) {
                continue;
            }
            // Ingress can durably accept deliveries while the history is being
            // fetched (or a prior POST runs). Never reuse the initial snapshot.
            if let Some(guid) = guid {
                if self
                    .store()?
                    .read()?
                    .deliveries
                    .contains(&format!("{id}:{guid}"))
                {
                    guids.insert(guid.to_owned());
                    continue;
                }
            }
            self.api(
                "POST",
                &format!("repos/{repo}/hooks/{id}/deliveries/{delivery_id}/attempts"),
                None,
            )
            .await?;
            ids.insert(delivery_id);
            if let Some(guid) = guid {
                guids.insert(guid.to_owned());
            }
        }
        if !complete {
            // A bounded history scan says nothing about the configured hook's
            // readiness. Keep it observable without poisoning RepoHook.error or
            // retrying a successfully configured URL through lifecycle jobs.
            self.store()?.change(|d| {
                d.last_error.get_or_insert_with(|| {
                    format!("recovery warning for {repo}: delivery recovery exceeds five pages; inspect older deliveries manually")
                });
                Ok(())
            })?;
        }
        Ok(())
    }
}

// Parse gh --include without ever forwarding credentials to a Link-provided host.
fn delivery_page(output: &str, base: &str) -> Result<(Value, Option<String>)> {
    let (headers, body) = output
        .split_once("\r\n\r\n")
        .or_else(|| output.split_once("\n\n"))
        .ok_or_else(|| Failure::Invalid("delivery response lacks headers".into()))?;
    let mut next = None;
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("link") {
            continue;
        }
        for link in value.split(',') {
            let mut parts = link.trim().split(';');
            let url = parts.next().unwrap_or_default().trim();
            if !parts.any(|part| part.trim() == "rel=\"next\"") {
                continue;
            }
            let endpoint = url
                .strip_prefix("<https://api.github.com/")
                .and_then(|url| url.strip_suffix('>'))
                .ok_or_else(|| Failure::Invalid("unsafe delivery pagination link".into()))?;
            let (path, query) = endpoint
                .split_once('?')
                .ok_or_else(|| Failure::Invalid("delivery pagination lacks cursor".into()))?;
            let mut keys = std::collections::BTreeSet::new();
            let valid = query.split('&').all(|field| {
                let Some((key, value)) = field.split_once('=') else {
                    return false;
                };
                keys.insert(key)
                    && match key {
                        "per_page" => value == "100",
                        "cursor" => {
                            !value.is_empty()
                                && value
                                    .bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"-._~%=+".contains(&b))
                        }
                        _ => false,
                    }
            });
            // GitHub canonicalizes repo slugs to numeric repository IDs in Link.
            // Accept only the same hook's deliveries route, then apply its cursor
            // to our already verified base; never follow a Link-provided repo.
            let canonical_path = path
                .strip_prefix("repositories/")
                .and_then(|path| path.split_once('/'))
                .is_some_and(|(repository_id, suffix)| {
                    repository_id.bytes().all(|b| b.is_ascii_digit())
                        && repository_id.parse::<u64>().is_ok_and(|id| id > 0)
                        && base
                            .rsplit_once("/hooks/")
                            .is_some_and(|(_, hook)| suffix == format!("hooks/{hook}"))
                });
            if (path != base && !canonical_path)
                || !valid
                || !keys.contains("cursor")
                || next.is_some()
            {
                return Err(Failure::Invalid("unsafe delivery pagination link".into()));
            }
            next = Some(format!("{base}?{query}"));
        }
    }
    Ok((serde_json::from_str(body)?, next))
}

// Repository hook lists use numeric pages; deliveries retain their separate
// opaque-cursor parser above. Never let a Link change the host or repository.
fn hook_page(output: &str, base: &str) -> Result<(Value, Option<String>)> {
    let (headers, body) = output
        .split_once("\r\n\r\n")
        .or_else(|| output.split_once("\n\n"))
        .ok_or(Failure::AmbiguousHook)?;
    let mut next = None;
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("link") {
            continue;
        }
        for link in value.split(',') {
            let mut parts = link.trim().split(';');
            let url = parts.next().unwrap_or_default().trim();
            if !parts.any(|part| part.trim() == "rel=\"next\"") {
                continue;
            }
            let endpoint = url
                .strip_prefix("<https://api.github.com/")
                .and_then(|url| url.strip_suffix('>'))
                .ok_or(Failure::AmbiguousHook)?;
            let (path, query) = endpoint.split_once('?').ok_or(Failure::AmbiguousHook)?;
            let mut keys = std::collections::BTreeSet::new();
            let valid = query.split('&').all(|field| {
                let Some((key, value)) = field.split_once('=') else {
                    return false;
                };
                keys.insert(key)
                    && match key {
                        "per_page" => value == "100",
                        "page" => {
                            !value.is_empty()
                                && value.bytes().all(|b| b.is_ascii_digit())
                                && value.parse::<u64>().is_ok_and(|p| p > 1)
                        }
                        _ => false,
                    }
            });
            if path != base || !valid || !keys.contains("page") || next.is_some() {
                return Err(Failure::AmbiguousHook);
            }
            next = Some(endpoint.to_owned());
        }
    }
    Ok((serde_json::from_str(body)?, next))
}

fn hook_events() -> Value {
    json!([
        "pull_request",
        "issue_comment",
        "pull_request_review_comment",
        "pull_request_review",
        "check_run",
        "status",
        "workflow_run"
    ])
}

#[cfg(test)]
mod delivery_tests {
    use super::*;

    const BASE: &str = "repos/owner/repo/hooks/42/deliveries";

    #[test]
    fn follows_opaque_cursor_from_link_and_accepts_last_page() {
        let endpoint = format!("{BASE}?per_page=100&cursor=opaque%3D");
        let response = format!(
            "HTTP/2.0 200 OK\r\nlink: <https://api.github.com/{endpoint}>; rel=\"next\"\r\n\r\n[]"
        );
        assert_eq!(delivery_page(&response, BASE).unwrap().1, Some(endpoint));
        assert_eq!(
            delivery_page("HTTP/2.0 200 OK\n\n[]", BASE).unwrap(),
            (json!([]), None)
        );
    }

    #[test]
    fn canonical_repository_link_keeps_the_verified_base_and_opaque_cursor() {
        let response = "HTTP/2.0 200 OK\r\nLink: <https://api.github.com/repositories/1185701685/hooks/42/deliveries?per_page=100&cursor=v1_3844358593348378624%3D>; rel=\"next\"\r\n\r\n[{\"id\":7}]";
        assert_eq!(
            delivery_page(response, BASE).unwrap(),
            (
                json!([{ "id": 7 }]),
                Some(format!(
                    "{BASE}?per_page=100&cursor=v1_3844358593348378624%3D"
                ))
            )
        );
    }

    #[test]
    fn canonical_repository_link_accepts_a_repository_named_hooks() {
        let base = "repos/github/hooks/hooks/42/deliveries";
        let response = "HTTP/2.0 200 OK\nLink: <https://api.github.com/repositories/123/hooks/42/deliveries?per_page=100&cursor=opaque%3D>; rel=\"next\"\n\n[]";
        assert_eq!(
            delivery_page(response, base).unwrap(),
            (
                json!([]),
                Some(format!("{base}?per_page=100&cursor=opaque%3D"))
            )
        );
    }

    #[test]
    fn rejects_multiple_next_links_even_with_a_canonical_alias() {
        let response = format!(
            "HTTP/2.0 200 OK\nLink: <https://api.github.com/{BASE}?cursor=a>; rel=\"next\", <https://api.github.com/repositories/123/hooks/42/deliveries?cursor=b>; rel=\"next\"\n\n[]"
        );
        assert!(delivery_page(&response, BASE).is_err());
    }

    #[test]
    fn rejects_links_outside_owned_hook_and_invalid_pagination() {
        for url in [
            format!("https://evil.example/{BASE}?cursor=a"),
            "https://evil.example/repositories/123/hooks/42/deliveries?cursor=a".into(),
            "https://api.github.com/repositories/123/hooks/99/deliveries?cursor=a".into(),
            "https://api.github.com/repositories/123/hooks/42/deliveries/7?cursor=a".into(),
            "https://api.github.com/repositories/123/hooks/42/../deliveries?cursor=a".into(),
            "https://api.github.com/repositories/123/hooks/42/deliveries?cursor=a&other=b".into(),
            "https://api.github.com/repositories/123/hooks/42/deliveries?cursor=a&cursor=b".into(),
            "https://api.github.com/repositories/0/hooks/42/deliveries?cursor=a".into(),
            "https://api.github.com/repositories/abc/hooks/42/deliveries?cursor=a".into(),
            "https://api.github.com/repositories/+123/hooks/42/deliveries?cursor=a".into(),
            "https://api.github.com/repositories//hooks/42/deliveries?cursor=a".into(),
            "https://api.github.com/repos/other/repo/hooks/42/deliveries?cursor=a".into(),
            format!("https://api.github.com/{BASE}?page=2"),
            format!("https://api.github.com/{BASE}?cursor="),
            format!("https://api.github.com/{BASE}?cursor=a&per_page=1000"),
            format!("https://api.github.com/{BASE}?cursor=a&cursor=b"),
        ] {
            let response = format!("HTTP/2.0 200 OK\nLink: <{url}>; rel=\"next\"\n\n[]");
            assert!(delivery_page(&response, BASE).is_err(), "{url}");
        }
    }
}
