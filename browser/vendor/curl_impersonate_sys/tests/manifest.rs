use curl_impersonate_sys::{artifact_for_target, CURL_CFFI_VERSION, CURL_IMPERSONATE_VERSION};

#[test]
fn frozen_engine_and_tier_one_artifacts_are_exact() {
    assert_eq!(CURL_CFFI_VERSION, "0.16.0");
    assert_eq!(CURL_IMPERSONATE_VERSION, "2.0.0");

    let x86 = artifact_for_target("x86_64-unknown-linux-gnu").unwrap();
    assert_eq!(x86.bytes, 26_572_973);
    assert_eq!(
        x86.sha256,
        "d8a98bc123fae4f04bb6a7584ff486333a334b3b08edaba1867929ae8d6ebb4d"
    );

    let arm = artifact_for_target("aarch64-unknown-linux-gnu").unwrap();
    assert_eq!(arm.bytes, 25_595_976);
    assert_eq!(
        arm.sha256,
        "12708019a6c1c3a7a7a40a8a379d12aaca127fcd13bda60b06fdb0e013f6433f"
    );

    assert!(artifact_for_target("x86_64-unknown-linux-musl").is_none());
}

#[test]
fn ffi_constants_match_the_frozen_header() {
    use curl_impersonate_sys::*;

    assert_eq!(CURLOPT_URL, 10_002);
    assert_eq!(CURLOPT_WRITEFUNCTION, 20_011);
    assert_eq!(CURLOPT_FOLLOWLOCATION, 52);
    assert_eq!(CURLINFO_RESPONSE_CODE, 0x20_0002);
    assert_eq!(CURL_GLOBAL_DEFAULT, 3);
}
