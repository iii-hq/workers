//! llama.cpp b11379, built from source by build.rs (cmake, no bindgen), through
//! the C shim in native/shim.cpp and llama.h functions declared by hand:
//! backend init, model loading, tokenization, and two ways to run a model.
//! [`Model::decide`] is the ordered-decision forward (the `clef` arch's head
//! in the graph, one score per option) in a context sized to its prompt and
//! freed after it. [`Model::new_context`] keeps a [`Context`] for the callers
//! that drive llama.cpp themselves: batches of tokens over sequences
//! ([`Batch`]), logits or embeddings per output row, sequence state snapshots.
use anyhow::{anyhow, bail, ensure, Result};
use std::{
    collections::HashMap,
    ffi::{c_char, CStr, CString},
    path::Path,
    ptr::{self, NonNull},
    slice,
    sync::{
        atomic::{AtomicU64, Ordering},
        Once,
    },
};

/// `LLAMA_STATE_SEQ_FLAGS_ON_DEVICE`.
const STATE_ON_DEVICE: u32 = 2;

/// llama.cpp's `llama_model`, opaque.
#[repr(C)]
struct RawModel {
    _private: [u8; 0],
}

/// llama.cpp's `llama_context`, opaque.
#[repr(C)]
struct RawContext {
    _private: [u8; 0],
}

/// llama.cpp's `llama_vocab`, opaque.
#[repr(C)]
struct RawVocab {
    _private: [u8; 0],
}

/// llama.cpp's `llama_memory_i`, opaque.
#[repr(C)]
struct RawMemory {
    _private: [u8; 0],
}

extern "C" {
    // native/shim.cpp
    fn ln_backend_init(dir: *const c_char, fallback: *const c_char, vk_max_nodes_per_submit: u32);
    fn ln_gpu_device(out: *mut c_char, len: usize) -> bool;
    fn ln_model_load(path: *const c_char, gpu_layers: i32, use_gpu_devices: bool) -> *mut RawModel;
    fn ln_decide(
        model: *const RawModel,
        n_threads: i32,
        ids: *const i32,
        orders: *const u8,
        n_tokens: i32,
        scores: *mut f32,
        n_scores: i32,
    ) -> i32;
    fn ln_context_new(
        model: *mut RawModel,
        n_ctx: u32,
        n_batch: u32,
        n_ubatch: u32,
        n_seq_max: u32,
        n_threads: i32,
        embeddings: bool,
        pooling_type: i32,
        kv_unified: bool,
    ) -> *mut RawContext;
    fn ln_process(
        ctx: *mut RawContext,
        encode: bool,
        tokens: *const i32,
        pos: *const i32,
        seq: *const i32,
        output: *const i8,
        orders: *const u8,
        n_tokens: i32,
    ) -> i32;
    // llama.h
    fn llama_model_free(model: *mut RawModel);
    fn llama_model_meta_val_str(
        model: *const RawModel,
        key: *const c_char,
        buf: *mut c_char,
        buf_size: usize,
    ) -> i32;
    fn llama_model_n_embd_out(model: *const RawModel) -> i32;
    fn llama_model_get_vocab(model: *const RawModel) -> *const RawVocab;
    fn llama_vocab_n_tokens(vocab: *const RawVocab) -> i32;
    fn llama_tokenize(
        vocab: *const RawVocab,
        text: *const c_char,
        text_len: i32,
        tokens: *mut i32,
        n_tokens_max: i32,
        add_special: bool,
        parse_special: bool,
    ) -> i32;
    fn llama_free(ctx: *mut RawContext);
    fn llama_n_ctx(ctx: *const RawContext) -> u32;
    fn llama_n_batch(ctx: *const RawContext) -> u32;
    fn llama_n_seq_max(ctx: *const RawContext) -> u32;
    fn llama_get_logits_ith(ctx: *mut RawContext, i: i32) -> *mut f32;
    fn llama_get_embeddings_ith(ctx: *mut RawContext, i: i32) -> *mut f32;
    fn llama_get_memory(ctx: *const RawContext) -> *mut RawMemory;
    fn llama_memory_clear(mem: *mut RawMemory, data: bool);
    fn llama_state_seq_get_size_ext(ctx: *mut RawContext, seq_id: i32, flags: u32) -> usize;
    fn llama_state_seq_get_data_ext(
        ctx: *mut RawContext,
        dst: *mut u8,
        size: usize,
        seq_id: i32,
        flags: u32,
    ) -> usize;
    fn llama_state_seq_set_data_ext(
        ctx: *mut RawContext,
        src: *const u8,
        size: usize,
        seq_id: i32,
        flags: u32,
    ) -> usize;
}

/// Load llama.cpp's backends, once per process: later calls do nothing, and
/// [`gpu_device`] and [`Model::load`] make the first call with `None` when no
/// one has. Linux x86_64 builds them as modules (build.rs sets
/// `LLAMA_NATIVE_BACKENDS_DIR`): from the executable's directory (the
/// published layout), else from the build's output (tests run from deps/).
/// Elsewhere they are linked in. `vk_max_nodes_per_submit` sets
/// `GGML_VK_MAX_NODES_PER_SUBMIT` unless the operator did; `None` leaves the
/// environment alone.
pub fn init_backends(vk_max_nodes_per_submit: Option<u32>) {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let nodes = vk_max_nodes_per_submit.unwrap_or(0);
        let Some(fallback) = option_env!("LLAMA_NATIVE_BACKENDS_DIR") else {
            unsafe { ln_backend_init(ptr::null(), ptr::null(), nodes) };
            return;
        };
        let dir = std::env::current_exe()
            .ok()
            .and_then(|exe| CString::new(exe.parent()?.as_os_str().as_encoded_bytes()).ok());
        let fallback = CString::new(fallback).expect("no NUL in a path");
        unsafe {
            ln_backend_init(
                dir.as_ref().map_or(ptr::null(), |dir| dir.as_ptr()),
                fallback.as_ptr(),
                nodes,
            )
        };
    });
}

/// "<description> (<backend>)" of the first device that is not the CPU.
pub fn gpu_device() -> Option<String> {
    init_backends(None);
    let mut text = [0 as c_char; 256];
    unsafe { ln_gpu_device(text.as_mut_ptr(), text.len()) }.then(|| {
        unsafe { CStr::from_ptr(text.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

/// A loaded GGUF, freed on drop.
pub struct Model {
    raw: NonNull<RawModel>,
    threads: i32,
    /// The device running the model ("CPU" or the GPU's description).
    pub device: String,
}

// SAFETY: a loaded llama_model has no thread affinity, and contexts only read
// it. Not `Sync`: concurrent contexts over one model on the Vulkan backend are
// unverified, so callers serialize (a Mutex or one thread).
unsafe impl Send for Model {}

impl Model {
    /// Load a GGUF. `gpu_layers` None offloads every layer when a GPU device
    /// exists, Some(n) that many. With Some(0), `keep_gpu_devices` false
    /// hides the GPU devices from llama.cpp altogether; true leaves them
    /// visible, as llama-cpp-2 does (CPU weights then stay in the GPU's
    /// pinned host buffer, unrepacked, and large batch matmuls go to it).
    pub fn load(
        gguf: &Path,
        gpu_layers: Option<u32>,
        threads: usize,
        keep_gpu_devices: bool,
    ) -> Result<Self> {
        let gpu = gpu_device();
        let layers = match gpu_layers {
            None if gpu.is_some() => -1,
            None => 0,
            Some(n) => i32::try_from(n).unwrap_or(-1),
        };
        let device = match gpu {
            Some(gpu) if layers != 0 => gpu,
            _ => "CPU".into(),
        };
        let path = CString::new(gguf.as_os_str().as_encoded_bytes())?;
        let raw = NonNull::new(unsafe {
            ln_model_load(path.as_ptr(), layers, layers != 0 || keep_gpu_devices)
        })
        .ok_or_else(|| anyhow!("load {}: llama.cpp could not load it", gguf.display()))?;
        Ok(Self {
            raw,
            threads: i32::try_from(threads).unwrap_or(8),
            device,
        })
    }

    /// The GGUF's `general.architecture`.
    pub fn architecture(&self) -> Option<String> {
        self.meta("general.architecture")
    }

    /// The GGUF metadata value at `key`, as text.
    pub fn meta(&self, key: &str) -> Option<String> {
        let key = CString::new(key).ok()?;
        let raw = self.raw.as_ptr();
        // snprintf's contract: the full length, whatever fits the buffer.
        let len = unsafe { llama_model_meta_val_str(raw, key.as_ptr(), ptr::null_mut(), 0) };
        let len = usize::try_from(len).ok()?;
        let mut text = vec![0u8; len + 1];
        unsafe {
            llama_model_meta_val_str(raw, key.as_ptr(), text.as_mut_ptr().cast(), text.len())
        };
        text.truncate(len);
        Some(String::from_utf8_lossy(&text).into_owned())
    }

    /// Tokens in the vocabulary (the length of a logits row).
    pub fn n_vocab(&self) -> usize {
        unsafe { llama_vocab_n_tokens(llama_model_get_vocab(self.raw.as_ptr())) as usize }
    }

    /// Width of an embeddings row (the hidden width unless the graph ends in a head).
    pub fn n_embd_out(&self) -> usize {
        unsafe { llama_model_n_embd_out(self.raw.as_ptr()) as usize }
    }

    /// `text`'s token ids as llama-cpp-2's `str_to_token` gives them: special
    /// tokens written in the text are parsed, a NUL is refused, and
    /// `add_special` adds the BOS/EOS the vocabulary asks for (AddBos::Always).
    pub fn tokenize(&self, text: &str, add_special: bool) -> Result<Vec<i32>> {
        ensure!(!text.contains('\0'), "text holds a NUL byte");
        let vocab = unsafe { llama_model_get_vocab(self.raw.as_ptr()) };
        let len = i32::try_from(text.len())?;
        let mut ids = vec![0; (text.len() / 2 + usize::from(add_special)).max(8)];
        for _ in 0..2 {
            // SAFETY: `ids` holds n_tokens_max entries; llama.cpp copies `text`.
            let n = unsafe {
                llama_tokenize(
                    vocab,
                    text.as_ptr().cast(),
                    len,
                    ids.as_mut_ptr(),
                    i32::try_from(ids.len())?,
                    add_special,
                    true,
                )
            };
            // A negative count is the size it needs (i32::MIN: too many).
            match usize::try_from(n) {
                Ok(n) => {
                    ids.truncate(n);
                    return Ok(ids);
                }
                Err(_) if n != i32::MIN => ids.resize(n.unsigned_abs() as usize, 0),
                Err(_) => break,
            }
        }
        bail!("llama.cpp could not tokenize {} bytes", text.len())
    }

    /// A context over this model, freed on drop.
    pub fn new_context(&self, params: ContextParams) -> Result<Context<'_>> {
        let raw = unsafe {
            ln_context_new(
                self.raw.as_ptr(),
                params.n_ctx,
                params.n_batch,
                params.n_ubatch,
                params.n_seq_max,
                params.threads,
                params.embeddings,
                params.pooling as i32,
                params.kv_unified,
            )
        };
        let raw = NonNull::new(raw)
            .ok_or_else(|| anyhow!("llama.cpp could not create a context ({params:?})"))?;
        Ok(Context {
            raw,
            model: self,
            device_states: HashMap::new(),
        })
    }

    /// One forward over the prompt: `orders[i]` is token i's
    /// `llama_decision_order` (0 none; question noul 1, choice 2, score 3;
    /// option 4), and the result holds the score of each of the `n_scores`
    /// options (runs of 4) in prompt order. Blocks for the whole pass;
    /// nothing cancels it.
    pub fn decide(&self, ids: &[i32], orders: &[u8], n_scores: usize) -> Result<Vec<f32>> {
        ensure!(
            orders.len() == ids.len(),
            "{} orders for {} tokens",
            orders.len(),
            ids.len()
        );
        let mut scores = vec![0f32; n_scores];
        // SAFETY: `ids` and `orders` hold n_tokens entries, `scores` n_scores.
        let status = unsafe {
            ln_decide(
                self.raw.as_ptr(),
                self.threads,
                ids.as_ptr(),
                orders.as_ptr(),
                i32::try_from(ids.len())?,
                scores.as_mut_ptr(),
                i32::try_from(n_scores)?,
            )
        };
        ensure!(status == 0, "llama.cpp: ln_decide returned {status}");
        Ok(scores)
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { llama_model_free(self.raw.as_ptr()) }
    }
}

/// llama.h's `llama_pooling_type`: how an embeddings context pools a
/// sequence's rows (`None` keeps one row per token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pooling {
    /// The GGUF's own.
    Unspecified = -1,
    None = 0,
    Mean = 1,
    Cls = 2,
    Last = 3,
    Rank = 4,
}

/// The `llama_context_params` a [`Context`] is created with; the fields not
/// here keep llama.cpp's defaults. `Default` is llama.cpp's at b11379.
#[derive(Clone, Copy, Debug)]
pub struct ContextParams {
    /// The window in tokens, shared by the sequences (0: the model's own).
    pub n_ctx: u32,
    /// The most tokens one decode takes.
    pub n_batch: u32,
    /// Tokens per micro-batch (at most n_batch). An encode, or a context
    /// without memory or causal attention, takes one micro-batch at most.
    pub n_ubatch: u32,
    /// Sequences: ids 0..n_seq_max.
    pub n_seq_max: u32,
    pub threads: i32,
    /// Keep embeddings rather than logits: native/llama-output-reserve.patch
    /// leaves such a context without logits.
    pub embeddings: bool,
    pub pooling: Pooling,
    /// One KV pool for every sequence instead of n_ctx / n_seq_max each.
    pub kv_unified: bool,
}

impl Default for ContextParams {
    fn default() -> Self {
        Self {
            n_ctx: 512,
            n_batch: 2048,
            n_ubatch: 512,
            n_seq_max: 1,
            threads: 4,
            embeddings: false,
            pooling: Pooling::Unspecified,
            kv_unified: false,
        }
    }
}

/// Tokens for one [`Context::decode`] or [`Context::encode`].
#[derive(Default)]
pub struct Batch {
    tokens: Vec<i32>,
    pos: Vec<i32>,
    seq: Vec<i32>,
    output: Vec<i8>,
    orders: Vec<u8>,
}

impl Batch {
    /// `token` at position `pos` of sequence `seq`. `output` keeps its logits
    /// (or embeddings) for reading at this row's index in the batch.
    pub fn add(&mut self, token: i32, pos: i32, seq: i32, output: bool) {
        self.add_ordered(token, pos, seq, output, 0);
    }

    /// [`Self::add`] with the token's `llama_decision_order` (as in
    /// [`Model::decide`]). A laya decision head reads each token's question
    /// type from it (1 noul, 2 choice, 3 score on every token of a row) and
    /// runs once instead of once per type.
    pub fn add_ordered(&mut self, token: i32, pos: i32, seq: i32, output: bool, order: u8) {
        self.tokens.push(token);
        self.pos.push(pos);
        self.seq.push(seq);
        self.output.push(i8::from(output));
        self.orders.push(order);
    }
}

/// A sequence's memory, from [`Context::state_seq_get`]: only llama.cpp
/// writes these bytes.
#[derive(Clone)]
pub struct SeqState {
    bytes: Vec<u8>,
    /// On the device: the sequence it came from and its snapshot id. The
    /// bytes then only describe the tensors, which stay in its context.
    device: Option<(i32, u64)>,
}

/// A llama.cpp context over a [`Model`]: its memory (KV cache, recurrent
/// state) and the outputs of its last pass. Freed on drop; not `Send`.
pub struct Context<'m> {
    raw: NonNull<RawContext>,
    model: &'m Model,
    /// Per sequence, the id of the on-device snapshot llama.cpp still holds.
    device_states: HashMap<i32, u64>,
}

impl Context<'_> {
    /// The window in tokens (llama.cpp pads the one asked for).
    pub fn n_ctx(&self) -> u32 {
        unsafe { llama_n_ctx(self.raw.as_ptr()) }
    }

    pub fn n_batch(&self) -> u32 {
        unsafe { llama_n_batch(self.raw.as_ptr()) }
    }

    /// Run `batch` through the model into the sequences' memory. Errs when
    /// llama.cpp refuses the batch or fails the pass; a bad batch never
    /// aborts the process (`ln_process`).
    pub fn decode(&mut self, batch: &Batch) -> Result<()> {
        self.process(batch, false)
    }

    /// Run `batch` as one non-causal micro-batch (encoders, decision heads).
    /// Errs on a context with memory (KV cache): decode there.
    pub fn encode(&mut self, batch: &Batch) -> Result<()> {
        self.process(batch, true)
    }

    fn process(&mut self, batch: &Batch, encode: bool) -> Result<()> {
        // Only llama_batch_ext carries orders: a batch without any takes llama_batch.
        let orders = if batch.orders.iter().any(|&order| order != 0) {
            batch.orders.as_ptr()
        } else {
            ptr::null()
        };
        // SAFETY: the five vectors hold one entry per token.
        let status = unsafe {
            ln_process(
                self.raw.as_ptr(),
                encode,
                batch.tokens.as_ptr(),
                batch.pos.as_ptr(),
                batch.seq.as_ptr(),
                batch.output.as_ptr(),
                orders,
                i32::try_from(batch.tokens.len())?,
            )
        };
        ensure!(status == 0, "llama.cpp: ln_process returned {status}");
        Ok(())
    }

    /// The logits of batch row `i` of the last pass (negative: from the last
    /// output back), `n_vocab` of them; None unless that row was an output.
    pub fn logits_ith(&self, i: i32) -> Option<&[f32]> {
        let row = unsafe { llama_get_logits_ith(self.raw.as_ptr(), i) };
        (!row.is_null()).then(|| unsafe { slice::from_raw_parts(row, self.model.n_vocab()) })
    }

    /// The embeddings of batch row `i` of the last pass, `n_embd_out` of them;
    /// None unless that row was an output of an embeddings context.
    pub fn embeddings_ith(&self, i: i32) -> Option<&[f32]> {
        let row = unsafe { llama_get_embeddings_ith(self.raw.as_ptr(), i) };
        (!row.is_null()).then(|| unsafe { slice::from_raw_parts(row, self.model.n_embd_out()) })
    }

    /// Empty every sequence's memory.
    pub fn clear_kv_cache(&mut self) {
        unsafe { llama_memory_clear(llama_get_memory(self.raw.as_ptr()), true) }
    }

    /// Snapshot sequence `seq`'s memory. A host snapshot holds the data and
    /// restores into any context shaped like this one. `on_device` keeps the
    /// data in a device-side copy this context owns (no round trip through
    /// host memory): that snapshot restores only here, and only until the
    /// next on-device snapshot of `seq` replaces the copy.
    pub fn state_seq_get(&mut self, seq: i32, on_device: bool) -> Result<SeqState> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        self.check_seq(seq)?;
        let flags = if on_device { STATE_ON_DEVICE } else { 0 };
        // Even a failed snapshot may have replaced the device copy.
        if on_device {
            self.device_states.remove(&seq);
        }
        let raw = self.raw.as_ptr();
        let size = unsafe { llama_state_seq_get_size_ext(raw, seq, flags) };
        let mut bytes = vec![0u8; size];
        let written =
            unsafe { llama_state_seq_get_data_ext(raw, bytes.as_mut_ptr(), size, seq, flags) };
        ensure!(
            size > 0 && written == size,
            "llama.cpp: state of sequence {seq}: {written} of {size} bytes"
        );
        let device = on_device.then(|| {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            self.device_states.insert(seq, id);
            (seq, id)
        });
        Ok(SeqState { bytes, device })
    }

    /// Restore `state` into sequence `seq` (any sequence of a context shaped
    /// like the one it came from; this very context for an on-device one).
    pub fn state_seq_set(&mut self, state: &SeqState, seq: i32) -> Result<()> {
        self.check_seq(seq)?;
        let flags = match state.device {
            None => 0,
            // llama.cpp aborts on a device copy it does not hold, and would
            // read a newer one of the same size without noticing.
            Some((from, id)) => {
                ensure!(
                    self.device_states.get(&from) == Some(&id),
                    "on-device state of sequence {from} is gone"
                );
                STATE_ON_DEVICE
            }
        };
        let bytes = &state.bytes;
        let read = unsafe {
            llama_state_seq_set_data_ext(self.raw.as_ptr(), bytes.as_ptr(), bytes.len(), seq, flags)
        };
        ensure!(
            read == bytes.len(),
            "llama.cpp: restoring sequence {seq} read {read} of {} bytes",
            bytes.len()
        );
        Ok(())
    }

    /// llama.cpp asserts (aborts) on a sequence outside its memory.
    fn check_seq(&self, seq: i32) -> Result<()> {
        let n_seq_max = unsafe { llama_n_seq_max(self.raw.as_ptr()) };
        ensure!(
            u32::try_from(seq).is_ok_and(|seq| seq < n_seq_max),
            "sequence {seq} outside 0..{n_seq_max}"
        );
        Ok(())
    }
}

impl Drop for Context<'_> {
    fn drop(&mut self) {
        unsafe { llama_free(self.raw.as_ptr()) }
    }
}
