//! The frozen prompt against SemIf's own `apply_chat_template` output
//! (`tests/fixtures/prompts.json`, from `make_prompts.py`).
use judge_semif::prompt::{render, state_prefix};
use serde_json::Value;

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/prompts.json")).unwrap()
}

#[test]
fn prompts_and_state_prefixes_match_semif() {
    for case in fixtures() {
        let options: Vec<String> = serde_json::from_value(case["options"].clone()).unwrap();
        let prompt = render(&case["state"], case["question"].as_str().unwrap(), &options);
        assert_eq!(prompt, case["prompt"].as_str().unwrap());
        // SemIf drops the prefix's last token; its decoded text must still
        // lead our prefix, and our prefix must lead every prompt.
        let prefix = state_prefix(&case["state"]);
        assert!(prefix.starts_with(case["prefix_tokens_text"].as_str().unwrap()));
        assert!(prompt.starts_with(&prefix));
    }
}

/// Token ids from the Qwen3.5 GGUF vocabulary equal SemIf's reference
/// tokenizer. Needs the real model: `SEMIF_GGUF=/path/Qwen_Qwen3.5-4B-Q4_K_M.gguf`.
#[test]
#[ignore]
fn gguf_tokens_match_the_reference_tokenizer() {
    use llama_cpp_2::{
        llama_backend::LlamaBackend,
        model::{params::LlamaModelParams, AddBos, LlamaModel},
    };
    let gguf = std::env::var("SEMIF_GGUF").expect("SEMIF_GGUF points at the Qwen3.5-4B GGUF");
    let backend = LlamaBackend::init().unwrap();
    let model = LlamaModel::load_from_file(&backend, gguf, &LlamaModelParams::default()).unwrap();
    for case in fixtures() {
        let ids: Vec<i32> = model
            .str_to_token(case["prompt"].as_str().unwrap(), AddBos::Never)
            .unwrap()
            .into_iter()
            .map(|t| t.0)
            .collect();
        assert_eq!(
            ids,
            serde_json::from_value::<Vec<i32>>(case["token_ids"].clone()).unwrap()
        );
    }
}
