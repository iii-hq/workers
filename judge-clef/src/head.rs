//! Clef's joint schema head (`JointSchemaHead`, joint_schema_model.py:281-459)
//! on candle, f32 on the CPU, and the backbone `output.weight` rows its
//! lexical prior reads.
use crate::encode::{Encoded, Field};
use anyhow::{ensure, Context, Result};
use candle_core::{
    quantized::{ggml_file::qtensor_from_ggml, gguf_file::Content, GgmlDType},
    DType, Device, Module, Tensor,
};
use candle_nn::{
    embedding, layer_norm, linear, linear_no_bias, ops::softmax_last_dim, Embedding, LayerNorm,
    Linear, VarBuilder,
};
use serde::Deserialize;
use std::{fs::File, io::BufReader, ops::Range, os::unix::fs::FileExt, path::Path};

/// torch.nn.LayerNorm's default eps, used by every norm of the head.
const EPS: f64 = 1e-5;
/// Query rows per attention step: bounds the `[heads, rows, L]` scores
/// (16 × 128 × 16k × 4 B = 128 MiB) however many options a request has.
const ROWS: usize = 128;

/// output.weight (the untied lm_head) of the backbone GGUF, read row by row:
/// the prior needs a few hundred of its rows, never the whole matrix.
pub struct Lexicon {
    file: File,
    dtype: GgmlDType,
    /// File offset of row 0.
    start: u64,
    /// Bytes per row: whole blocks, so n rows are exactly the n*row bytes
    /// qtensor_from_ggml reads (it does not check the length).
    row: usize,
    vocab: usize,
    hidden: usize,
}

impl Lexicon {
    /// Read the GGUF header and locate output.weight.
    pub fn open(gguf: &Path) -> Result<Self> {
        let file = File::open(gguf).with_context(|| format!("open {}", gguf.display()))?;
        let content = Content::read(&mut BufReader::new(&file))?;
        let info = content
            .tensor_infos
            .get("output.weight")
            .context("the GGUF has no output.weight")?;
        // GGUF dims are innermost first; candle reverses them: [vocab, hidden].
        let (vocab, hidden) = info.shape.dims2()?;
        let (block, size) = (info.ggml_dtype.block_size(), info.ggml_dtype.type_size());
        ensure!(
            hidden.is_multiple_of(block),
            "output.weight rows of {hidden} are not whole {block}-element blocks"
        );
        Ok(Self {
            start: content.tensor_data_offset + info.offset,
            row: hidden / block * size,
            dtype: info.ggml_dtype,
            vocab,
            hidden,
            file,
        })
    }

    /// The rows' width: the backbone's hidden size.
    pub fn hidden(&self) -> usize {
        self.hidden
    }

    /// Rows of `ids` as f32 `[ids.len(), hidden]`, dequantized.
    pub fn rows(&self, ids: &[u32]) -> Result<Tensor> {
        let mut raw = vec![0u8; ids.len() * self.row];
        for (&id, out) in ids.iter().zip(raw.chunks_exact_mut(self.row)) {
            ensure!(
                (id as usize) < self.vocab,
                "token {id} is outside output.weight's {} rows",
                self.vocab
            );
            self.file
                .read_exact_at(out, self.start + u64::from(id) * self.row as u64)?;
        }
        let rows = qtensor_from_ggml(self.dtype, &raw, vec![ids.len(), self.hidden], &Device::Cpu)?;
        Ok(rows.dequantize(&Device::Cpu)?)
    }
}

/// joint_head_config.json.
#[derive(Deserialize)]
struct Config {
    hidden_size: usize,
    width: usize,
    routing_layers: usize,
    layers: usize,
    heads: usize,
    feedforward: usize,
}

/// `nn.MultiheadAttention(width, heads, batch_first=True)`, no mask.
struct Attention {
    q: Linear,
    /// k and v in one matmul.
    kv: Linear,
    out: Linear,
    heads: usize,
}

impl Attention {
    fn load(vb: VarBuilder, w: usize, heads: usize) -> Result<Self> {
        // in_proj rows: q, k, v.
        let weight = vb.get((3 * w, w), "in_proj_weight")?;
        let bias = vb.get(3 * w, "in_proj_bias")?;
        Ok(Self {
            q: Linear::new(weight.narrow(0, 0, w)?, Some(bias.narrow(0, 0, w)?)),
            kv: Linear::new(weight.narrow(0, w, 2 * w)?, Some(bias.narrow(0, w, 2 * w)?)),
            out: linear(w, w, vb.pp("out_proj"))?,
            heads,
        })
    }

    /// `query` [n, w] attends over `memory` [m, w].
    fn forward(&self, query: &Tensor, memory: &Tensor) -> Result<Tensor> {
        let (n, w) = query.dims2()?;
        let m = memory.dim(0)?;
        let hd = w / self.heads;
        // [len, w] → [heads, len, hd]
        let split = |x: Tensor, len: usize| {
            x.reshape((len, self.heads, hd))?
                .transpose(0, 1)?
                .contiguous()
        };
        // ponytail: K/V are projected over all L memory rows in each of the 6
        // cross-attentions, most of the head's FLOPs. Folding W_k/W_v into the
        // few queries is exact (the K bias cancels in the softmax, the V bias
        // passes through) and ~3x cheaper; do it if the head shows in request time.
        let kv = self.kv.forward(memory)?;
        let k = split(kv.narrow(1, 0, w)?, m)?.t()?;
        let v = split(kv.narrow(1, w, w)?, m)?;
        let q = (split(self.q.forward(query)?, n)? / (hd as f64).sqrt())?;
        let mut parts = Vec::with_capacity(n.div_ceil(ROWS));
        for at in (0..n).step_by(ROWS) {
            let q = q.narrow(1, at, ROWS.min(n - at))?;
            parts.push(softmax_last_dim(&q.matmul(&k)?)?.matmul(&v)?);
        }
        let heads = Tensor::cat(&parts, 1)?.transpose(0, 1)?.reshape((n, w))?;
        Ok(self.out.forward(&heads)?)
    }
}

/// A head block as clef.cpp keeps them: `EvidenceRoutingLayer` (cross-attention
/// to the normed memory) or `TransformerDecoderLayer(norm_first)`
/// (self-attention, then cross-attention to the raw memory).
struct Block {
    self_attn: Option<(LayerNorm, Attention)>,
    cross_norm: LayerNorm,
    memory_norm: Option<LayerNorm>,
    cross: Attention,
    ff_norm: LayerNorm,
    up: Linear,
    down: Linear,
}

impl Block {
    /// `evidence_layers.N`.
    fn routing(vb: VarBuilder, c: &Config) -> Result<Self> {
        let (w, f) = (c.width, c.feedforward);
        Ok(Self {
            self_attn: None,
            cross_norm: layer_norm(w, EPS, vb.pp("query_norm"))?,
            memory_norm: Some(layer_norm(w, EPS, vb.pp("memory_norm"))?),
            cross: Attention::load(vb.pp("attention"), w, c.heads)?,
            ff_norm: layer_norm(w, EPS, vb.pp("feedforward_norm"))?,
            up: linear(w, f, vb.pp("feedforward.0"))?,
            down: linear(f, w, vb.pp("feedforward.3"))?,
        })
    }

    /// `layers.N`.
    fn decoder(vb: VarBuilder, c: &Config) -> Result<Self> {
        let (w, f) = (c.width, c.feedforward);
        Ok(Self {
            self_attn: Some((
                layer_norm(w, EPS, vb.pp("norm1"))?,
                Attention::load(vb.pp("self_attn"), w, c.heads)?,
            )),
            cross_norm: layer_norm(w, EPS, vb.pp("norm2"))?,
            memory_norm: None,
            cross: Attention::load(vb.pp("multihead_attn"), w, c.heads)?,
            ff_norm: layer_norm(w, EPS, vb.pp("norm3"))?,
            up: linear(w, f, vb.pp("linear1"))?,
            down: linear(f, w, vb.pp("linear2"))?,
        })
    }

    fn forward(&self, x: &Tensor, memory: &Tensor) -> Result<Tensor> {
        let mut x = x.clone();
        if let Some((norm, attn)) = &self.self_attn {
            let h = norm.forward(&x)?;
            x = (&x + attn.forward(&h, &h)?)?;
        }
        // The reference norms the memory twice (key and value), to the same result.
        let memory = match &self.memory_norm {
            Some(norm) => norm.forward(memory)?,
            None => memory.clone(),
        };
        x = (&x + self.cross.forward(&self.cross_norm.forward(&x)?, &memory)?)?;
        let ff = self
            .down
            .forward(&self.up.forward(&self.ff_norm.forward(&x)?)?.gelu_erf()?)?;
        Ok((x + ff)?)
    }
}

pub struct Head {
    hidden_norm: LayerNorm,
    memory: Linear,
    question: Linear,
    option_question: Linear,
    global: Linear,
    option_context: Linear,
    option_lexical: Linear,
    types: Embedding,
    routing: Vec<Block>,
    option_summary_norm: LayerNorm,
    layers: Vec<Block>,
    field_norm: LayerNorm,
    option_norm: LayerNorm,
    scorer: Linear,
    scorer_out: Linear,
    prior_scale: f64,
    joint_scale: f64,
    /// sigmoid(residual_gate).
    gate: f64,
    width: usize,
}

impl Head {
    /// `weights`: joint_head.safetensors (bf16, read as f32); `config`:
    /// joint_head_config.json, whose hidden_size must equal `hidden`.
    pub fn load(weights: &Path, config: &Path, hidden: usize) -> Result<Self> {
        let c: Config = serde_json::from_slice(
            &std::fs::read(config).with_context(|| format!("read {}", config.display()))?,
        )?;
        ensure!(
            c.hidden_size == hidden,
            "the joint head expects hidden size {}, the backbone has {hidden}",
            c.hidden_size
        );
        ensure!(
            c.heads > 0 && c.width.is_multiple_of(c.heads),
            "width {} does not split into {} heads",
            c.width,
            c.heads
        );
        let bytes =
            std::fs::read(weights).with_context(|| format!("read {}", weights.display()))?;
        let vb = VarBuilder::from_buffered_safetensors(bytes, DType::F32, &Device::Cpu)?;
        let (h, w) = (c.hidden_size, c.width);
        let project = |name: &str| linear_no_bias(h, w, vb.pp(name));
        // The scalars in f64 from their stored bf16, as llama.cpp's converter.
        let scalar =
            |name: &str| -> Result<f64> { Ok(f64::from(vb.get((), name)?.to_scalar::<f32>()?)) };
        // exp(min(param, ln 100)) (joint_schema_model.py:438, :453).
        let scale = |name: &str| -> Result<f64> { Ok(scalar(name)?.min(100f64.ln()).exp()) };
        Ok(Self {
            hidden_norm: layer_norm(h, EPS, vb.pp("hidden_norm"))?,
            memory: project("memory_projection")?,
            question: project("question_projection")?,
            option_question: project("option_question_projection")?,
            global: project("global_projection")?,
            option_context: project("option_context_projection")?,
            option_lexical: project("option_lexical_projection")?,
            types: embedding(3, w, vb.pp("type_embedding"))?,
            routing: (0..c.routing_layers)
                .map(|i| Block::routing(vb.pp(format!("evidence_layers.{i}")), &c))
                .collect::<Result<_>>()?,
            option_summary_norm: layer_norm(w, EPS, vb.pp("option_summary_norm"))?,
            layers: (0..c.layers)
                .map(|i| Block::decoder(vb.pp(format!("layers.{i}")), &c))
                .collect::<Result<_>>()?,
            field_norm: layer_norm(w, EPS, vb.pp("field_norm"))?,
            option_norm: layer_norm(w, EPS, vb.pp("option_norm"))?,
            scorer: linear(4 * w, w, vb.pp("residual_scorer.0"))?,
            scorer_out: linear(w, 1, vb.pp("residual_scorer.3"))?,
            prior_scale: scale("prior_logit_scale")?,
            joint_scale: scale("joint_logit_scale")?,
            gate: 1.0 / (1.0 + (-scalar("residual_gate")?).exp()),
            width: w,
        })
    }

    /// One logit per option of each field: `hidden` [L, H] final-norm states,
    /// `lexical` [T, H] the output.weight rows of `option_ids`.
    fn forward(&self, hidden: Tensor, fields: &[Field], lexical: &Tensor) -> Result<Vec<Vec<f32>>> {
        let l = hidden.dim(0)?;
        let options: Vec<&Range<usize>> = fields.iter().flat_map(|f| &f.options).collect();
        let mean = |x: &Tensor, r: &Range<usize>| x.narrow(0, r.start, r.len())?.mean_keepdim(0);
        let cat = |rows: candle_core::Result<Vec<Tensor>>| Tensor::cat(&rows?, 0);

        let s = self.hidden_norm.forward(&hidden)?; // :353
        drop(hidden);
        let memory = self.memory.forward(&s)?; // [L, W] :357
        let global = s.narrow(0, l - 1, 1)?; // :358
        let question = cat(fields.iter().map(|f| mean(&s, &f.span)).collect())?; // [Q, H]
        let context = cat(options.iter().map(|r| mean(&s, r)).collect())?; // [O, H]
        let anchor = l2(&question.broadcast_add(&global)?, 1e-12)?; // :433-436
        let global = self.global.forward(&global)?;
        drop(s);
        let mut at = 0;
        let lexical = cat(options
            .iter()
            .map(|r| {
                at += r.len();
                mean(lexical, &(at - r.len()..at))
            })
            .collect())?; // [O, H] :379-383
        let owner: Vec<u32> = fields
            .iter()
            .enumerate()
            .flat_map(|(i, f)| std::iter::repeat_n(i as u32, f.options.len()))
            .collect();
        let owner = Tensor::from_vec(owner, options.len(), &Device::Cpu)?;

        let mut routed = ((self.option_context.forward(&context)?
            + self.option_lexical.forward(&lexical)?)?
            + self
                .option_question
                .forward(&question)?
                .index_select(&owner, 0)?)?; // :392-398
        for block in &self.routing {
            routed = block.forward(&routed, &memory)?;
        }

        let base = self.question.forward(&question)?; // :405
        let mut at = 0;
        let summaries = cat(fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let options = routed.narrow(0, at, f.options.len())?;
                at += f.options.len();
                let scores = base.narrow(0, i, 1)?.matmul(&options.t()?)?;
                softmax_last_dim(&(scores / (self.width as f64).sqrt())?)?.matmul(&options)
            })
            .collect())?; // :406-414
        let kinds = Tensor::from_vec(
            fields.iter().map(|f| f.kind).collect::<Vec<_>>(),
            fields.len(),
            &Device::Cpu,
        )?;
        let mut x = ((&base + self.option_summary_norm.forward(&summaries)?)?
            .broadcast_add(&global)?
            + self.types.forward(&kinds)?)?; // :415-420
        for block in &self.layers {
            x = block.forward(&x, &memory)?;
        }
        let x = self.field_norm.forward(&x)?.index_select(&owner, 0)?; // [O, W] :424

        let prior =
            ((l2(&lexical, 1e-12)? * anchor.index_select(&owner, 0)?)?.sum(1)? * self.prior_scale)?; // :437-439
        let options = self.option_norm.forward(&routed)?; // :440
        let cosine = (l2(&x, 1e-8)? * l2(&options, 1e-8)?)?.sum(1)?; // :442
        let features = Tensor::cat(
            &[&x, &options, &(&x * &options)?, &(&x - &options)?.abs()?],
            1,
        )?;
        let residual = self
            .scorer_out
            .forward(&self.scorer.forward(&features)?.gelu_erf()?)?
            .squeeze(1)?; // :452
        let logits = (prior + (((cosine * self.joint_scale)? + residual)? * self.gate)?)?
            .to_vec1::<f32>()?; // :453-457
        let mut rest = logits.as_slice();
        Ok(fields
            .iter()
            .map(|f| {
                let (mine, tail) = rest.split_at(f.options.len());
                rest = tail;
                mine.to_vec()
            })
            .collect())
    }

    /// `forward` on one encoded record: `states` is the engine's `[L × H]`.
    pub fn logits(
        &self,
        lexicon: &Lexicon,
        states: Vec<f32>,
        encoded: &Encoded,
    ) -> Result<Vec<Vec<f32>>> {
        let hidden = Tensor::from_vec(states, (encoded.ids.len(), lexicon.hidden()), &Device::Cpu)?;
        let lexical = lexicon.rows(&option_ids(&encoded.ids, &encoded.fields))?;
        self.forward(hidden, &encoded.fields, &lexical)
    }
}

/// x / max(‖x‖₂, eps) per row: F.normalize (eps 1e-12) and each side of
/// F.cosine_similarity (1e-8).
fn l2(x: &Tensor, eps: f64) -> candle_core::Result<Tensor> {
    x.broadcast_div(&x.sqr()?.sum_keepdim(1)?.sqrt()?.maximum(eps)?)
}

/// Token ids of every option span, options in order: the rows `forward` takes as `lexical`.
fn option_ids(ids: &[u32], fields: &[Field]) -> Vec<u32> {
    fields
        .iter()
        .flat_map(|f| &f.options)
        .flat_map(|r| ids[r.clone()].iter().copied())
        .collect()
}
