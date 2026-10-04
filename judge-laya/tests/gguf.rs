//! The Rust converter writes the GGUF llama.cpp's own converter
//! (`convert_hf_to_gguf.py` at b11379) produced for the tiny checkpoint: the
//! same tensors, byte for byte, and the same model and decision metadata.
use std::{collections::BTreeMap, path::Path};

/// Keys only the converter's own tooling reads: names, the pooling (the
/// worker pools nothing) and llama.cpp's tokenizer and chat template (the
/// worker tokenizes itself).
const UNWRITTEN: [&str; 11] = [
    "general.name",
    "general.size_label",
    "modern-bert.classifier.pooling_type",
    "tokenizer.ggml.pre",
    "tokenizer.ggml.add_bos_token",
    "tokenizer.ggml.add_eos_token",
    "tokenizer.ggml.add_sep_token",
    "tokenizer.ggml.unknown_token_id",
    "tokenizer.ggml.mask_token_id",
    "tokenizer.chat_template.systemone",
    "tokenizer.chat_templates",
];

/// A GGUF v3 file: metadata as typed text, tensors as (dims, type, bytes).
struct Gguf {
    meta: BTreeMap<String, String>,
    tensors: BTreeMap<String, (Vec<u64>, u32, Vec<u8>)>,
}

/// Reads little-endian GGUF fields from `.0` at offset `.1`.
struct Cursor<'a>(&'a [u8], usize);

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        self.1 += n;
        &self.0[self.1 - n..self.1]
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    fn string(&mut self) -> String {
        let n = self.u64() as usize;
        String::from_utf8(self.take(n).to_vec()).unwrap()
    }
    fn value(&mut self, ty: u32) -> String {
        let text = match ty {
            0 | 1 | 7 => self.take(1)[0].to_string(),
            2 | 3 => u16::from_le_bytes(self.take(2).try_into().unwrap()).to_string(),
            4 | 5 => self.u32().to_string(),
            6 => f32::from_bits(self.u32()).to_string(),
            8 => self.string(),
            9 => {
                let (ty, n) = (self.u32(), self.u64());
                (0..n)
                    .map(|_| self.value(ty))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            10..=12 => self.u64().to_string(),
            other => panic!("GGUF value type {other}"),
        };
        format!("{ty}:{text}")
    }
}

fn read(path: &Path) -> Gguf {
    let data = std::fs::read(path).unwrap();
    let mut at = Cursor(&data, 0);
    assert_eq!(at.take(4), b"GGUF");
    assert_eq!(at.u32(), 3);
    let (n_tensors, n_meta) = (at.u64(), at.u64());
    let meta = (0..n_meta)
        .map(|_| {
            let key = at.string();
            let ty = at.u32();
            (key, at.value(ty))
        })
        .collect();
    let infos: Vec<_> = (0..n_tensors)
        .map(|_| {
            let name = at.string();
            let dims: Vec<u64> = (0..at.u32()).map(|_| at.u64()).collect();
            (name, dims, at.u32(), at.u64())
        })
        .collect();
    // Tensor data starts at the next 32-byte boundary (the default alignment).
    let start = at.1.div_ceil(32) * 32;
    let tensors = infos
        .into_iter()
        .map(|(name, dims, ty, offset)| {
            let size = dims.iter().product::<u64>() as usize * if ty == 1 { 2 } else { 4 };
            let from = start + offset as usize;
            (name, (dims, ty, data[from..from + size].to_vec()))
        })
        .collect();
    Gguf { meta, tensors }
}

#[test]
fn converted_checkpoint_matches_llama_cpp_converter() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("model.gguf");
    let tiny = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny");
    judge_laya::gguf::convert(
        &tiny.join("model.safetensors"),
        &tiny.join("encoder/config.json"),
        &tiny.join("rl_agent_config.json"),
        &tiny.join("tokenizer.json"),
        &out,
    )
    .unwrap();
    let ours = read(&out);
    let reference = read(&tiny.with_file_name("tiny.reference.gguf"));

    assert_eq!(
        ours.tensors.keys().collect::<Vec<_>>(),
        reference.tensors.keys().collect::<Vec<_>>()
    );
    for (name, (dims, ty, bytes)) in &reference.tensors {
        let (our_dims, our_ty, our_bytes) = &ours.tensors[name];
        assert_eq!((our_dims, our_ty), (dims, ty), "{name}");
        assert!(our_bytes == bytes, "{name} data");
    }
    let written = |gguf: Gguf| -> BTreeMap<String, String> {
        gguf.meta
            .into_iter()
            .filter(|(key, _)| !UNWRITTEN.contains(&key.as_str()))
            .collect()
    };
    let (ours, reference) = (written(ours), written(reference));
    assert_eq!(
        ours.keys().collect::<Vec<_>>(),
        reference.keys().collect::<Vec<_>>()
    );
    for (key, value) in &reference {
        assert!(ours[key] == *value, "{key}");
    }
}
