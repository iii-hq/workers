use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunction, RegisterTriggerType};
use storage::backend::factory::{self, LocalBackendCtx};
use storage::backend::local;
use storage::config::{redact_url, BucketConfig as CfgBucket, WorkerConfig};
use storage::handlers::{
    delete_object::{self, DeleteReq},
    get_object::{self, GetReq},
    presign_url::{self, PresignReq},
    put_object::{self, PutReq},
    AppState,
};
use storage::rustfs::{health, spawn};
use storage::triggers::dispatcher::EngineDispatcher;
use storage::triggers::handler::{ObjectCreatedHandler, ObjectDeletedHandler};
use storage::triggers::pollers::{cf_queue, pubsub, rustfs_webhook, sqs};
use storage::triggers::registry::TriggerRegistry;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "storage", about = "storage worker")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    tracing::info!(
        name = storage::worker_name(),
        config = %cli.config,
        url = %redact_url(&cli.url),
        "starting"
    );

    let cfg = WorkerConfig::from_file(&cli.config)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("loading config from {}", cli.config))?;

    // Pre-compute which buckets have a notifications source. Trigger
    // registrations targeting any other bucket fail fast in the handler.
    let mut wired_buckets: HashSet<String> = HashSet::new();
    for (name, bc) in &cfg.buckets {
        let has_notifs = match bc {
            CfgBucket::S3(s) => s.notifications.is_some(),
            CfgBucket::Gcs(g) => g.notifications.is_some(),
            CfgBucket::R2(r) => r.notifications.is_some(),
            CfgBucket::Local(_) => true, // rustfs sidecar always wires the receiver
        };
        if has_notifs {
            wired_buckets.insert(name.clone());
        }
    }
    let wired_buckets = Arc::new(wired_buckets);

    let needs_local = cfg
        .buckets
        .values()
        .any(|b| matches!(b, CfgBucket::Local(_)));

    // Pre-allocate the webhook port BEFORE spawning rustfs so we can pass the
    // URL at spawn time. TOCTOU window between drop and bind is acceptable on
    // loopback in a single-tenant worker container.
    let webhook_port: Option<u16> = if needs_local {
        Some(spawn::allocate_port().map_err(|e| anyhow::anyhow!(e.to_wire_string()))?)
    } else {
        None
    };
    let webhook_addr: Option<SocketAddr> = webhook_port.map(|p| {
        SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, p))
    });
    let webhook_url: Option<String> =
        webhook_addr.as_ref().map(|a| format!("http://{a}/notify"));

    let mut rustfs_handle = None;
    let mut local_ctx: Option<LocalBackendCtx> = None;

    let mut local_data_dir: Option<String> = None;
    if needs_local {
        let data_dir = cfg
            .providers
            .local
            .as_ref()
            .map(|l| l.data_dir.clone())
            .unwrap_or_else(|| "./data/storage".to_string());
        local_data_dir = Some(data_dir.clone());
        let port = spawn::allocate_port().map_err(|e| anyhow::anyhow!(e.to_wire_string()))?;
        let (access_key, secret_key) = spawn::ephemeral_credentials();
        let handle = spawn::spawn(spawn::RustfsSpawnOpts {
            data_dir,
            access_key: access_key.clone(),
            secret_key: secret_key.clone(),
            port,
            notify_webhook_url: webhook_url.clone(),
        })
        .await
        .map_err(|e| anyhow::anyhow!(e.to_wire_string()))?;
        health::wait_for_healthy(port, Duration::from_secs(30))
            .await
            .map_err(|e| anyhow::anyhow!(e.to_wire_string()))?;
        // Create buckets up front; webhook subscription happens later, AFTER
        // the webhook receiver is bound, so the target can pass rustfs's
        // online probe during init.
        for (name, bucket_cfg) in &cfg.buckets {
            if let CfgBucket::Local(l) = bucket_cfg {
                let underlying = l.bucket.clone().unwrap_or_else(|| name.clone());
                local::ensure_bucket(port, &access_key, &secret_key, &underlying)
                    .await
                    .map_err(|e| anyhow::anyhow!(format!("{e}")))?;
            }
        }
        local_ctx = Some(LocalBackendCtx {
            port,
            access_key_id: access_key,
            secret_access_key: secret_key,
        });
        rustfs_handle = Some(handle);
        tracing::info!(port, "rustfs sidecar ready");
    }

    let mut backends = HashMap::new();
    for (name, bucket_cfg) in &cfg.buckets {
        let b = factory::build(name, bucket_cfg, &cfg.providers, local_ctx.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!(format!("{e}")))
            .with_context(|| format!("building backend `{name}`"))?;
        tracing::info!(bucket = %name, provider = b.provider(), "backend ready");
        backends.insert(name.clone(), b);
    }

    let state = AppState {
        backends: Arc::new(backends),
    };

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let registry = Arc::new(TriggerRegistry::new());
    let dispatcher = Arc::new(EngineDispatcher::new(iii.clone(), registry.clone()));

    // Bind the rustfs notify webhook receiver on the port we pre-allocated.
    let mut webhook_handle: Option<rustfs_webhook::WebhookHandle> = None;
    if needs_local {
        let port = webhook_port.expect("pre-allocated when needs_local");
        let mut local_bucket_reverse: HashMap<String, String> = HashMap::new();
        for (name, bc) in &cfg.buckets {
            if let CfgBucket::Local(l) = bc {
                let underlying = l.bucket.clone().unwrap_or_else(|| name.clone());
                local_bucket_reverse.insert(underlying, name.clone());
            }
        }
        let state = rustfs_webhook::WebhookState {
            bucket_reverse: Arc::new(local_bucket_reverse),
            dispatch_timeout: Duration::from_secs(60),
            dispatch: dispatcher.clone(),
        };
        let handle = rustfs_webhook::spawn_receiver_on(state, port)
            .await
            .map_err(|e| anyhow::anyhow!(format!("rustfs webhook receiver: {e}")))?;
        tracing::info!(addr = %handle.addr, "rustfs notify webhook receiver bound");
        webhook_handle = Some(handle);

        // Now that the receiver is listening on the webhook port, register the
        // webhook target with rustfs and subscribe each local bucket to it.
        // This must happen AFTER the receiver bind: rustfs's notify subsystem
        // probes the webhook URL during target init() — if the URL isn't
        // reachable, the target is silently dropped from the live target_list,
        // and put_bucket_notification_configuration then fails with
        // "Configuration error: No targets configured".
        if let (Some(url), Some(ctx), Some(data_dir)) =
            (&webhook_url, local_ctx.as_ref(), local_data_dir.as_ref())
        {
            // rustfs validates queue_dir as an absolute path. The configured
            // data_dir may be relative (e.g. `./data/storage` from
            // config.yaml), so canonicalize before deriving the queue dir.
            let queue_dir_rel = std::path::Path::new(data_dir).join(".webhook-queue");
            let _ = std::fs::create_dir_all(&queue_dir_rel);
            let queue_dir = std::fs::canonicalize(&queue_dir_rel).unwrap_or(queue_dir_rel);
            local::register_webhook_target(
                ctx.port,
                &ctx.access_key_id,
                &ctx.secret_access_key,
                url,
                &queue_dir,
            )
            .await
            .map_err(|e| anyhow::anyhow!(format!("{e}")))?;
            tracing::info!("rustfs webhook target registered");

            for (name, bucket_cfg) in &cfg.buckets {
                if let CfgBucket::Local(l) = bucket_cfg {
                    let underlying = l.bucket.clone().unwrap_or_else(|| name.clone());
                    local::subscribe_bucket_notifications(
                        ctx.port,
                        &ctx.access_key_id,
                        &ctx.secret_access_key,
                        &underlying,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!(format!("{e}")))?;
                    tracing::info!(
                        bucket = %name,
                        underlying = %underlying,
                        "rustfs bucket subscribed to webhook events"
                    );
                }
            }
        }
    }

    // Group bucket notifications by upstream source so we don't double-poll.
    let mut sqs_queues: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut pubsub_subs: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut cf_queues: HashMap<(String, String, String), HashMap<String, String>> = HashMap::new();
    let mut sqs_regions: HashMap<String, String> = HashMap::new();

    for (name, bc) in &cfg.buckets {
        match bc {
            CfgBucket::S3(s) => {
                if let Some(notif) = &s.notifications {
                    let underlying = s.bucket.clone().unwrap_or_else(|| name.clone());
                    sqs_queues
                        .entry(notif.sqs_queue_url.clone())
                        .or_default()
                        .insert(underlying, name.clone());
                    sqs_regions
                        .entry(notif.sqs_queue_url.clone())
                        .or_insert_with(|| s.region.clone());
                }
            }
            CfgBucket::Gcs(g) => {
                if let Some(notif) = &g.notifications {
                    let underlying = g.bucket.clone().unwrap_or_else(|| name.clone());
                    pubsub_subs
                        .entry(notif.pubsub_subscription.clone())
                        .or_default()
                        .insert(underlying, name.clone());
                }
            }
            CfgBucket::R2(r) => {
                if let Some(notif) = &r.notifications {
                    let underlying = r.bucket.clone().unwrap_or_else(|| name.clone());
                    cf_queues
                        .entry((
                            r.account_id.clone(),
                            notif.queue_id.clone(),
                            notif.api_token.clone(),
                        ))
                        .or_default()
                        .insert(underlying, name.clone());
                }
            }
            CfgBucket::Local(_) => {}
        }
    }

    for (queue_url, reverse) in sqs_queues {
        let region = sqs_regions
            .get(&queue_url)
            .cloned()
            .unwrap_or_else(|| "us-east-1".to_string());
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region))
            .load()
            .await;
        let client = aws_sdk_sqs::Client::new(&sdk_config);
        let poller = sqs::SqsPoller::new(client, queue_url.clone(), Arc::new(reverse));
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            sqs::run_loop(poller, dispatcher).await;
        });
        tracing::info!(queue_url = %queue_url, "sqs poller started");
    }

    for (subscription, reverse) in pubsub_subs {
        let pubsub_config = match gcloud_pubsub::client::ClientConfig::default()
            .with_auth()
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    subscription = %subscription,
                    error = %e,
                    "pubsub auth failed; skipping subscription"
                );
                continue;
            }
        };
        let pubsub_client = match gcloud_pubsub::client::Client::new(pubsub_config).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    subscription = %subscription,
                    error = %e,
                    "pubsub client init failed; skipping subscription"
                );
                continue;
            }
        };
        let poller = pubsub::PubsubPoller::new(subscription.clone(), Arc::new(reverse));
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            pubsub::run_loop(poller, pubsub_client, dispatcher).await;
        });
        tracing::info!(subscription = %subscription, "pubsub poller started");
    }

    for ((account_id, queue_id, api_token), reverse) in cf_queues {
        if let Err(e) = cf_queue::probe_auth(&account_id, &queue_id, &api_token).await {
            tracing::error!(
                queue_id = %queue_id,
                error = %e.to_wire_string(),
                "cf-queue auth probe failed; skipping subscription (will not deliver)"
            );
            continue;
        }
        let poller = cf_queue::CfQueuePoller::new(
            account_id,
            queue_id.clone(),
            api_token,
            Arc::new(reverse),
        );
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            cf_queue::run_loop(poller, dispatcher).await;
        });
        tracing::info!(queue_id = %queue_id, "cf-queue poller started");
    }

    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("storage::putObject", move |req: PutReq| {
                let st = st.clone();
                async move { put_object::handle(&st, req).await }
            })
            .description(
                "Write an object to a configured bucket. Body is base64; max 10MB inline.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("storage::getObject", move |req: GetReq| {
                let st = st.clone();
                async move { get_object::handle(&st, req).await }
            })
            .description("Read an object. Body is base64; for large objects use presignUrl."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("storage::deleteObject", move |req: DeleteReq| {
                let st = st.clone();
                async move { delete_object::handle(&st, req).await }
            })
            .description("Delete an object. No-op when the object does not exist."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            RegisterFunction::new_async("storage::presignUrl", move |req: PresignReq| {
                let st = st.clone();
                async move { presign_url::handle(&st, req).await }
            })
            .description(
                "Issue a short-lived URL the browser can hit directly to PUT or GET an object.",
            ),
        );
    }

    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        "storage::object-created",
        "Fires when an object is written to a configured bucket whose notifications source is wired up.",
        ObjectCreatedHandler {
            registry: registry.clone(),
            wired_buckets: wired_buckets.clone(),
        },
    ));
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        "storage::object-deleted",
        "Fires when an object is removed from a configured bucket whose notifications source is wired up.",
        ObjectDeletedHandler {
            registry: registry.clone(),
            wired_buckets: wired_buckets.clone(),
        },
    ));

    tracing::info!(
        "storage registered 4 functions and 2 trigger types, waiting for invocations"
    );
    wait_for_shutdown_signal().await?;
    tracing::info!("storage shutting down");
    iii.shutdown_async().await;
    if let Some(h) = webhook_handle {
        let _ = h.shutdown_tx.send(());
    }
    if let Some(h) = rustfs_handle {
        spawn::shutdown(h).await;
    }
    Ok(())
}

async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
