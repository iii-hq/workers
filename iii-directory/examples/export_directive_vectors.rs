//! Offline, opt-in synthetic fixture export; no worker registration or Hub loader.
use anyhow::{ensure, Context, Result};
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

fn verified(root: &Path, name: &str, size: usize, sha: &str) -> Result<Vec<u8>> {
    let bytes = std::fs::read(root.join(name)).with_context(|| format!("read {name}"))?;
    ensure!(bytes.len() == size, "size mismatch: {name}");
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == sha,
        "SHA mismatch: {name}"
    );
    Ok(bytes)
}

fn main() -> Result<()> {
    ensure!(
        std::env::var("DIRECTIVE_VECTOR_EXPORT").as_deref() == Ok("1"),
        "explicit opt-in required"
    );
    for name in [
        "CARGO_NET_OFFLINE",
        "ORT_SKIP_DOWNLOAD",
        "ORT_OFFLINE",
        "HF_HUB_OFFLINE",
    ] {
        ensure!(
            std::env::var(name).as_deref() == Ok("1")
                || std::env::var(name).as_deref() == Ok("true"),
            "offline flag required: {name}"
        );
    }
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 4,
        "usage: export_directive_vectors MODEL_ROOT INPUT_JSON NEW_OUTPUT_JSON"
    );
    let root = Path::new(&args[1]);
    ensure!(
        !Path::new(&args[3]).exists(),
        "never overwrite an existing fixture"
    );
    let input = std::fs::read(&args[2])?;
    let cases: Value = serde_json::from_slice(&input)?;
    let mut texts = BTreeMap::new();
    for case in cases.as_array().context("cases array")? {
        texts.insert(case["query"].as_str().context("query")?.to_owned(), ());
        for item in case["candidates"].as_array().context("candidates")? {
            texts.insert(
                item["text"].as_str().context("candidate text")?.to_owned(),
                (),
            );
        }
    }
    ensure!(texts.len() <= 64, "bounded synthetic inputs only");
    // Same pinned bytes, mean pooling, max length and thread count as
    // search_semantic::load_verified_minilm / LoadedModel::encode.
    let model_sha = "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452";
    let tokenizer_sha = "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037";
    let onnx = verified(root, "onnx/model.onnx", 90_405_214, model_sha)?;
    let files = TokenizerFiles {
        tokenizer_file: verified(root, "tokenizer.json", 466_247, tokenizer_sha)?,
        config_file: verified(
            root,
            "config.json",
            612,
            "953f9c0d463486b10a6871cc2fd59f223b2c70184f49815e7efbcab5d8908b41",
        )?,
        special_tokens_map_file: verified(
            root,
            "special_tokens_map.json",
            112,
            "303df45a03609e4ead04bc3dc1536d0ab19b5358db685b6f3da123d05ec200e3",
        )?,
        tokenizer_config_file: verified(
            root,
            "tokenizer_config.json",
            350,
            "acb92769e8195aabd29b7b2137a9e6d6e25c476a4f15aa4355c233426c61576b",
        )?,
    };
    let mut model = TextEmbedding::try_new_from_user_defined(
        UserDefinedEmbeddingModel::new(onnx, files).with_pooling(Pooling::Mean),
        InitOptionsUserDefined::new()
            .with_max_length(512)
            .with_intra_threads(2),
    )?;
    let texts: Vec<String> = texts.into_keys().collect();
    let mut vectors = model.embed(texts.clone(), Some(8))?;
    ensure!(vectors.len() == texts.len(), "one vector per exact input");
    for v in &mut vectors {
        ensure!(
            v.len() == 384 && v.iter().all(|n| n.is_finite()),
            "invalid vector"
        );
        let norm: f32 = v.iter().map(|n| n * n).sum();
        ensure!(norm.is_finite() && norm > f32::EPSILON, "zero norm");
        let inverse = norm.sqrt().recip();
        for n in v.iter_mut() {
            *n *= inverse;
        }
        ensure!(
            (v.iter().map(|n| n * n).sum::<f32>() - 1.0).abs() < 0.00001,
            "L2 norm"
        );
    }
    let items: BTreeMap<_, _> = texts
        .into_iter()
        .zip(vectors)
        .enumerate()
        .map(|(n, (text, v))| (format!("input-{n:02}"), json!({"text":text,"v":v})))
        .collect();
    let output = json!({"model":"sentence-transformers/all-MiniLM-L6-v2@c9745ed1d9f2","dims":384,
        "provenance":{"repository":"sentence-transformers/all-MiniLM-L6-v2",
        "revision":"c9745ed1d9f207416be6d2e6f8de32d1f16199bf","model_sha256":model_sha,
        "tokenizer_sha256":tokenizer_sha,"input_sha256":format!("{:x}",Sha256::digest(&input)),
        "backend":"fastembed=6.0.0; ort=2.0.0-rc.13; local user-defined model",
        "pooling":"mean","normalization":"L2 f32","max_length":512,"intra_threads":2,
        "batch_size":8,"both_ends":true,"provider_calls":0,"synthetic_only":true},"items":items});
    let bytes = serde_json::to_vec(&output)?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])?
        .write_all(&bytes)?;
    println!(
        "{}",
        json!({"fixture":args[3],"sha256":format!("{:x}",Sha256::digest(&bytes)),
        "unique_inputs":output["items"].as_object().unwrap().len(),"dimensions":384,"both_ends":true,"provider_calls":0})
    );
    Ok(())
}
