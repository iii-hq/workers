//! Clef on llama.cpp's own `clef` arch (b11379, built by build.rs) through
//! native/clef.cpp: the backbone and the joint decision head run in one graph,
//! one score per option. Each call creates a context sized to its prompt and
//! frees it, so between calls only the model holds memory.
use crate::encode::Encoded;
use anyhow::{anyhow, ensure, Result};
use std::{
    ffi::{c_char, CStr, CString},
    path::Path,
    ptr::{self, NonNull},
    sync::Once,
};

/// llama.cpp's `llama_model`, opaque.
#[repr(C)]
struct RawModel {
    _private: [u8; 0],
}

extern "C" {
    // native/clef.cpp
    fn clef_backend_init(dir: *const c_char, fallback: *const c_char);
    fn clef_gpu_device(out: *mut c_char, len: usize) -> bool;
    fn clef_model_load(path: *const c_char, gpu_layers: i32) -> *mut RawModel;
    fn clef_decide(
        model: *const RawModel,
        n_threads: i32,
        ids: *const i32,
        orders: *const u8,
        n_tokens: i32,
        scores: *mut f32,
        n_scores: i32,
    ) -> i32;
    // llama.h
    fn llama_model_free(model: *mut RawModel);
    fn llama_model_meta_val_str(
        model: *const RawModel,
        key: *const c_char,
        buf: *mut c_char,
        buf_size: usize,
    ) -> i32;
}

/// llama.cpp's backends are process-global: load them once. Linux x86_64
/// builds them as modules (build.rs sets `CLEF_BACKENDS_DIR`): from the
/// executable's directory (the published layout), else from the build's
/// output (tests run from deps/). Elsewhere they are linked in.
fn init_backends() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let Some(fallback) = option_env!("CLEF_BACKENDS_DIR") else {
            unsafe { clef_backend_init(ptr::null(), ptr::null()) };
            return;
        };
        let dir = std::env::current_exe()
            .ok()
            .and_then(|exe| CString::new(exe.parent()?.as_os_str().as_encoded_bytes()).ok());
        let fallback = CString::new(fallback).expect("no NUL in a path");
        unsafe {
            clef_backend_init(
                dir.as_ref().map_or(ptr::null(), |dir| dir.as_ptr()),
                fallback.as_ptr(),
            )
        };
    });
}

/// A loaded Clef GGUF, freed on drop.
pub struct Model {
    raw: NonNull<RawModel>,
    threads: i32,
    /// The device running the model ("CPU" or the GPU's description).
    pub device: String,
}

// SAFETY: a loaded llama_model has no thread affinity and `decide` only reads
// it. Not `Sync`: concurrent contexts over one model on the Vulkan backend are
// unverified, so callers serialize (a Mutex or one thread).
unsafe impl Send for Model {}

impl Model {
    /// Load a GGUF of arch `clef`. `gpu_layers` None offloads every layer when
    /// a GPU device exists; Some(0) keeps llama.cpp off every GPU.
    pub fn load(gguf: &Path, gpu_layers: Option<u32>, threads: usize) -> Result<Self> {
        init_backends();
        let mut text = [0 as c_char; 256];
        let gpu = unsafe { clef_gpu_device(text.as_mut_ptr(), text.len()) }.then(|| {
            unsafe { CStr::from_ptr(text.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        });
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
        let raw = NonNull::new(unsafe { clef_model_load(path.as_ptr(), layers) })
            .ok_or_else(|| anyhow!("load {}: llama.cpp could not load it", gguf.display()))?;
        let model = Self {
            raw,
            threads: i32::try_from(threads).unwrap_or(8),
            device,
        };
        // A qwen35 GGUF of the backbone loads too, without the decision head.
        let mut arch = [0 as c_char; 64];
        let found = unsafe {
            llama_model_meta_val_str(
                model.raw.as_ptr(),
                c"general.architecture".as_ptr(),
                arch.as_mut_ptr(),
                arch.len(),
            )
        } >= 0;
        let arch = found.then(|| unsafe { CStr::from_ptr(arch.as_ptr()) }.to_string_lossy());
        ensure!(
            arch.as_deref() == Some("clef"),
            "{}: architecture {arch:?}, not clef",
            gguf.display()
        );
        Ok(model)
    }

    /// One forward over the prompt: the score of every option of every field,
    /// in prompt order (softmax per field). Blocks for the whole pass; nothing
    /// cancels it.
    pub fn decide(&self, encoded: &Encoded) -> Result<Vec<f32>> {
        // llama_decision_order: question noul 1, choice 2, score 3; option 4.
        let mut orders = vec![0u8; encoded.ids.len()];
        for field in &encoded.fields {
            orders[field.span.clone()].fill(field.kind as u8 + 1);
            for option in &field.options {
                orders[option.clone()].fill(4);
            }
        }
        let ids: Vec<i32> = encoded.ids.iter().map(|&id| id as i32).collect();
        let mut scores = vec![0f32; encoded.fields.iter().map(|f| f.options.len()).sum()];
        // SAFETY: `ids` and `orders` hold n_tokens entries, `scores` n_scores.
        let status = unsafe {
            clef_decide(
                self.raw.as_ptr(),
                self.threads,
                ids.as_ptr(),
                orders.as_ptr(),
                i32::try_from(ids.len())?,
                scores.as_mut_ptr(),
                i32::try_from(scores.len())?,
            )
        };
        ensure!(status == 0, "llama.cpp: clef_decide returned {status}");
        Ok(scores)
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { llama_model_free(self.raw.as_ptr()) }
    }
}
