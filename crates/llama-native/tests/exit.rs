//! A process that exits with a model and a context still loaded, on the GPU
//! when there is one, exits cleanly: a judge worker stopped with its model in
//! its slot, or a test binary holding its model in a static. On Metal those
//! buffers hold residency sets when the process exits.
use iii_llama_native::{ContextParams, Model};
use std::path::Path;
use std::process::Command;

const CHILD: &str = "III_LLAMA_NATIVE_EXIT_CHILD";
const TEST: &str = "a_process_exits_cleanly_with_its_model_still_loaded";

#[test]
fn a_process_exits_cleanly_with_its_model_still_loaded() {
    if std::env::var_os(CHILD).is_some() {
        let gguf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../judge-decider/tests/fixtures/tiny-qwen3.gguf");
        // Every layer on the GPU when there is one, as the providers load.
        let model: &'static Model = Box::leak(Box::new(
            Model::load(&gguf, None, 2, true).expect("tiny GGUF loads"),
        ));
        let mut ctx = model
            .new_context(ContextParams {
                n_ctx: 64,
                n_batch: 16,
                n_ubatch: 16,
                n_seq_max: 1,
                threads: 2,
                ..Default::default()
            })
            .expect("context");
        ctx.clear_kv_cache();
        // Neither is freed: the process exits with both allocated.
        std::mem::forget(ctx);
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([TEST, "--exact", "--test-threads=1"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "the child ran the test: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
