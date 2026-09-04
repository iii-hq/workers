//! CI proof for the MiniLM release targets. Cargo.toml's target cfg decides
//! whether ort/fastembed are dependencies; build.rs' `MINILM_TARGETS` decides
//! whether the dense lane compiles. The two are meant to agree, and a target
//! on which they drift builds green as BM25-only. With `III_REQUIRE_MINILM`
//! set, this fails instead — and linking this binary is the static ONNX
//! Runtime proof for the host target.

#[test]
// Constant per target by design: that constant is what this test proves.
#[allow(clippy::assertions_on_constants)]
fn minilm_compiled_in_when_required() {
    if std::env::var_os("III_REQUIRE_MINILM").is_some() {
        assert!(
            cfg!(minilm),
            "{} is expected to carry the MiniLM dense lane but built BM25-only",
            env!("TARGET")
        );
    }
}
