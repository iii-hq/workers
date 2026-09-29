//! Write the checkpoint's encoder as the GGUF llama.cpp loads (`modern-bert`),
//! the way llama.cpp's `convert_hf_to_gguf.py` does, straight from
//! `model.safetensors`: no Python, no separate download. Matrices are f16
//! (laya ships them in f16, so they are copied as they are), norms f32. The
//! vocabulary only satisfies llama.cpp's loader: the worker tokenizes with the
//! checkpoint's own tokenizer and passes ids.
use anyhow::{anyhow, bail, Context, Result};
use half::{bf16, f16};
use safetensors::{Dtype, SafeTensors};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufWriter, Seek, Write},
    path::Path,
};

const ALIGNMENT: u64 = 32;
const GGML_F32: u32 = 0;
const GGML_F16: u32 = 1;

enum Meta {
    U32(u32),
    I32s(Vec<i32>),
    F32(f32),
    Bool(bool),
    Str(String),
    Strs(Vec<String>),
}

/// Convert `weights`' `encoder.*` tensors, described by `encoder_config` and
/// `tokenizer`, into a GGUF at `out` (written through a temporary file).
pub fn convert(weights: &Path, encoder_config: &Path, tokenizer: &Path, out: &Path) -> Result<()> {
    let cfg: Value = serde_json::from_slice(&std::fs::read(encoder_config)?)?;
    let file = File::open(weights).with_context(|| format!("open {}", weights.display()))?;
    // SAFETY: the checkpoint is read-only for the worker's lifetime.
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let st = SafeTensors::deserialize(&mmap)?;

    let int = |key: &str| -> Result<u32> {
        cfg.get(key)
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .ok_or_else(|| anyhow!("encoder config lacks {key}"))
    };
    let layers = int("num_hidden_layers")?;
    let theta = |kind: &str, flat: &str, default: f64| {
        cfg.get(flat)
            .and_then(Value::as_f64)
            .or_else(|| {
                cfg.pointer(&format!("/rope_parameters/{kind}/rope_theta"))?
                    .as_f64()
            })
            .unwrap_or(default) as f32
    };
    let eps = ["layer_norm_eps", "norm_eps"]
        .iter()
        .find_map(|key| cfg.get(*key).and_then(Value::as_f64))
        .unwrap_or(1e-5) as f32;
    let (tokens, types, merges) = vocabulary(tokenizer, int("vocab_size")? as usize)?;
    let arch = "modern-bert";
    let key = |k: &str| format!("{arch}.{k}");
    let mut meta: Vec<(String, Meta)> = vec![
        ("general.architecture".into(), Meta::Str(arch.into())),
        ("general.type".into(), Meta::Str("model".into())),
        ("general.name".into(), Meta::Str("laya encoder".into())),
        ("general.file_type".into(), Meta::U32(1)),
        ("general.quantization_version".into(), Meta::U32(2)),
        (key("block_count"), Meta::U32(layers)),
        (
            key("context_length"),
            Meta::U32(int("max_position_embeddings")?),
        ),
        (key("embedding_length"), Meta::U32(int("hidden_size")?)),
        (
            key("feed_forward_length"),
            Meta::U32(int("intermediate_size")?),
        ),
        (
            key("attention.head_count"),
            Meta::U32(int("num_attention_heads")?),
        ),
        (
            key("rope.freq_base"),
            Meta::F32(theta("full_attention", "global_rope_theta", 160_000.0)),
        ),
        (
            key("rope.freq_base_swa"),
            Meta::F32(theta("sliding_attention", "local_rope_theta", 10_000.0)),
        ),
        (key("attention.layer_norm_rms_epsilon"), Meta::F32(eps)),
        (key("attention.layer_norm_epsilon"), Meta::F32(eps)),
        (key("attention.causal"), Meta::Bool(false)),
        (
            key("attention.sliding_window"),
            Meta::U32(int("local_attention")?),
        ),
        (
            key("attention.sliding_window_pattern"),
            Meta::U32(int("global_attn_every_n_layers")?),
        ),
        (key("rope.scaling.type"), Meta::Str("none".into())),
        (key("vocab_size"), Meta::U32(tokens.len() as u32)),
        ("tokenizer.ggml.model".into(), Meta::Str("gpt2".into())),
        // Only llama.cpp's own tokenizer reads it, and the worker never uses that.
        ("tokenizer.ggml.pre".into(), Meta::Str("default".into())),
        ("tokenizer.ggml.tokens".into(), Meta::Strs(tokens)),
        ("tokenizer.ggml.token_type".into(), Meta::I32s(types)),
        ("tokenizer.ggml.merges".into(), Meta::Strs(merges)),
    ];
    if let Some(act) = cfg.get("hidden_activation").and_then(Value::as_str) {
        meta.push((key("hidden_activation"), Meta::Str(act.into())));
    }
    for (field, name) in [
        ("bos_token_id", "bos"),
        ("eos_token_id", "eos"),
        ("sep_token_id", "seperator"),
        ("pad_token_id", "padding"),
    ] {
        if let Some(id) = cfg.get(field).and_then(Value::as_u64) {
            meta.push((
                format!("tokenizer.ggml.{name}_token_id"),
                Meta::U32(id as u32),
            ));
        }
    }

    // (gguf name, checkpoint name, ggml type), in llama.cpp's order.
    let mut tensors = vec![
        (
            "token_embd_norm.weight".to_string(),
            "encoder.embeddings.norm.weight".to_string(),
            GGML_F32,
        ),
        (
            "token_embd.weight".into(),
            "encoder.embeddings.tok_embeddings.weight".into(),
            GGML_F16,
        ),
        (
            "output_norm.weight".into(),
            "encoder.final_norm.weight".into(),
            GGML_F32,
        ),
    ];
    for i in 0..layers {
        let src = |part: &str| format!("encoder.layers.{i}.{part}.weight");
        let dst = |part: &str| format!("blk.{i}.{part}.weight");
        tensors.push((dst("attn_output"), src("attn.Wo"), GGML_F16));
        tensors.push((dst("attn_qkv"), src("attn.Wqkv"), GGML_F16));
        // Layer 0 has no attention norm (identity).
        if st.tensor(&src("attn_norm")).is_ok() {
            tensors.push((dst("attn_norm"), src("attn_norm"), GGML_F32));
        }
        tensors.push((dst("ffn_up"), src("mlp.Wi"), GGML_F16));
        tensors.push((dst("ffn_down"), src("mlp.Wo"), GGML_F16));
        tensors.push((dst("ffn_norm"), src("mlp_norm"), GGML_F32));
    }

    // A private temporary name: concurrent workers (or tests) may convert the
    // same checkpoint at once; the atomic rename makes the last one win.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = out.with_extension(format!(
        "gguf.{}-{}.partial",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut w =
        BufWriter::new(File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?);
    w.write_all(b"GGUF")?;
    w.write_all(&3u32.to_le_bytes())?;
    w.write_all(&(tensors.len() as u64).to_le_bytes())?;
    w.write_all(&(meta.len() as u64).to_le_bytes())?;
    for (name, value) in &meta {
        string(&mut w, name)?;
        match value {
            Meta::U32(v) => {
                w.write_all(&4u32.to_le_bytes())?;
                w.write_all(&v.to_le_bytes())?;
            }
            Meta::F32(v) => {
                w.write_all(&6u32.to_le_bytes())?;
                w.write_all(&v.to_le_bytes())?;
            }
            Meta::Bool(v) => {
                w.write_all(&7u32.to_le_bytes())?;
                w.write_all(&[*v as u8])?;
            }
            Meta::Str(v) => {
                w.write_all(&8u32.to_le_bytes())?;
                string(&mut w, v)?;
            }
            Meta::Strs(values) => {
                w.write_all(&9u32.to_le_bytes())?;
                w.write_all(&8u32.to_le_bytes())?;
                w.write_all(&(values.len() as u64).to_le_bytes())?;
                for v in values {
                    string(&mut w, v)?;
                }
            }
            Meta::I32s(values) => {
                w.write_all(&9u32.to_le_bytes())?;
                w.write_all(&5u32.to_le_bytes())?;
                w.write_all(&(values.len() as u64).to_le_bytes())?;
                for v in values {
                    w.write_all(&v.to_le_bytes())?;
                }
            }
        }
    }
    let mut offset = 0u64;
    let mut sources = Vec::with_capacity(tensors.len());
    for (name, src, ggml) in &tensors {
        let view = st
            .tensor(src)
            .with_context(|| format!("checkpoint lacks {src}"))?;
        let shape = view.shape().to_vec();
        string(&mut w, name)?;
        w.write_all(&(shape.len() as u32).to_le_bytes())?;
        // GGUF lists dimensions innermost first; the data layout is the same.
        for dim in shape.iter().rev() {
            w.write_all(&(*dim as u64).to_le_bytes())?;
        }
        w.write_all(&ggml.to_le_bytes())?;
        w.write_all(&offset.to_le_bytes())?;
        let bytes = shape.iter().product::<usize>() as u64 * if *ggml == GGML_F16 { 2 } else { 4 };
        offset += bytes.div_ceil(ALIGNMENT) * ALIGNMENT;
        sources.push((view, *ggml));
    }
    let mut written = w.stream_position()?;
    pad(&mut w, &mut written)?;
    for (view, ggml) in sources {
        let data = encode(view.dtype(), view.data(), ggml)?;
        w.write_all(&data)?;
        written += data.len() as u64;
        pad(&mut w, &mut written)?;
    }
    w.into_inner()
        .map_err(|e| anyhow!("flush: {e}"))?
        .sync_all()?;
    std::fs::rename(&tmp, out)?;
    Ok(())
}

fn pad(w: &mut impl Write, written: &mut u64) -> Result<()> {
    let padding = (ALIGNMENT - *written % ALIGNMENT) % ALIGNMENT;
    w.write_all(&vec![0u8; padding as usize])?;
    *written += padding;
    Ok(())
}

fn string(w: &mut impl Write, v: &str) -> Result<()> {
    w.write_all(&(v.len() as u64).to_le_bytes())?;
    w.write_all(v.as_bytes())?;
    Ok(())
}

/// Checkpoint bytes as ggml f16 or f32 little-endian.
fn encode(dtype: Dtype, data: &[u8], ggml: u32) -> Result<Vec<u8>> {
    let values: Vec<f32> = match dtype {
        Dtype::F16 if ggml == GGML_F16 => return Ok(data.to_vec()),
        Dtype::F32 if ggml == GGML_F32 => return Ok(data.to_vec()),
        Dtype::F16 => data
            .chunks_exact(2)
            .map(|b| f16::from_le_bytes([b[0], b[1]]).to_f32())
            .collect(),
        Dtype::BF16 => data
            .chunks_exact(2)
            .map(|b| bf16::from_le_bytes([b[0], b[1]]).to_f32())
            .collect(),
        Dtype::F32 => data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        other => bail!("unsupported checkpoint dtype {other:?}"),
    };
    Ok(if ggml == GGML_F16 {
        values
            .iter()
            .flat_map(|v| f16::from_f32(*v).to_le_bytes())
            .collect()
    } else {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    })
}

/// `tokenizer.json`'s BPE vocabulary as llama.cpp's gpt2 arrays: tokens by id
/// (missing ids become `[PAD{i}]`, unused), token types and merges.
fn vocabulary(tokenizer: &Path, vocab_size: usize) -> Result<(Vec<String>, Vec<i32>, Vec<String>)> {
    let json: Value = serde_json::from_slice(&std::fs::read(tokenizer)?)?;
    let size = vocab_size.max(
        json.pointer("/model/vocab")
            .and_then(Value::as_object)
            .map_or(0, |vocab| vocab.len()),
    );
    let mut tokens: Vec<Option<String>> = vec![None; size];
    let mut types = vec![5i32; size];
    if let Some(vocab) = json.pointer("/model/vocab").and_then(Value::as_object) {
        for (token, id) in vocab {
            if let Some(id) = id.as_u64().map(|id| id as usize).filter(|&id| id < size) {
                tokens[id] = Some(token.clone());
                types[id] = 1;
            }
        }
    }
    for added in json
        .get("added_tokens")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(content)) = (
            added
                .get("id")
                .and_then(Value::as_u64)
                .map(|id| id as usize),
            added.get("content").and_then(Value::as_str),
        ) else {
            continue;
        };
        if id < size {
            tokens[id] = Some(content.to_owned());
            types[id] = if added.get("special").and_then(Value::as_bool) == Some(true) {
                3
            } else {
                4
            };
        }
    }
    let tokens = tokens
        .into_iter()
        .enumerate()
        .map(|(i, token)| token.unwrap_or_else(|| format!("[PAD{i}]")))
        .collect();
    let merges = json
        .pointer("/model/merges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|merge| match merge {
            Value::String(pair) => Some(pair.clone()),
            Value::Array(pair) => Some(
                pair.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        })
        .collect();
    Ok((tokens, types, merges))
}
