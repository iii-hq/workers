//! Explicit, isolated contract check for an S3-compatible test bucket.

use aws_credential_types::Credentials;
use aws_sdk_s3::{
    primitives::ByteStream,
    types::{
        ChecksumAlgorithm, ChecksumMode, ChecksumType, CompletedMultipartUpload, CompletedPart,
    },
    Client,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::FutureExt;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::panic::AssertUnwindSafe;

fn checksum(bytes: &[u8]) -> String {
    STANDARD.encode(Sha256::digest(bytes))
}

#[tokio::test]
#[ignore = "opt-in: requires an isolated S3 test bucket and explicit credentials"]
async fn multipart_contract() {
    let names = [
        "S3_TEST_ENDPOINT",
        "S3_TEST_BUCKET",
        "S3_TEST_REGION",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
    ];
    let missing: Vec<_> = names
        .iter()
        .filter(|name| std::env::var(name).map_or(true, |v| v.trim().is_empty()))
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "missing required environment variables: {}",
        missing.join(", ")
    );
    let bucket = std::env::var("S3_TEST_BUCKET").unwrap();
    let config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(aws_sdk_s3::config::Region::new(
            std::env::var("S3_TEST_REGION").unwrap(),
        ))
        .endpoint_url(std::env::var("S3_TEST_ENDPOINT").unwrap())
        .force_path_style(true)
        .credentials_provider(Credentials::new(
            std::env::var("AWS_ACCESS_KEY_ID").unwrap(),
            std::env::var("AWS_SECRET_ACCESS_KEY").unwrap(),
            std::env::var("AWS_SESSION_TOKEN").ok(),
            None,
            "multipart-contract",
        ))
        .build();
    let client = Client::from_conf(config);
    let mut random = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut random);
    random[6] = (random[6] & 0x0f) | 0x40;
    random[8] = (random[8] & 0x3f) | 0x80;
    let h = hex::encode(random);
    let token = format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    );
    let prefix = format!("qa/multipart/{token}/");
    let keys = [
        format!("{prefix}single"),
        format!("{prefix}double"),
        format!("{prefix}abort"),
    ];
    let mut uploads = Vec::<(String, String)>::new();
    let result = AssertUnwindSafe(async {
        for (index, key) in keys.iter().enumerate() {
            let created = client.create_multipart_upload().bucket(&bucket).key(key)
                .checksum_algorithm(ChecksumAlgorithm::Sha256).checksum_type(ChecksumType::Composite)
                .metadata("token", &token).send().await.expect("create multipart with SHA256/COMPOSITE");
            let id = created.upload_id().expect("upload ID").to_owned();
            uploads.push((key.clone(), id.clone()));
            if index == 2 {
                client.abort_multipart_upload().bucket(&bucket).key(key).upload_id(&id).send().await.expect("abort multipart");
                uploads.retain(|(_, active_id)| active_id != &id);
                let error = client.list_parts().bucket(&bucket).key(key).upload_id(&id).send().await.expect_err("aborted upload must not have parts");
                assert_eq!(error.raw_response().map(|response| response.status().as_u16()), Some(404));
                continue;
            }
            let first = vec![0x5a; if index == 0 { 32 } else { 16 * 1024 * 1024 }];
            let invalid = client.upload_part().bucket(&bucket).key(key).upload_id(&id).part_number(1)
                .checksum_sha256(checksum(b"wrong checksum")).body(ByteStream::from(first.clone())).send().await;
            let error = invalid.expect_err("provider accepted incorrect part checksum");
            assert_eq!(error.raw_response().map(|response| response.status().as_u16()), Some(400));
            let mut parts = Vec::new();
            for (number, data) in [first, vec![0x7a]].into_iter().enumerate().take(index + 1) {
                let sum = checksum(&data);
                let response = client.upload_part().bucket(&bucket).key(key).upload_id(&id).part_number((number + 1) as i32)
                    .checksum_sha256(&sum).body(ByteStream::from(data)).send().await.expect("upload part");
                parts.push(CompletedPart::builder().part_number((number + 1) as i32)
                    .e_tag(response.e_tag().expect("part ETag")).checksum_sha256(sum).build());
            }
            let listed = client.list_parts().bucket(&bucket).key(key).upload_id(&id).send().await.expect("list parts");
            assert_eq!(listed.parts().len(), index + 1);
            for (part, expected) in listed.parts().iter().zip(&parts) {
                assert_eq!(part.part_number(), expected.part_number());
                assert_eq!(part.e_tag(), expected.e_tag());
                assert_eq!(part.checksum_sha256(), expected.checksum_sha256());
            }
            if index == 1 {
                let swapped = vec![CompletedPart::builder().part_number(1).e_tag(parts[1].e_tag().unwrap()).checksum_sha256(parts[0].checksum_sha256().unwrap()).build(), parts[1].clone()];
                let error = client.complete_multipart_upload().bucket(&bucket).key(key).upload_id(&id)
                    .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(swapped)).build()).send().await.expect_err("provider accepted a swapped ETag");
                assert_eq!(error.raw_response().map(|response| response.status().as_u16()), Some(400));
            }
            client.complete_multipart_upload().bucket(&bucket).key(key).upload_id(&id)
                .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(parts)).build()).send().await.expect("complete multipart");
            uploads.retain(|(_, active_id)| active_id != &id);
            let head = client.head_object().bucket(&bucket).key(key).checksum_mode(ChecksumMode::Enabled).send().await.expect("head object checksum");
            assert_eq!(head.metadata().and_then(|m| m.get("token")), Some(&token));
            assert!(head.checksum_sha256().is_some());
            if index == 1 {
                assert_eq!(head.content_length(), Some(16 * 1024 * 1024 + 1));
                let range_response = client.get_object().bucket(&bucket).key(key).range("bytes=0-15").send().await.expect("range get");
                assert_eq!(range_response.content_length(), Some(16));
                assert_eq!(range_response.content_range(), Some("bytes 0-15/16777217"));
                let body = range_response.body.collect().await.expect("range body").into_bytes();
                assert_eq!(&body[..], &[0x5a; 16]);
            }
        }
        // Probe whether completion can prevent overwriting an existing key.
        let key = &keys[0];
        let created = client.create_multipart_upload().bucket(&bucket).key(key)
            .checksum_algorithm(ChecksumAlgorithm::Sha256).checksum_type(ChecksumType::Composite)
            .send().await.expect("create conditional probe");
        let id = created.upload_id().expect("conditional upload ID").to_owned();
        uploads.push((key.clone(), id.clone()));
        let data = b"conditional probe".to_vec();
        let sum = checksum(&data);
        let part = client.upload_part().bucket(&bucket).key(key).upload_id(&id).part_number(1)
            .checksum_sha256(&sum).body(ByteStream::from(data)).send().await.expect("upload conditional probe");
        let manifest = CompletedMultipartUpload::builder().parts(CompletedPart::builder().part_number(1)
            .e_tag(part.e_tag().expect("conditional ETag")).checksum_sha256(sum).build()).build();
        match client.complete_multipart_upload().bucket(&bucket).key(key).upload_id(&id)
            .if_none_match("*").multipart_upload(manifest).send().await {
            Ok(_) => {
                uploads.retain(|(_, active_id)| active_id != &id);
                eprintln!("conditional completion: existing object was overwritten; precondition not enforced");
            },
            Err(error) => match error.raw_response().map(|response| response.status().as_u16()) {
                Some(412) => eprintln!("conditional completion: existing object rejected with 412"),
                status => eprintln!("conditional completion: inconclusive or unsupported (HTTP {status:?})"),
            },
        }
    }).catch_unwind().await;
    let mut cleanup_failed = false;
    for (key, id) in &uploads {
        if client
            .abort_multipart_upload()
            .bucket(&bucket)
            .key(key)
            .upload_id(id)
            .send()
            .await
            .is_err()
        {
            cleanup_failed = true;
        }
    }
    for key in &keys {
        if client
            .delete_object()
            .bucket(&bucket)
            .key(key)
            .send()
            .await
            .is_err()
        {
            cleanup_failed = true;
        }
    }
    if cleanup_failed {
        eprintln!("failed to clean up one or more test resources");
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    assert!(!cleanup_failed, "failed to clean up test objects");
}

#[test]
fn part_checksum_is_sha256_base64() {
    assert_eq!(
        checksum(b"abc"),
        "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
    );
}
