//! Port of laya's `render_options` + `build_sequence`:
//! `[CLS] "<type> question: <ins>" [SEP] [MASK] opt0 [MASK] opt1 … [SEP] <state> [SEP]`.
use anyhow::{anyhow, Result};
use serde_json::Value;
use tokenizers::Tokenizer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QType {
    Choice = 0,
    Score = 1,
    Noul = 2,
}
impl QType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Noul => "noul",
        }
    }
}

/// One question already rendered to option texts (label-index order).
pub struct Question {
    pub qtype: QType,
    pub instructions: String,
    pub options: Vec<String>,
}

/// `json.dumps(..., ensure_ascii=False)` with Python's default `", "` / `": "`
/// separators: the checkpoint was trained on that spelling, and every space is
/// a token.
struct PythonJson {
    first: Vec<bool>,
}
impl serde_json::ser::Formatter for PythonJson {
    fn begin_object<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.first.push(true);
        w.write_all(b"{")
    }
    fn end_object<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.first.pop();
        w.write_all(b"}")
    }
    fn begin_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(b": ")
    }
    fn begin_array<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(b"[")
    }
    fn end_array<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(b"]")
    }
    fn begin_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
}

pub fn python_json(value: &Value) -> String {
    let mut out = Vec::new();
    let mut ser =
        serde_json::Serializer::with_formatter(&mut out, PythonJson { first: Vec::new() });
    serde::Serialize::serialize(value, &mut ser).expect("json value serializes");
    String::from_utf8(out).expect("json is utf-8")
}

fn render_criterion(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => python_json(other),
    }
}

/// `render_options`: choice = "key: desc" (or key), score = "level i: desc", noul = false/true.
pub fn render_options(qtype: QType, criteria: Option<&Value>) -> Result<Vec<String>> {
    Ok(match qtype {
        QType::Choice => criteria
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("choice needs an object of criteria"))?
            .iter()
            .map(|(k, v)| match v {
                Value::Null => k.clone(),
                Value::String(s) if s.is_empty() => k.clone(),
                v => format!("{k}: {}", render_criterion(v)),
            })
            .collect(),
        QType::Score => criteria
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("score needs an array of levels"))?
            .iter()
            .enumerate()
            .map(|(i, c)| format!("level {i}: {}", render_criterion(c)))
            .collect(),
        QType::Noul => {
            let get = |key: &str| {
                criteria
                    .and_then(|c| c.get(key))
                    .filter(|v| !v.is_null() && v.as_str() != Some(""))
                    .map(render_criterion)
            };
            vec![
                format!(
                    "false: {}",
                    get("false").unwrap_or_else(|| "no, the statement does not hold".into())
                ),
                format!(
                    "true: {}",
                    get("true").unwrap_or_else(|| "yes, the statement holds".into())
                ),
            ]
        }
    })
}

pub fn serialize_state(state: &Value) -> String {
    match state {
        Value::String(s) => s.clone(),
        other => python_json(other),
    }
}

pub struct Sequence {
    pub ids: Vec<u32>,
    pub markers: Vec<usize>,
    /// State tokens the window could not hold (right-truncated, like laya).
    pub state_dropped: usize,
}

pub struct Encoder {
    tok: Tokenizer,
    cls: u32,
    sep: u32,
    mask: u32,
    pub pad: u32,
    mask_text: String,
    pub max_len: usize,
    pub head_max_len: usize,
}

impl Encoder {
    /// Special tokens by role, as laya reads them from `tokenizer_config.json`:
    /// ModernBERT spells them `[CLS]`/`[SEP]`/`[MASK]`/`[PAD]`, mmBERT
    /// (`laya-multilingual`, Gemma tokenizer) `<bos>`/`<eos>`/`<mask>`/`<pad>`.
    pub fn new(tok: Tokenizer, max_len: usize, head_max_len: usize) -> Result<Self> {
        let role = |names: &[&str]| {
            names
                .iter()
                .find_map(|name| tok.token_to_id(name).map(|id| (id, name.to_string())))
                .ok_or_else(|| anyhow!("tokenizer lacks any of {names:?}"))
        };
        let (mask, mask_text) = role(&["[MASK]", "<mask>"])?;
        Ok(Self {
            cls: role(&["[CLS]", "<bos>", "<s>"])?.0,
            sep: role(&["[SEP]", "<eos>", "</s>"])?.0,
            mask,
            pad: role(&["[PAD]", "<pad>"])?.0,
            mask_text,
            tok,
            max_len,
            head_max_len,
        })
    }

    /// `[CLS] text [SEP]` capped at `max` tokens, like `tokenizer(text,
    /// truncation=True, max_length=max)`.
    pub fn encode_text(&self, text: &str, max: usize) -> Result<Vec<u32>> {
        let mut ids = self
            .tok
            .encode(text, true)
            .map_err(|e| anyhow!("tokenize: {e}"))?
            .get_ids()
            .to_vec();
        if ids.len() > max.max(2) {
            ids.truncate(max.max(2) - 1);
            ids.push(self.sep);
        }
        Ok(ids)
    }

    fn encode(&self, text: &str) -> Result<Vec<u32>> {
        Ok(self
            .tok
            .encode(text, false)
            .map_err(|e| anyhow!("tokenize: {e}"))?
            .get_ids()
            .to_vec())
    }

    /// `build_sequence` (default option order, right truncation of the state).
    pub fn build(&self, state: &Value, q: &Question) -> Result<Sequence> {
        let ins = q.instructions.replace(&self.mask_text, " ");
        let mut head = self.encode(&format!("{} question: {ins}", q.qtype.name()))?;
        let mut opts: Vec<Vec<u32>> = Vec::with_capacity(q.options.len());
        for opt in &q.options {
            let mut ids = vec![self.mask];
            let body = self.encode(&format!(" {}", opt.replace(&self.mask_text, " ")))?;
            ids.extend(body.into_iter().take(48));
            opts.push(ids);
        }
        let total: usize = opts.iter().map(Vec::len).sum();
        let mut opt_budget = self.head_max_len as isize - total as isize;
        if opt_budget < 16 {
            let per = ((self.head_max_len - 16) / opts.len().max(1)).max(4);
            for o in &mut opts {
                o.truncate(per);
            }
            let total: usize = opts.iter().map(Vec::len).sum();
            opt_budget = self.head_max_len as isize - total as isize;
        }
        head.truncate((opt_budget.max(8)) as usize);
        let mut ids = vec![self.cls];
        ids.extend(head);
        ids.push(self.sep);
        let mut markers = Vec::with_capacity(opts.len());
        for o in opts {
            markers.push(ids.len());
            ids.extend(o);
        }
        ids.push(self.sep);
        let room = self.max_len.saturating_sub(ids.len() + 1);
        let mut st = self.encode(&serialize_state(state).replace(&self.mask_text, " "))?;
        let state_dropped = st.len().saturating_sub(room);
        st.truncate(room);
        ids.extend(st);
        ids.push(self.sep);
        ids.truncate(self.max_len);
        markers.retain(|&m| m < self.max_len);
        Ok(Sequence {
            ids,
            markers,
            state_dropped,
        })
    }
}
