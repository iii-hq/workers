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
    async fn prepare_watch(&self, id: &str) -> Result<()> {
        let data = self.store()?.read()?;
        let w = data.watches.get(id).ok_or(Failure::NotFound)?;
        if w.lease_id.is_none() {
            let lease = self.invoke("quick-tunnel::acquire", json!({"consumer_id": consumer_id(&data.installation, id), "tunnel_id": self.config.tunnel_id, "expires_at": w.spec.expires_at.to_rfc3339()})).await?;
            let lease_id = lease["lease_id"]
                .as_str()
                .ok_or_else(|| Failure::Invalid("tunnel response missing lease_id".into()))?
                .to_owned();
            self.store()?.change(|d| {
                if let Some(w) = d.watches.get_mut(id) {
                    w.lease_id = Some(lease_id);
                }
                Ok(())
            })?;
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
        {
            return Ok(false);
        }
        let config =
            json!({"url": url, "content_type":"json", "insecure_ssl":"0", "secret":h.secret});
        let id = if let Some(id) = h.hook_id {
            self.verify_owned(repo, h).await?;
            self.api(
                "PATCH",
                &format!("repos/{repo}/hooks/{id}"),
                Some(json!({"active":true,"config":config,"events":hook_events()})),
            )
            .await?;
            id
        } else {
            // A timed-out create is deliberately NOT retried/adopted. Its durable
            // intent remains visible; an operator must resolve the ambiguity.
            if h.create_started {
                return Err(Failure::AmbiguousHook);
            }
            self.store()?.change(|d| {
                let h = d.repos.get_mut(repo).ok_or(Failure::NotFound)?;
                if h.create_started {
                    return Err(Failure::AmbiguousHook);
                }
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
                .await?;
            created["id"].as_u64().ok_or(Failure::AmbiguousHook)?
        };
        self.store()?.change(|d| {
            let h = d.repos.get_mut(repo).ok_or(Failure::NotFound)?;
            h.hook_id = Some(id);
            h.url = Some(url);
            h.generation = Some(generation.into());
            h.error = None;
            Ok(())
        })?;
        Ok(true)
    }
    async fn verify_owned(&self, repo: &str, hook: &RepoHook) -> Result<()> {
        let id = hook.hook_id.ok_or(Failure::Ownership)?;
        let actual = self
            .api("GET", &format!("repos/{repo}/hooks/{id}"), None)
            .await?;
        if actual["id"].as_u64() != Some(id)
            || actual.pointer("/config/url").and_then(Value::as_str) != hook.url.as_deref()
        {
            return Err(Failure::Ownership);
        }
        Ok(())
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
        self.store()?.change(|d| {
            let w = d.watches.get_mut(id).ok_or(Failure::NotFound)?;
            let changed = w.snapshot != snapshot || w.status == WatchState::Preparing;
            w.snapshot = snapshot;
            w.status = if d.tunnel_status == "ready"
                && d.repos
                    .get(&w.spec.repo)
                    .is_some_and(|h| h.hook_id.is_some() && h.error.is_none())
            {
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
                normalize::persist_event(d, event, source);
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
                normalize::persist_event(d, event, "expiry");
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
        let maintenance = self.maintain_locked().await;
        if let Err(e) = &maintenance {
            self.store()?.change(|d| {
                d.last_error = Some(e.to_string());
                Ok(())
            })?;
        }
        let data = self.store()?.read()?;
        let ids: Vec<_> = data
            .watches
            .values()
            .filter(|w| w.live() && repo.is_none_or(|r| r.eq_ignore_ascii_case(&w.spec.repo)))
            .map(|w| w.spec.watch_id.clone())
            .collect();
        for id in ids {
            // Acquire is idempotent per consumer and expiry. Also repairs a lost
            // tunnel backend after restart; per-watch leases protect the latest expiry.
            self.store()?.change(|d| {
                if let Some(w) = d.watches.get_mut(&id) {
                    w.lease_id = None;
                }
                Ok(())
            })?;
            if let Err(e) = self.prepare_watch(&id).await {
                self.watch_error(&id, &e)?;
                return Err(e);
            }
        }
        for name in data
            .repos
            .keys()
            .filter(|r| repo.is_none_or(|x| x.eq_ignore_ascii_case(r)))
        {
            self.redeliver(name).await?;
        }
        self.publish_pending().await?;
        maintenance?;
        self.operation_response()
    }
    async fn redeliver(&self, repo: &str) -> Result<()> {
        let data = self.store()?.read()?;
        let Some(h) = data.repos.get(repo) else {
            return Ok(());
        };
        let Some(id) = h.hook_id else {
            return Ok(());
        };
        self.verify_owned(repo, h).await?;
        // Bounded recovery: oldest retained failures beyond 500 need manual action.
        for page in 1..=5 {
            let deliveries = self
                .api(
                    "GET",
                    &format!("repos/{repo}/hooks/{id}/deliveries?per_page=100&page={page}"),
                    None,
                )
                .await?;
            let rows = deliveries
                .as_array()
                .ok_or_else(|| Failure::Invalid("invalid delivery list".into()))?;
            for row in rows {
                let failed = row["status_code"]
                    .as_u64()
                    .is_none_or(|s| !(200..300).contains(&s));
                if failed {
                    if let Some(delivery_id) = row["id"].as_u64() {
                        self.api(
                            "POST",
                            &format!("repos/{repo}/hooks/{id}/deliveries/{delivery_id}/attempts"),
                            None,
                        )
                        .await?;
                    }
                }
            }
            if rows.len() < 100 {
                break;
            }
        }
        Ok(())
    }
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
