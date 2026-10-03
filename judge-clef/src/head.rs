//! Clef's joint schema head (`JointSchemaHead`) on candle, f32 on the CPU, and
//! the backbone `output.weight` rows its lexical prior reads. STUB: the
//! interface is fixed; the body is implemented in its own change.
use crate::encode::{Encoded, Field};
use anyhow::Result;
use candle_core::Tensor;
use std::path::Path;

/// output.weight (the untied lm_head) of the backbone GGUF, read row by row.
pub struct Lexicon;

impl Lexicon {
    /// Read the GGUF header and locate output.weight.
    pub fn open(gguf: &Path) -> Result<Self> {
        let _ = gguf;
        todo!("Lexicon::open")
    }

    /// The rows' width: the backbone's hidden size.
    pub fn hidden(&self) -> usize {
        todo!("Lexicon::hidden")
    }

    /// Rows of `ids` as f32 `[ids.len(), hidden]`, dequantized.
    pub fn rows(&self, ids: &[u32]) -> Result<Tensor> {
        let _ = ids;
        todo!("Lexicon::rows")
    }
}

pub struct Head;

impl Head {
    /// `weights`: joint_head.safetensors (bf16, read as f32); `config`:
    /// joint_head_config.json, whose hidden_size must equal `hidden`.
    pub fn load(weights: &Path, config: &Path, hidden: usize) -> Result<Self> {
        let _ = (weights, config, hidden);
        todo!("Head::load")
    }

    /// One logit per option of each field: `hidden` [L, H] final-norm states,
    /// `lexical` [T, H] the output.weight rows of `option_ids`.
    pub fn forward(&self, hidden: Tensor, fields: &[Field], lexical: &Tensor) -> Result<Vec<Vec<f32>>> {
        let _ = (hidden, fields, lexical);
        todo!("Head::forward")
    }

    /// `forward` on one encoded record: `states` is the engine's `[L × H]`.
    pub fn logits(&self, lexicon: &Lexicon, states: Vec<f32>, encoded: &Encoded) -> Result<Vec<Vec<f32>>> {
        let _ = (lexicon, states, encoded);
        todo!("Head::logits")
    }
}

/// Token ids of every option span, options in order: the rows `forward` takes as `lexical`.
pub fn option_ids(ids: &[u32], fields: &[Field]) -> Vec<u32> {
    fields
        .iter()
        .flat_map(|f| &f.options)
        .flat_map(|r| ids[r.clone()].iter().copied())
        .collect()
}
