//! llama.cpp b11379, built from source by build.rs (cmake, no bindgen), through
//! the C shim in native/shim.cpp and a few llama.h functions declared by hand:
//! backend init, model loading and the ordered-decision forward (the `clef`
//! arch's head in the graph, one score per option). Each `decide` creates a
//! context sized to its prompt and frees it, so between calls only the model
//! holds memory.
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
    // native/shim.cpp
    fn ln_backend_init(dir: *const c_char, fallback: *const c_char, vk_max_nodes_per_submit: u32);
    fn ln_gpu_device(out: *mut c_char, len: usize) -> bool;
    fn ln_model_load(path: *const c_char, gpu_layers: i32) -> *mut RawModel;
    fn ln_decide(
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

// SAFETY: a loaded llama_model has no thread affinity and `decide` only reads
// it. Not `Sync`: concurrent contexts over one model on the Vulkan backend are
// unverified, so callers serialize (a Mutex or one thread).
unsafe impl Send for Model {}

impl Model {
    /// Load a GGUF. `gpu_layers` None offloads every layer when a GPU device
    /// exists; Some(0) uses no GPU device at all.
    pub fn load(gguf: &Path, gpu_layers: Option<u32>, threads: usize) -> Result<Self> {
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
        let raw = NonNull::new(unsafe { ln_model_load(path.as_ptr(), layers) })
            .ok_or_else(|| anyhow!("load {}: llama.cpp could not load it", gguf.display()))?;
        Ok(Self {
            raw,
            threads: i32::try_from(threads).unwrap_or(8),
            device,
        })
    }

    /// The GGUF's `general.architecture`.
    pub fn architecture(&self) -> Option<String> {
        let mut arch = [0 as c_char; 64];
        let found = unsafe {
            llama_model_meta_val_str(
                self.raw.as_ptr(),
                c"general.architecture".as_ptr(),
                arch.as_mut_ptr(),
                arch.len(),
            )
        } >= 0;
        found.then(|| {
            unsafe { CStr::from_ptr(arch.as_ptr()) }
                .to_string_lossy()
                .into_owned()
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
