//! The candle head against Cloudflare's own `JointSchemaHead` run in float64
//! on the tiny fixture (`make_fixtures.py`), and the lexicon's row reads.
use candle_core::{
    quantized::{gguf_file, GgmlDType, QTensor},
    Device, Tensor,
};
use judge_clef::{
    encode::{Encoded, Field},
    head::{option_ids, Head, Lexicon},
};
use serde::Deserialize;
use std::{fs::File, path::PathBuf};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn tiny_head() -> Head {
    Head::load(
        &fixture("tiny/joint_head.safetensors"),
        &fixture("tiny/joint_head_config.json"),
        32,
    )
    .unwrap()
}

#[derive(Deserialize)]
struct Case {
    name: String,
    ids: Vec<u32>,
    hidden: Vec<f32>,
    fields: Vec<CaseField>,
    logits: Vec<Vec<f64>>,
}

#[derive(Deserialize)]
struct CaseField {
    kind: u32,
    span: [usize; 2],
    options: Vec<[usize; 2]>,
}

fn cases() -> Vec<(Case, Encoded)> {
    let cases: Vec<Case> =
        serde_json::from_slice(&std::fs::read(fixture("head_cases.json")).unwrap()).unwrap();
    cases
        .into_iter()
        .map(|c| {
            let fields = c
                .fields
                .iter()
                .map(|f| Field {
                    kind: f.kind,
                    span: f.span[0]..f.span[1],
                    options: f.options.iter().map(|o| o[0]..o[1]).collect(),
                })
                .collect();
            let encoded = Encoded {
                ids: c.ids.clone(),
                fields,
                dropped: 0,
            };
            (c, encoded)
        })
        .collect()
}

#[test]
fn head_matches_the_reference() {
    let head = tiny_head();
    let lexicon = Lexicon::open(&fixture("tiny/backbone.gguf")).unwrap();
    for (case, encoded) in cases() {
        let logits = head.logits(&lexicon, case.hidden, &encoded).unwrap();
        assert_eq!(logits.len(), case.logits.len(), "{}", case.name);
        for (ours, theirs) in logits.iter().zip(&case.logits) {
            assert_eq!(ours.len(), theirs.len(), "{}", case.name);
            for (a, b) in ours.iter().zip(theirs) {
                assert!(
                    (f64::from(*a) - b).abs() < 1e-4,
                    "{}: {a} vs {b}",
                    case.name
                );
            }
        }
    }
}

#[test]
fn head_refuses_what_the_reference_cannot_score() {
    assert!(Head::load(
        &fixture("tiny/joint_head.safetensors"),
        &fixture("tiny/joint_head_config.json"),
        64,
    )
    .is_err());
    let head = tiny_head();
    let lexicon = Lexicon::open(&fixture("tiny/backbone.gguf")).unwrap();
    let (case, encoded) = cases().remove(0);
    let hidden =
        || Tensor::from_vec(case.hidden.clone(), (case.ids.len(), 32), &Device::Cpu).unwrap();
    let lexical = lexicon
        .rows(&option_ids(&encoded.ids, &encoded.fields))
        .unwrap();

    let mut empty = encoded.fields.clone();
    empty[1].span = 20..20;
    assert!(head.forward(hidden(), &empty, &lexical).is_err());
    let short = lexical.narrow(0, 1, lexical.dim(0).unwrap() - 1).unwrap();
    assert!(head.forward(hidden(), &encoded.fields, &short).is_err());
}

#[test]
fn lexicon_reads_output_rows_of_a_gguf() {
    let device = Device::Cpu;
    let quantize = |scale: f32| {
        let values = Tensor::rand(-scale, scale, (8, 512), &device).unwrap();
        QTensor::quantize(&values, GgmlDType::Q6K).unwrap()
    };
    let (embd, output) = (quantize(1.0), quantize(2.0));
    let mut file = tempfile::NamedTempFile::new().unwrap();
    // token_embd first, so output.weight's data starts past offset 0.
    gguf_file::write(
        file.as_file_mut(),
        &[],
        &[("token_embd.weight", &embd), ("output.weight", &output)],
    )
    .unwrap();

    let lexicon = Lexicon::open(file.path()).unwrap();
    assert_eq!(lexicon.hidden(), 512);
    let ids = [5u32, 0, 7, 5];
    let expected = output
        .dequantize(&device)
        .unwrap()
        .index_select(&Tensor::new(&ids, &device).unwrap(), 0)
        .unwrap();
    assert_eq!(
        lexicon.rows(&ids).unwrap().to_vec2::<f32>().unwrap(),
        expected.to_vec2::<f32>().unwrap()
    );
    assert!(lexicon.rows(&[8]).is_err());
}

#[test]
fn lexicon_reads_the_tiny_backbone() {
    let gguf = fixture("tiny/backbone.gguf");
    let lexicon = Lexicon::open(&gguf).unwrap();
    assert_eq!(lexicon.hidden(), 32);
    let mut file = File::open(&gguf).unwrap();
    let content = gguf_file::Content::read(&mut file).unwrap();
    let output = content
        .tensor(&mut file, "output.weight", &Device::Cpu)
        .unwrap()
        .dequantize(&Device::Cpu)
        .unwrap();
    let rows = output.to_vec2::<f32>().unwrap();
    assert_eq!(
        lexicon.rows(&[261, 0]).unwrap().to_vec2::<f32>().unwrap(),
        vec![rows[261].clone(), rows[0].clone()]
    );
    assert!(lexicon.rows(&[262]).is_err());
}
