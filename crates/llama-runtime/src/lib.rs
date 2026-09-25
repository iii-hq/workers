//! llama.cpp runtime shared by the in-process judge providers (judge-semif,
//! judge-decider, judge-laya): backend init and module loading, device
//! selection, model loading, and one thread that owns the `LlamaContext` (not
//! `Send`) and serves jobs in arrival order, so a single forward runs on the
//! hardware at a time. What a job does with the model is the provider's
//! business; `scorer` is the job of the providers that read option labels.
pub use llama_cpp_2;
pub mod scorer;

use anyhow::{anyhow, Result};
use llama_cpp_2::{
    context::{params::LlamaContextParams, LlamaContext},
    llama_backend::LlamaBackend,
    model::{params::LlamaModelParams, LlamaModel},
};
use std::{
    path::Path,
    sync::{mpsc, Arc, OnceLock},
};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
}

/// The loaded model and its context, owned by the runtime thread; both are
/// freed when the last `Runtime` handle is dropped and the thread ends.
pub struct Session<'a> {
    pub model: &'a LlamaModel,
    pub ctx: LlamaContext<'a>,
}

type Job = Box<dyn for<'a> FnOnce(&mut Session<'a>) + Send>;

/// Handle to the runtime thread; clones share it.
#[derive(Clone)]
pub struct Runtime {
    jobs: mpsc::Sender<Job>,
    device: Arc<str>,
}

/// llama.cpp's backend is process-global and initializes once.
fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            load_backend_modules();
            LlamaBackend::init().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow!("llama.cpp init: {e}"))
}

/// Linux x86_64 and Windows ship llama.cpp's backends as modules (Cargo.toml):
/// load them from the executable's directory (the published layout), else from
/// the build's own output (tests). A module whose system library is missing,
/// such as the Vulkan loader, is skipped, and the CPU module runs the model.
#[cfg(any(
    all(target_os = "linux", target_arch = "x86_64"),
    target_os = "windows"
))]
fn load_backend_modules() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        llama_cpp_2::llama_backend::load_backends_from_path(&dir);
    }
    if llama_cpp_2::list_llama_ggml_backend_devices().is_empty() {
        llama_cpp_2::llama_backend::load_backends();
    }
}

/// Elsewhere the backends are linked in (Metal on macOS, CPU otherwise).
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    target_os = "windows"
)))]
fn load_backend_modules() {}

impl Runtime {
    /// Load `gguf` on a new thread and return once it answers (or failed).
    /// `configure` sets the context parameters the provider needs (window,
    /// batch sizes, sequences, pooling); the thread count comes from `options`.
    pub fn spawn(
        gguf: &Path,
        options: Options,
        configure: impl FnOnce(LlamaContextParams) -> LlamaContextParams + Send + 'static,
    ) -> Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready_tx, ready) = mpsc::channel();
        let gguf = gguf.to_path_buf();
        std::thread::Builder::new()
            .name("llama-runtime".into())
            .spawn(move || {
                let load = || -> Result<(LlamaModel, String)> {
                    let backend = backend()?;
                    let gpu = llama_cpp_2::list_llama_ggml_backend_devices()
                        .into_iter()
                        .find(|d| !d.backend.eq_ignore_ascii_case("cpu"));
                    let layers =
                        options
                            .gpu_layers
                            .unwrap_or(if gpu.is_some() { u32::MAX } else { 0 });
                    let device = match (&gpu, layers) {
                        (Some(d), n) if n > 0 => format!("{} ({})", d.description, d.backend),
                        _ => "CPU".into(),
                    };
                    let model = LlamaModel::load_from_file(
                        backend,
                        &gguf,
                        &LlamaModelParams::default().with_n_gpu_layers(layers),
                    )
                    .map_err(|e| anyhow!("load {}: {e}", gguf.display()))?;
                    Ok((model, device))
                };
                let (model, device) = match load() {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let threads = i32::try_from(options.threads).unwrap_or(8);
                let params = configure(
                    LlamaContextParams::default()
                        .with_n_threads(threads)
                        .with_n_threads_batch(threads),
                );
                // The context borrows the model; both live until the jobs end.
                let ctx = match backend().and_then(|backend| {
                    model
                        .new_context(backend, params)
                        .map_err(|e| anyhow!("context: {e}"))
                }) {
                    Ok(ctx) => ctx,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let mut session = Session { model: &model, ctx };
                let _ = ready_tx.send(Ok(device));
                for job in inbox {
                    job(&mut session);
                }
            })?;
        let device: String = ready
            .recv()
            .map_err(|_| anyhow!("llama.cpp runtime thread exited during load"))??;
        tracing::info!(device, "selected inference device");
        Ok(Self {
            jobs,
            device: device.into(),
        })
    }

    /// The device running the model ("CPU" or the GPU's description).
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Queue `job`; the receiver yields its result. Dropping the receiver does
    /// not stop the work: jobs observe their own deadline or cancel flag.
    pub fn submit<R: Send + 'static>(
        &self,
        job: impl for<'a> FnOnce(&mut Session<'a>) -> R + Send + 'static,
    ) -> oneshot::Receiver<R> {
        let (reply, receiver) = oneshot::channel();
        // A send error means the thread died: the dropped reply reports it.
        let _ = self.jobs.send(Box::new(move |session: &mut Session<'_>| {
            let _ = reply.send(job(session));
        }));
        receiver
    }

    /// Run `job` and wait for it (load-time setup; blocks the calling thread).
    pub fn run<R: Send + 'static>(
        &self,
        job: impl for<'a> FnOnce(&mut Session<'a>) -> R + Send + 'static,
    ) -> Result<R> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.jobs
            .send(Box::new(move |session: &mut Session<'_>| {
                let _ = reply.send(job(session));
            }))
            .map_err(|_| anyhow!("llama.cpp runtime thread exited"))?;
        receiver
            .recv()
            .map_err(|_| anyhow!("llama.cpp runtime thread exited"))
    }
}
