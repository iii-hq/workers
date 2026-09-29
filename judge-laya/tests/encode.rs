//! Tokenization parity with laya's Python `build_sequence`, from committed
//! fixtures (`tests/fixtures/encode.json`, generated once with the real
//! tokenizer, which the tiny fixture shares).
use judge_laya::encode::{render_options, Encoder, QType, Question};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Case {
    name: String,
    state: Value,
    #[serde(rename = "type")]
    qtype: String,
    instructions: String,
    criteria: Option<Value>,
    options: Vec<String>,
    ids: Vec<u32>,
    markers: Vec<usize>,
}

#[test]
fn sequences_match_the_python_reference_token_for_token() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let cases: Vec<Case> =
        serde_json::from_slice(&std::fs::read(fixtures.join("encode.json")).unwrap()).unwrap();
    let tokenizer = tokenizers::Tokenizer::from_file(fixtures.join("tiny/tokenizer.json")).unwrap();
    let encoder = Encoder::new(tokenizer, 512, 192).unwrap();
    assert!(cases.len() >= 8);
    for case in cases {
        let qtype = match case.qtype.as_str() {
            "choice" => QType::Choice,
            "score" => QType::Score,
            _ => QType::Noul,
        };
        let options = render_options(qtype, case.criteria.as_ref()).unwrap();
        assert_eq!(options, case.options, "{}: rendered options", case.name);
        let sequence = encoder
            .build(
                &case.state,
                &Question {
                    qtype,
                    instructions: case.instructions.clone(),
                    options,
                },
            )
            .unwrap();
        assert_eq!(sequence.markers, case.markers, "{}: markers", case.name);
        assert_eq!(sequence.ids, case.ids, "{}: token ids", case.name);
    }
}
