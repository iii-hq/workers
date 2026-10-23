//! The frozen prompt against SemIf's own `apply_chat_template` output
//! (`tests/fixtures/prompts.json`, from `make_prompts.py`).
use judge_semif::prompt::render;
use serde_json::Value;

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/prompts.json")).unwrap()
}

#[test]
fn prompts_match_semif() {
    for case in fixtures() {
        let options: Vec<String> = serde_json::from_value(case["options"].clone()).unwrap();
        let prompt = render(&case["state"], case["question"].as_str().unwrap(), &options);
        assert_eq!(prompt, case["prompt"].as_str().unwrap());
    }
}

/// Token ids from the Qwen3.5 GGUF vocabulary equal SemIf's reference
/// tokenizer. Needs the real model: `SEMIF_GGUF=/path/Qwen_Qwen3.5-4B-Q4_K_M.gguf`.
#[test]
#[ignore]
fn gguf_tokens_match_the_reference_tokenizer() {
    use iii_llama_runtime::iii_llama_native::Model;
    let gguf = std::env::var("SEMIF_GGUF").expect("SEMIF_GGUF points at the Qwen3.5-4B GGUF");
    let model = Model::load(gguf.as_ref(), Some(0), 1, false).unwrap();
    for case in fixtures() {
        let ids = model
            .tokenize(case["prompt"].as_str().unwrap(), false)
            .unwrap();
        assert_eq!(
            ids,
            serde_json::from_value::<Vec<i32>>(case["token_ids"].clone()).unwrap()
        );
    }
}
