//! The Rust converter writes the encoder GGUF llama.cpp's own converter
//! (`convert_hf_to_gguf.py`) produced for the tiny checkpoint: same tensors,
//! byte for byte, and the same model metadata.
use candle_core::quantized::gguf_file::{Content, Value};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn read(path: &Path) -> (Content, File) {
    let mut file = File::open(path).unwrap();
    (Content::read(&mut file).unwrap(), file)
}

fn bytes(content: &Content, file: &mut File, name: &str) -> Vec<u8> {
    let info = &content.tensor_infos[name];
    let len = info.shape.elem_count() * info.ggml_dtype.type_size() / info.ggml_dtype.block_size();
    file.seek(SeekFrom::Start(content.tensor_data_offset + info.offset))
        .unwrap();
    let mut data = vec![0u8; len];
    file.read_exact(&mut data).unwrap();
    data
}

#[test]
fn converted_encoder_matches_llama_cpp_converter() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("encoder.gguf");
    let tiny = fixture("tiny");
    judge_laya::gguf::convert(
        &tiny.join("model.safetensors"),
        &tiny.join("encoder/config.json"),
        &tiny.join("tokenizer.json"),
        &out,
    )
    .unwrap();
    let (ours, mut ours_file) = read(&out);
    let (reference, mut reference_file) = read(&fixture("tiny-encoder.reference.gguf"));

    let mut names: Vec<&String> = reference.tensor_infos.keys().collect();
    names.sort();
    let mut ours_names: Vec<&String> = ours.tensor_infos.keys().collect();
    ours_names.sort();
    assert_eq!(ours_names, names);
    for name in names {
        let (a, b) = (&ours.tensor_infos[name], &reference.tensor_infos[name]);
        assert_eq!(a.shape, b.shape, "{name} shape");
        assert_eq!(a.ggml_dtype, b.ggml_dtype, "{name} type");
        assert_eq!(
            bytes(&ours, &mut ours_file, name),
            bytes(&reference, &mut reference_file, name),
            "{name} data"
        );
    }
    let value = |content: &Content, key: &str| content.metadata.get(key).map(|v| format!("{v:?}"));
    for key in [
        "general.architecture",
        "modern-bert.block_count",
        "modern-bert.context_length",
        "modern-bert.embedding_length",
        "modern-bert.feed_forward_length",
        "modern-bert.attention.head_count",
        "modern-bert.rope.freq_base",
        "modern-bert.rope.freq_base_swa",
        "modern-bert.attention.layer_norm_epsilon",
        "modern-bert.attention.causal",
        "modern-bert.attention.sliding_window",
        "modern-bert.attention.sliding_window_pattern",
        "modern-bert.vocab_size",
        "modern-bert.hidden_activation",
        "tokenizer.ggml.model",
        "tokenizer.ggml.bos_token_id",
        "tokenizer.ggml.eos_token_id",
        "tokenizer.ggml.padding_token_id",
    ] {
        assert_eq!(value(&ours, key), value(&reference, key), "{key}");
    }
    let len = |content: &Content, key: &str| match content.metadata.get(key) {
        Some(Value::Array(values)) => values.len(),
        _ => 0,
    };
    for key in ["tokenizer.ggml.tokens", "tokenizer.ggml.merges"] {
        assert_eq!(len(&ours, key), len(&reference, key), "{key}");
    }
}
