//! Native local-backend round-trip with no external process or service.

use std::collections::HashMap;
use std::sync::Arc;
use storage::backend::{GetReq, ListReq, PresignMethod, PresignReq, PutReq};
use storage::config::{LocalHttpConfig, LocalProviderConfig};
use storage::triggers::dispatcher::EventDispatcher;
use storage::triggers::normalize::ObjectEventNormalized;

struct Ack;

#[async_trait::async_trait]
impl EventDispatcher for Ack {
    async fn dispatch(&self, _: ObjectEventNormalized) -> bool {
        true
    }
}

#[tokio::test]
async fn native_local_round_trip_and_signed_download() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = LocalProviderConfig {
        data_dir: temp.path().to_string_lossy().to_string(),
        http: Some(LocalHttpConfig {
            bind_address: "127.0.0.1:0".into(),
            public_url: None,
        }),
    };
    let mut prepared = storage::backend::local::prepare(Some(&config), Arc::new(Ack))
        .await
        .expect("prepare native local store");
    let backend =
        storage::backend::local::build(&prepared.context, "scratch".into(), "scratch".into())
            .expect("build local bucket");
    let http = storage::backend::local::start_http(&mut prepared).expect("HTTP enabled");

    let key = "e2e/local/test.bin";
    let body = b"hello-local".to_vec();
    backend
        .put(PutReq {
            key: key.into(),
            body: body.clone(),
            content_type: "application/octet-stream".into(),
            cache_control: None,
            metadata: HashMap::new(),
        })
        .await
        .expect("put");
    let got = backend
        .get(GetReq {
            key: key.into(),
            ..Default::default()
        })
        .await
        .expect("get");
    assert_eq!(got.body, body);

    let listing = backend
        .list(ListReq {
            prefix: "e2e/".into(),
            delimiter: Some("/".into()),
            cursor: None,
            limit: 100,
        })
        .await
        .expect("list");
    assert_eq!(listing.common_prefixes, vec!["e2e/local/"]);

    let signed = backend
        .presign(PresignReq {
            key: key.into(),
            method: PresignMethod::Get,
            content_type: None,
            expires_in_seconds: 60,
            response_content_disposition: None,
            response_content_type: None,
        })
        .await
        .expect("presign GET");
    let downloaded = reqwest::get(signed.url)
        .await
        .expect("download request")
        .bytes()
        .await
        .expect("download body");
    assert_eq!(&downloaded[..], &body);

    backend
        .delete(storage::backend::DeleteReq {
            key: key.into(),
            version_id: None,
        })
        .await
        .expect("delete");
    http.shutdown();
}
