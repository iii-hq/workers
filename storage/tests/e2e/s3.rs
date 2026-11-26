//! Real AWS S3 round-trip. Gated on AWS_ACCESS_KEY_ID + S3_TEST_BUCKET.

use std::collections::HashMap;
use storage::backend::{GetReq, PutReq, factory};
use storage::config::{BucketConfig, ProvidersConfig, S3BucketConfig};

fn require_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("e2e:s3 — skip: {name} not set");
            None
        }
    }
}

#[tokio::test]
#[ignore = "opt-in: set AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, S3_TEST_BUCKET, S3_TEST_REGION"]
async fn s3_round_trip() {
    let Some(bucket) = require_env("S3_TEST_BUCKET") else {
        return;
    };
    // Without explicit credentials the SDK falls back to ADC, IMDS, etc. and
    // produces opaque "credential provider not found" errors deep in the
    // call. Skip up front with a clear reason instead.
    if require_env("AWS_ACCESS_KEY_ID").is_none() {
        return;
    }
    if require_env("AWS_SECRET_ACCESS_KEY").is_none() {
        return;
    }
    let region = std::env::var("S3_TEST_REGION").unwrap_or_else(|_| "us-east-1".into());
    let cfg = BucketConfig::S3(S3BucketConfig {
        bucket: Some(bucket.clone()),
        region,
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
        endpoint_url: None,
        force_path_style: None,
        notifications: None,
        display: None,
    });
    let material = storage::material::Material::literal(&cfg).expect("literal credentials");
    let backend = factory::build(
        "e2e",
        &cfg,
        &ProvidersConfig::default(),
        None,
        &material,
        factory::Purpose::Live,
    )
    .await
    .expect("backend build");
    let key = format!(
        "e2e/storage/{}.bin",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let body = b"hello-s3".to_vec();
    backend
        .put(PutReq {
            key: key.clone(),
            body: body.clone(),
            content_type: "application/octet-stream".into(),
            cache_control: None,
            metadata: HashMap::new(),
        })
        .await
        .expect("put");
    let got = backend
        .get(GetReq {
            key: key.clone(),
            ..Default::default()
        })
        .await
        .expect("get");
    assert_eq!(got.body, body);
    backend
        .delete(storage::backend::DeleteReq {
            key,
            version_id: None,
        })
        .await
        .expect("delete");
}

/// `storage::copyObject`'s server-side path between two buckets of one
/// S3-compatible account (R2 uses the same code). Opt-in against any S3
/// endpoint, e.g. a throwaway MinIO:
/// `S3_COPY_TEST_ENDPOINT=http://127.0.0.1:9000 AWS_ACCESS_KEY_ID=… AWS_SECRET_ACCESS_KEY=…`.
#[tokio::test]
#[ignore = "opt-in: set S3_COPY_TEST_ENDPOINT, AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY"]
async fn s3_copy_between_buckets_server_side() {
    use storage::backend::{CopyReq, HeadReq};
    let Some(endpoint) = require_env("S3_COPY_TEST_ENDPOINT") else {
        return;
    };
    let (Some(access_key_id), Some(secret_access_key)) = (
        require_env("AWS_ACCESS_KEY_ID"),
        require_env("AWS_SECRET_ACCESS_KEY"),
    ) else {
        return;
    };
    let suffix = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let build = |bucket: String| {
        let cfg = BucketConfig::S3(S3BucketConfig {
            bucket: Some(bucket),
            region: "us-east-1".into(),
            access_key_id: Some(access_key_id.clone()),
            secret_access_key: Some(secret_access_key.clone()),
            session_token: None,
            endpoint_url: Some(endpoint.clone()),
            force_path_style: Some(true),
            notifications: None,
            display: None,
        });
        async move {
            let material = storage::material::Material::literal(&cfg).expect("literal credentials");
            let backend = factory::build(
                "e2e",
                &cfg,
                &ProvidersConfig::default(),
                None,
                &material,
                factory::Purpose::Live,
            )
            .await
            .expect("backend build");
            backend.ensure_bucket().await.expect("bucket");
            backend
        }
    };
    let staging = build(format!("e2e-copy-staging-{suffix}")).await;
    let cdn = build(format!("e2e-copy-cdn-{suffix}")).await;
    let body: Vec<u8> = (0..1024 * 1024 + 3).map(|i| (i % 253) as u8).collect();
    let source_key = "staging/1/iii app (1).dmg";
    staging
        .put(PutReq {
            key: source_key.into(),
            body: body.clone(),
            content_type: "application/octet-stream".into(),
            cache_control: None,
            metadata: HashMap::from([("sha".to_string(), "1".to_string())]),
        })
        .await
        .expect("put");

    let copied = cdn
        .copy_from(
            staging.as_ref(),
            &CopyReq {
                source_key: source_key.into(),
                key: "workers/app/1/app.dmg".into(),
                content_type: Some("application/x-apple-diskimage".into()),
                cache_control: Some("public, max-age=31536000, immutable".into()),
                metadata: None,
            },
        )
        .await
        .expect("copy")
        .expect("same endpoint and key copy server-side");
    assert_eq!(copied.size, body.len() as u64);
    let got = cdn
        .get(GetReq {
            key: "workers/app/1/app.dmg".into(),
            ..Default::default()
        })
        .await
        .expect("get the copy");
    assert_eq!(got.body, body);
    assert_eq!(got.content_type, "application/x-apple-diskimage");
    let head = cdn
        .head(HeadReq {
            key: "workers/app/1/app.dmg".into(),
            ..Default::default()
        })
        .await
        .expect("head the copy");
    assert_eq!(
        head.metadata
            .unwrap_or_default()
            .get("sha")
            .map(String::as_str),
        Some("1")
    );

    let kept = cdn
        .copy_from(
            staging.as_ref(),
            &CopyReq {
                source_key: source_key.into(),
                key: "kept.dmg".into(),
                ..Default::default()
            },
        )
        .await
        .expect("copy keeping headers")
        .expect("server-side");
    assert_eq!(kept.size, body.len() as u64);
    let head = cdn
        .head(HeadReq {
            key: "kept.dmg".into(),
            ..Default::default()
        })
        .await
        .expect("head the kept copy");
    assert_eq!(head.content_type, "application/octet-stream");

    let missing = cdn
        .copy_from(
            staging.as_ref(),
            &CopyReq {
                source_key: "gone".into(),
                key: "never".into(),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(
        missing,
        Err(storage::backend::BackendError::NotFound)
    ));
}
