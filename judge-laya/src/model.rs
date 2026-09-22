//! laya `DecisionModel`: ModernBERT encoder → +type embedding → 2 pre-LN
//! transformer layers → scorer on the `[MASK]` marker positions.
use crate::download::Checkpoint;
use anyhow::{anyhow, Result};
use candle_core::{DType, Device, IndexOp, Module, Tensor, D};
use candle_nn::{
    embedding, layer_norm, linear, Embedding, LayerNorm, LayerNormConfig, Linear, VarBuilder,
};
use candle_transformers::models::modernbert::{Config as EncoderConfig, ModernBert};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// `rl_agent_config.json` — only what inference needs.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub encoder: String,
    #[serde(default = "two")]
    pub head_layers: usize,
    #[serde(default = "d512")]
    pub max_len: usize,
    #[serde(default = "d192")]
    pub head_max_len: usize,
    #[serde(default)]
    pub temperature: Vec<f64>,
    #[serde(default)]
    pub temperature_by_options: HashMap<String, f64>,
}
fn two() -> usize {
    2
}
fn d512() -> usize {
    512
}
fn d192() -> usize {
    192
}

/// transformers v5 `encoder/config.json` → candle's v4-style config.
#[derive(Debug, Deserialize)]
struct HfEncoderConfig {
    vocab_size: usize,
    hidden_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    intermediate_size: usize,
    max_position_embeddings: usize,
    #[serde(default = "eps")]
    layer_norm_eps: f64,
    pad_token_id: u32,
    global_attn_every_n_layers: usize,
    local_attention: usize,
    #[serde(default)]
    global_rope_theta: Option<f64>,
    #[serde(default)]
    local_rope_theta: Option<f64>,
    #[serde(default)]
    rope_parameters: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}
fn eps() -> f64 {
    1e-5
}

pub fn encoder_config(path: &Path) -> Result<EncoderConfig> {
    let hf: HfEncoderConfig = serde_json::from_slice(&std::fs::read(path)?)?;
    let theta = |kind: &str, flat: Option<f64>, fallback: f64| {
        flat.or_else(|| {
            hf.rope_parameters
                .as_ref()?
                .get(kind)?
                .get("rope_theta")?
                .as_f64()
        })
        .unwrap_or(fallback)
    };
    Ok(EncoderConfig {
        vocab_size: hf.vocab_size,
        hidden_size: hf.hidden_size,
        num_hidden_layers: hf.num_hidden_layers,
        num_attention_heads: hf.num_attention_heads,
        intermediate_size: hf.intermediate_size,
        max_position_embeddings: hf.max_position_embeddings,
        layer_norm_eps: hf.layer_norm_eps,
        pad_token_id: hf.pad_token_id,
        global_attn_every_n_layers: hf.global_attn_every_n_layers,
        global_rope_theta: theta("full_attention", hf.global_rope_theta, 160_000.0),
        local_attention: hf.local_attention,
        local_rope_theta: theta("sliding_attention", hf.local_rope_theta, 10_000.0),
        classifier_config: None,
    })
}

/// `nn.TransformerEncoderLayer(d, nhead, 4d, norm_first=True)` with relu.
struct HeadLayer {
    in_proj: Linear,
    out_proj: Linear,
    linear1: Linear,
    linear2: Linear,
    norm1: LayerNorm,
    norm2: LayerNorm,
    heads: usize,
}

impl HeadLayer {
    fn load(vb: VarBuilder, d: usize, heads: usize) -> Result<Self> {
        let ln = LayerNormConfig {
            eps: 1e-5,
            ..Default::default()
        };
        let in_proj = Linear::new(
            vb.get((3 * d, d), "self_attn.in_proj_weight")?,
            Some(vb.get(3 * d, "self_attn.in_proj_bias")?),
        );
        Ok(Self {
            in_proj,
            out_proj: linear(d, d, vb.pp("self_attn.out_proj"))?,
            linear1: linear(d, 4 * d, vb.pp("linear1"))?,
            linear2: linear(4 * d, d, vb.pp("linear2"))?,
            norm1: layer_norm(d, ln, vb.pp("norm1"))?,
            norm2: layer_norm(d, ln, vb.pp("norm2"))?,
            heads,
        })
    }

    /// `key_bias`: (b, 1, 1, s) with 0 for real keys and -inf for padding.
    fn forward(&self, x: &Tensor, key_bias: &Tensor) -> Result<Tensor> {
        let (b, s, d) = x.dims3()?;
        let hd = d / self.heads;
        let h = self.norm1.forward(x)?;
        let qkv = self.in_proj.forward(&h)?;
        let split = |i: usize| -> Result<Tensor> {
            Ok(qkv
                .narrow(D::Minus1, i * d, d)?
                .reshape((b, s, self.heads, hd))?
                .transpose(1, 2)?
                .contiguous()?)
        };
        let (q, k, v) = (split(0)?, split(1)?, split(2)?);
        let scores = (q.matmul(&k.transpose(2, 3)?)? / (hd as f64).sqrt())?;
        let scores = scores.broadcast_add(key_bias)?;
        let attn = candle_nn::ops::softmax_last_dim(&scores)?;
        let ctx = attn.matmul(&v)?.transpose(1, 2)?.reshape((b, s, d))?;
        let x = (x + self.out_proj.forward(&ctx)?)?;
        let ff = self
            .linear2
            .forward(&self.linear1.forward(&self.norm2.forward(&x)?)?.relu()?)?;
        Ok((x + ff)?)
    }
}

pub struct LayaModel {
    encoder: ModernBert,
    head: Vec<HeadLayer>,
    type_emb: Embedding,
    scorer_norm: LayerNorm,
    scorer_1: Linear,
    scorer_3: Linear,
    pub agent: AgentConfig,
    pub pad: u32,
    device: Device,
}

impl LayaModel {
    pub fn load(checkpoint: &Checkpoint, device: Device) -> Result<Self> {
        let agent: AgentConfig = serde_json::from_slice(&std::fs::read(&checkpoint.agent_config)?)?;
        let cfg = encoder_config(&checkpoint.encoder_config)?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[&checkpoint.weights], DType::F32, &device)?
        };
        // The checkpoint stores the encoder under `encoder.`; candle expects `model.`.
        let enc_vb = vb.clone().rename_f(|name: &str| {
            format!("encoder.{}", name.strip_prefix("model.").unwrap_or(name))
        });
        let encoder = ModernBert::load(enc_vb, &cfg)?;
        let d = cfg.hidden_size;
        let heads = (d / 64).max(1);
        let head = (0..agent.head_layers)
            .map(|i| HeadLayer::load(vb.pp(format!("head.layers.{i}")), d, heads))
            .collect::<Result<Vec<_>>>()?;
        let ln = LayerNormConfig {
            eps: 1e-5,
            ..Default::default()
        };
        Ok(Self {
            encoder,
            head,
            type_emb: embedding(3, d, vb.pp("type_emb"))?,
            scorer_norm: layer_norm(d, ln, vb.pp("scorer.0"))?,
            scorer_1: linear(d, d, vb.pp("scorer.1"))?,
            scorer_3: linear(d, 1, vb.pp("scorer.3"))?,
            agent,
            pad: cfg.pad_token_id,
            device,
        })
    }

    /// Raw option logits per row (before temperature), one Vec per sequence.
    pub fn logits(&self, rows: &[(Vec<u32>, Vec<usize>, u32)]) -> Result<Vec<Vec<f32>>> {
        let b = rows.len();
        let s = rows
            .iter()
            .map(|r| r.0.len())
            .max()
            .ok_or_else(|| anyhow!("empty batch"))?;
        let mut ids = vec![self.pad; b * s];
        let mut mask = vec![0u8; b * s];
        for (i, (seq, _, _)) in rows.iter().enumerate() {
            ids[i * s..i * s + seq.len()].copy_from_slice(seq);
            mask[i * s..i * s + seq.len()].fill(1);
        }
        let ids = Tensor::from_vec(ids, (b, s), &self.device)?;
        let mask_t = Tensor::from_vec(mask.clone(), (b, s), &self.device)?;
        let mut h = self.encoder.forward(&ids, &mask_t)?;
        let qtypes = Tensor::from_vec(
            rows.iter().map(|r| r.2).collect::<Vec<_>>(),
            b,
            &self.device,
        )?;
        h = h.broadcast_add(&self.type_emb.forward(&qtypes)?.unsqueeze(1)?)?;
        // Key padding: -inf on padded keys, shape (b, 1, 1, s).
        let bias: Vec<f32> = mask
            .iter()
            .map(|&m| if m == 1 { 0.0 } else { f32::NEG_INFINITY })
            .collect();
        let key_bias = Tensor::from_vec(bias, (b, 1, 1, s), &self.device)?;
        for layer in &self.head {
            h = layer.forward(&h, &key_bias)?;
        }
        let mut out = Vec::with_capacity(b);
        for (i, (_, markers, _)) in rows.iter().enumerate() {
            let idx = Tensor::from_vec(
                markers.iter().map(|&m| m as u32).collect::<Vec<_>>(),
                markers.len(),
                &self.device,
            )?;
            let m = h.i(i)?.index_select(&idx, 0)?;
            let z = self
                .scorer_3
                .forward(
                    &self
                        .scorer_1
                        .forward(&self.scorer_norm.forward(&m)?)?
                        .gelu_erf()?,
                )?
                .squeeze(D::Minus1)?;
            out.push(z.to_vec1::<f32>()?);
        }
        Ok(out)
    }

    /// Mean-pooled encoder states per token row, laya's `embed_fn_from_agent`
    /// (the shortlist's embedding when no separate bi-encoder is configured).
    pub fn embed(&self, rows: &[Vec<u32>]) -> Result<Vec<Vec<f32>>> {
        let b = rows.len();
        let s = rows
            .iter()
            .map(Vec::len)
            .max()
            .ok_or_else(|| anyhow!("empty batch"))?;
        let mut ids = vec![self.pad; b * s];
        let mut mask = vec![0u8; b * s];
        for (i, seq) in rows.iter().enumerate() {
            ids[i * s..i * s + seq.len()].copy_from_slice(seq);
            mask[i * s..i * s + seq.len()].fill(1);
        }
        let ids = Tensor::from_vec(ids, (b, s), &self.device)?;
        let mask = Tensor::from_vec(mask, (b, s), &self.device)?;
        let h = self.encoder.forward(&ids, &mask)?;
        let weights = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let pooled = h
            .broadcast_mul(&weights)?
            .sum(1)?
            .broadcast_div(&weights.sum(1)?.clamp(1f32, f32::MAX)?)?;
        Ok(pooled.to_vec2::<f32>()?)
    }

    /// laya's `temp_bucket` lookup, clamped to [0.5, 5] like `clamp_temperature`.
    pub fn temperature(&self, qtype: u32, k: usize) -> f64 {
        let size = match k {
            0..=2 => "2",
            3..=5 => "3-5",
            6..=10 => "6-10",
            _ => "11+",
        };
        let name = ["choice", "score", "noul"][qtype as usize];
        let raw = self
            .agent
            .temperature_by_options
            .get(&format!("{name}:{size}"))
            .copied()
            .or_else(|| self.agent.temperature.get(qtype as usize).copied())
            .unwrap_or(1.0);
        if raw.is_finite() {
            raw.clamp(0.5, 5.0)
        } else {
            1.0
        }
    }
}
