//! Encoding parity with Clef's own `encode_record`, from committed fixtures
//! (`tests/fixtures/encode_records.json`, written by `make_encode_fixture.py`):
//! the rendered pieces, then spans and truncation with UTF-8 bytes as token
//! ids. The release tokenizer is `#[ignore]`d: it needs `tokenizer.json` in
//! `$CLEF_CHECKPOINT_DIR`.
use judge_clef::encode::{encode, options, pieces, Encoded, PREFIX};
use judge_contract::{ErrorCode, Evaluation};
use serde::Deserialize;
use std::{ops::Range, path::PathBuf};

/// `[kind, span, option spans]`, as the fixture writes a field.
type Field = (u32, (usize, usize), Vec<(usize, usize)>);
type Flat = (Vec<u32>, Vec<Field>, usize);

#[derive(Deserialize)]
struct Record {
    evaluation: Evaluation,
    pieces: Vec<String>,
    option_ids: Vec<Vec<String>>,
    tokens: Flat,
    bytes: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    max_length: usize,
    expected: Result<Flat, ErrorCode>,
}

fn records() -> Vec<Record> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/encode_records.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn flat(encoded: Encoded) -> Flat {
    let pair = |range: &Range<usize>| (range.start, range.end);
    let fields = encoded.fields.iter();
    let fields = fields.map(|f| (f.kind, pair(&f.span), f.options.iter().map(pair).collect()));
    (encoded.ids, fields.collect(), encoded.dropped)
}

fn bytes(text: &str) -> Result<Vec<u32>, ErrorCode> {
    Ok(text.bytes().map(u32::from).collect())
}

#[test]
fn pieces_match_the_reference() {
    let records = records();
    assert!(records.len() >= 6);
    for record in records {
        let id = &record.evaluation.id;
        let (state, rest) = pieces(&record.evaluation);
        let ours: Vec<&str> = [PREFIX, &state]
            .into_iter()
            .chain(rest.iter().map(|(_, text)| text.as_str()))
            .collect();
        assert_eq!(ours, record.pieces, "{id}: pieces");
        let questions = record.evaluation.questions.values();
        let option_ids: Vec<Vec<String>> = questions
            .map(|q| options(q).1.into_iter().map(|(id, _)| id).collect())
            .collect();
        assert_eq!(option_ids, record.option_ids, "{id}: option order");
    }
}

#[test]
fn spans_and_truncation_match_the_reference_on_bytes() {
    let (mut truncated, mut rejected) = (0, 0);
    for record in records() {
        for case in record.bytes {
            let ours = encode(&record.evaluation, case.max_length, bytes).map(flat);
            let id = &record.evaluation.id;
            assert_eq!(ours, case.expected, "{id}: max_length {}", case.max_length);
            match case.expected {
                Ok((_, _, dropped)) => truncated += usize::from(dropped > 0),
                Err(_) => rejected += 1,
            }
        }
    }
    assert!(truncated >= 2 && rejected >= 1);
}

#[test]
fn an_empty_question_span_is_rejected() {
    let evaluation: Evaluation = serde_json::from_value(serde_json::json!({
        "id": "empty", "state": "s", "questions": {"": {"type": "noul", "instructions": ""}}
    }))
    .unwrap();
    assert_eq!(
        encode(&evaluation, 16384, bytes),
        Err(ErrorCode::InvalidRequest)
    );
}

#[test]
#[ignore = "needs the release tokenizer.json in $CLEF_CHECKPOINT_DIR"]
fn tokenizer_matches_the_reference() {
    let dir = std::env::var("CLEF_CHECKPOINT_DIR").expect("CLEF_CHECKPOINT_DIR");
    let tokenizer =
        tokenizers::Tokenizer::from_file(PathBuf::from(dir).join("tokenizer.json")).unwrap();
    let tokenize = |t: &str| {
        tokenizer
            .encode_fast(t, false)
            .map(|e| e.get_ids().to_vec())
            .map_err(|_| ErrorCode::InvalidRequest)
    };
    for record in records() {
        let ours = encode(&record.evaluation, 16384, tokenize).map(flat);
        assert_eq!(ours, Ok(record.tokens), "{}", record.evaluation.id);
    }
}
