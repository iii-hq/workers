use iii_python_core::artifact;

#[test]
fn embedded_zip_matches_pinned_size_and_digest_constant() {
    // sha2 is a build-dependency only and unavailable here. build.rs
    // verified the digest and wrote it to OUT_DIR/python-wasi.sha256, which
    // ZIP_SHA256 embeds via include_str!; asserting it against the pinned
    // literal proves the build-time verification ran and produced the
    // bytes we embedded.
    assert_eq!(artifact::PYTHON_WASI_ZIP.len(), 14_291_017);
    assert_eq!(
        artifact::ZIP_SHA256,
        "2e064d3fb8172471d39d741348efa722349c40b96301f69968dff714999c584b"
    );
}

#[test]
fn extraction_is_idempotent_and_yields_interpreter_and_stdlib() {
    let root = artifact::ensure_extracted().expect("first extraction");
    assert!(root.join("python.wasm").is_file());
    assert!(root.join("lib/python3.14/os.py").is_file());
    let again = artifact::ensure_extracted().expect("second call");
    assert_eq!(root, again);
}

#[test]
fn module_compiles_and_second_load_hits_cwasm_cache() {
    let root = artifact::ensure_extracted().unwrap();
    let engine = artifact::sandbox_engine().unwrap();
    let _m = artifact::load_module(&engine, &root).expect("cold compile");
    let cwasm = root.join(format!(
        "python-wasmtime-{}.cwasm",
        artifact::WASMTIME_VERSION
    ));
    assert!(
        cwasm.is_file(),
        "compile must persist a cwasm next to the artifact"
    );
    let _m2 = artifact::load_module(&engine, &root).expect("warm load");
}
