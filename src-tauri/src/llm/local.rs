//! Local light LLM via llama.cpp (`llama-cpp-2` bindings).
//! When GGUF weights exist, runs real generation. Without weights, returns a clear error
//! for summarize/chat cloud-less paths (callers may fall back intentionally).

use crate::domain::backend::{built_backends, resolve_llm_backend, BackendSupport, ComputeBackend};
use crate::domain::chat::{offline_answer, ChatMessage};
use crate::domain::i18n::Locale;
use crate::domain::summary::{extractive_summary, MeetingInsights, SummaryTemplate};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::TokenToStringError;
use parking_lot::Mutex;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// llama.cpp's global init, done once per process.
///
/// It registers the ggml backends and installs a log handler; calling it twice is
/// an error in the C library, so it lives here rather than beside each load.
static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend, String> {
    BACKEND
        .get_or_init(|| {
            load_ggml_backends();
            LlamaBackend::init().map_err(|e| format!("llama backend init: {e}"))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// Point ggml at its backend libraries before anything asks for one.
///
/// With `dynamic-backends`, Vulkan, CUDA and *every CPU variant* are separate
/// shared libraries chosen at runtime — which is what lets one binary use the
/// GPU on a machine that has one and AVX-512 on a machine that does not. The
/// installed app keeps them beside the executable; a `cargo run` has them only
/// in the build tree, which is the path the crate baked in as `BACKENDS_DIR`.
///
/// Not gated on a GPU feature. Even a build with none has its CPU backends as
/// separate modules, so skipping this would leave llama.cpp with no backend at
/// all rather than merely without a GPU.
///
/// Loading nothing here does not fall back to a slow CPU path — there would be
/// no CPU backend either, and every model load would fail.
fn load_ggml_backends() {
    use llama_cpp_2::llama_backend::{load_backends_from_path, BACKENDS_DIR};

    // The installed layout wins over the build tree: a developer machine has
    // both, and the libraries the binary shipped with are the ones it was
    // tested against. Two candidates because the two layouts differ — Windows
    // puts resources beside the executable, Linux puts them in
    // `../lib/Vesper` — and asking both is cheaper than asking which platform
    // this is.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    // A backend the user downloaded, before anything else. It is not beside the
    // executable and ggml will never find it on its own.
    load_downloaded_backend();

    // ggml scans the executable's own directory by itself. Loading that same
    // directory again registers every backend twice, and a device listed twice
    // is a card that gets asked to hold half a model each time.
    if exe_dir.as_deref().is_some_and(holds_a_backend) {
        return;
    }
    // Anywhere else has to be named. On Linux the bundle puts them beside the
    // resources rather than beside the binary, and a `cargo run` has them only
    // in the build tree.
    let resources = exe_dir.map(|dir| dir.join("..").join("lib").join("Vesper"));
    match resources
        .filter(|dir| holds_a_backend(dir))
        .or_else(|| BACKENDS_DIR.map(PathBuf::from))
    {
        Some(dir) => load_backends_from_path(&dir),
        None => tracing::warn!("no ggml backend directory found; local models will not load"),
    }
}

/// Register the CUDA pack, if the user asked for it and it unpacked.
///
/// Two steps, and the second is the one that is easy to miss. `ggml-cuda.dll`
/// imports `cudart` and `cublas` at *load* time, and Windows resolves those
/// from the executable's directory, the system directories and `PATH` — never
/// from the directory of the library doing the importing. Without the first
/// step the file loads and immediately fails, which looks exactly like the
/// machine not having a GPU.
///
/// Prepending to the process's own `PATH` rather than calling
/// `SetDllDirectory`: it is inherited by everything ggml opens afterwards, and
/// it does not disturb the default search order for anything else.
fn load_downloaded_backend() {
    use llama_cpp_2::llama_backend::load_backends_from_path;

    let dir = crate::paths::backends_dir().join("cuda-backend");
    if !dir.join("ggml-cuda.dll").is_file() {
        return;
    }
    if let Some(path) = std::env::var_os("PATH") {
        let mut entries = vec![dir.clone()];
        entries.extend(std::env::split_paths(&path));
        match std::env::join_paths(entries) {
            // SAFETY: called once, from inside the `OnceLock` that guards the
            // backend, before any thread can be looking at the environment.
            Ok(joined) => unsafe { std::env::set_var("PATH", joined) },
            Err(e) => {
                tracing::warn!("could not put the CUDA pack on PATH: {e}");
                return;
            }
        }
    }
    tracing::info!("loading the downloaded CUDA backend from {}", dir.display());
    load_backends_from_path(&dir);
}

/// Whether a directory holds ggml's loadable backends.
///
/// Probing for a CPU variant and not for `ggml-vulkan`: the CPU ones are always
/// built when dynamic backends are on, so their absence means the directory is
/// the wrong one rather than that the machine has no GPU.
fn holds_a_backend(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("ggml-cpu"))
    })
}

/// What this machine offers, asked of ggml rather than guessed at.
///
/// Asking ggml is what makes this work for hardware no vendor tool reports: an
/// Intel Arc has no `nvidia-smi` to interrogate, and a driver that is installed
/// but broken answers here the same way it will answer during a real load.
///
/// It lives beside the llama code because loading those backend libraries is
/// what makes a device list exist at all. whisper links its own ggml statically
/// and exposes no equivalent enumeration, so it reads this one — the same
/// machine and the same drivers, seen through the copy that can be asked.
///
/// Memoised: the answer cannot change while the process runs, and probing it is
/// a walk over every registered backend.
pub(crate) fn probe_support() -> BackendSupport {
    static SUPPORT: OnceLock<BackendSupport> = OnceLock::new();
    *SUPPORT.get_or_init(|| {
        let (cuda_built, vulkan_built) = built_backends();
        let mut support = BackendSupport {
            cuda_built,
            vulkan_built,
            ..Default::default()
        };
        // No initialised backend means no device list to read. Claiming a GPU
        // from the build flags alone would be a guess, and the cost of guessing
        // wrong is a load that fails instead of a transcript that is slow.
        if backend().is_err() {
            return support;
        }
        for device in llama_cpp_2::list_llama_ggml_backend_devices() {
            match device.backend.as_str() {
                // A CUDA device in this list means the CUDA backend registered,
                // and it registers whether it was compiled into the binary or
                // downloaded afterwards. `built_backends` cannot know about the
                // second — it reads compile-time flags — so a machine that
                // fetched the pack would have been told its own GPU was
                // unreachable and quietly kept using Vulkan.
                "CUDA" => {
                    support.cuda_present = true;
                    support.cuda_built = true;
                }
                "Vulkan" => support.vulkan_present = true,
                _ => {}
            }
        }
        support
    })
}

/// One GPU as ggml sees it.
#[derive(Debug, Clone)]
pub(crate) struct GpuDevice {
    pub backend: String,
    pub description: String,
    pub total_bytes: u64,
    /// False for an integrated chip, whose reported memory is a slice of system
    /// RAM rather than memory of its own.
    pub discrete: bool,
}

/// The GPUs this build can actually reach, with the memory each reports.
///
/// This is what capability reporting asks, and it is a different question from
/// "is there a graphics card in this machine". A card the build has no backend
/// for cannot hold a model, so it must not raise the recommended model size.
///
/// Filtered to GPU devices: ggml registers the CPU as a device too, and its
/// reported memory is the machine's RAM — which would read as an enormous
/// amount of VRAM.
pub(crate) fn gpu_devices() -> Vec<GpuDevice> {
    use llama_cpp_2::LlamaBackendDeviceType;

    if backend().is_err() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    llama_cpp_2::list_llama_ggml_backend_devices()
        .into_iter()
        .filter(|device| {
            matches!(
                device.device_type,
                LlamaBackendDeviceType::Gpu | LlamaBackendDeviceType::IntegratedGpu
            )
        })
        // ggml can end up with a backend registered more than once, and then
        // every card it owns is listed once per registration. Observed on a
        // laptop that reported its two GPUs four times.
        //
        // By ggml's own name for the device — `Vulkan0`, `Vulkan1` — because
        // that is what separates two real cards from one card seen twice. The
        // description does not: two identical cards share it.
        .filter(|device| seen.insert((device.backend.clone(), device.name.clone())))
        .map(|device| GpuDevice {
            discrete: matches!(device.device_type, LlamaBackendDeviceType::Gpu),
            backend: device.backend,
            description: device.description,
            total_bytes: device.memory_total as u64,
        })
        .collect()
}

/// Every layer on the GPU, and only on the GPU that was chosen.
///
/// A build carrying both backends registers one physical card twice — once as
/// CUDA and once as Vulkan — and llama.cpp's default of "use the GPU devices"
/// would split a single model across two views of the same hardware.
fn gpu_params(chosen: ComputeBackend) -> Result<LlamaModelParams, String> {
    use llama_cpp_2::LlamaBackendDeviceType;

    let mut seen = std::collections::HashSet::new();
    let all: Vec<_> = llama_cpp_2::list_llama_ggml_backend_devices()
        .into_iter()
        // Same duplicate registrations as in `gpu_devices`, and here they would
        // do real damage: llama.cpp would split one model across two views of a
        // single card, each believing it has the whole of that card's memory.
        .filter(|device| seen.insert((device.backend.clone(), device.name.clone())))
        .collect();
    let matching = |device: &llama_cpp_2::LlamaBackendDevice| {
        device.backend.eq_ignore_ascii_case(chosen.as_str())
    };
    // Discrete cards if there are any. Splitting a model between a real GPU and
    // the integrated chip sharing the machine's RAM is slower than either alone.
    let discrete: Vec<usize> = all
        .iter()
        .filter(|d| matching(d) && matches!(d.device_type, LlamaBackendDeviceType::Gpu))
        .map(|d| d.index)
        .collect();
    let devices = if discrete.is_empty() {
        all.iter()
            .filter(|d| matching(d))
            .map(|d| d.index)
            .collect()
    } else {
        discrete
    };
    // Clamped to the layer count inside llama.cpp; asking for every layer is
    // how "offload all of it" is spelled.
    let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX);
    if devices.is_empty() {
        return Ok(params);
    }
    params
        .with_devices(&devices)
        .map_err(|e| format!("selecting the {} device: {e}", chosen.as_str()))
}

/// Load the weights onto the chosen backend, and onto the CPU when that fails.
///
/// A GPU load fails for reasons that are about this machine at this moment —
/// VRAM taken by a game, a driver that just reset — and none of them are a
/// reason to refuse to summarise a meeting. The log line is what makes the
/// difference findable later, when the symptom is only "this got slow".
fn load_model(
    backend: &LlamaBackend,
    model_path: &Path,
    chosen: ComputeBackend,
) -> Result<(LlamaModel, ComputeBackend), String> {
    let cpu = || {
        LlamaModel::load_from_file(backend, model_path, &LlamaModelParams::default())
            .map_err(|e| format!("llama load failed: {e}"))
            .map(|model| (model, ComputeBackend::Cpu))
    };
    if !chosen.is_gpu() {
        return cpu();
    }
    match LlamaModel::load_from_file(backend, model_path, &gpu_params(chosen)?) {
        Ok(model) => {
            tracing::info!("local LLM offloaded to {}", chosen.as_str());
            Ok((model, chosen))
        }
        Err(e) => {
            tracing::warn!(
                "{} load failed ({e}); using the CPU instead",
                chosen.as_str()
            );
            cpu()
        }
    }
}

static LLM_CACHE: OnceLock<Mutex<Option<(String, LlamaModel)>>> = OnceLock::new();

fn llm_cache() -> &'static Mutex<Option<(String, LlamaModel)>> {
    LLM_CACHE.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone)]
pub struct LocalLlm {
    models_dir: PathBuf,
    /// When true (default in production), missing model → extractive offline fallback.
    /// Tests can set false to assert hard errors.
    pub soft_fallback: bool,
}

impl LocalLlm {
    pub fn new() -> Self {
        Self {
            models_dir: crate::paths::models_dir(),
            soft_fallback: true,
        }
    }

    pub fn with_models_dir(dir: PathBuf) -> Self {
        Self {
            models_dir: dir,
            soft_fallback: true,
        }
    }

    pub fn model_file(&self, model_id: &str) -> PathBuf {
        self.models_dir.join(model_id).join("model.gguf")
    }

    /// Ready means "verified against the catalog digest" — llama.cpp parses this
    /// file, so an unverified GGUF must not reach it.
    pub fn is_ready(&self, model_id: &str) -> bool {
        crate::models::artifact_is_verified(&self.model_file(model_id), model_id)
    }

    pub fn summarize(
        &self,
        transcript: &str,
        template: SummaryTemplate,
        locale: Locale,
        model_id: &str,
        backend_preference: &str,
    ) -> Result<MeetingInsights, String> {
        if self.is_ready(model_id) {
            let prompt = crate::domain::summary::build_summary_prompt(template, transcript, locale);
            let raw = run_llama(&self.model_file(model_id), &prompt, 512, backend_preference)?;
            return Ok(MeetingInsights::from_model_text(&raw));
        }
        if self.soft_fallback {
            return Ok(extractive_summary(transcript, 4));
        }
        Err(format!(
            "local LLM model `{model_id}` is not installed — download GGUF weights from Settings"
        ))
    }

    /// One short completion for a meeting title.
    ///
    /// No `soft_fallback` branch, unlike the two below: the offline substitutes
    /// are keyword-matched transcript lines, which are a usable stopgap for a
    /// summary and garbage as a name. Without weights this is an error and the
    /// caller keeps the date label.
    pub fn title(
        &self,
        prompt: &str,
        model_id: &str,
        backend_preference: &str,
    ) -> Result<String, String> {
        if !self.is_ready(model_id) {
            return Err(format!(
                "local LLM model `{model_id}` is not installed — download GGUF weights from Settings"
            ));
        }
        run_llama(&self.model_file(model_id), prompt, 32, backend_preference)
    }

    pub fn chat(
        &self,
        messages: &[ChatMessage],
        transcript: &str,
        model_id: &str,
        backend_preference: &str,
    ) -> Result<String, String> {
        let question = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        if self.is_ready(model_id) {
            let system = messages
                .iter()
                .find(|m| m.role == "system")
                .map(|m| m.content.as_str())
                .unwrap_or("You are Vesper, a private meeting assistant.");
            let prompt = format!("{system}\n\nUser: {question}\nAssistant:");
            return run_llama(&self.model_file(model_id), &prompt, 384, backend_preference);
        }
        if self.soft_fallback {
            return Ok(offline_answer(transcript, question));
        }
        Err(format!(
            "local LLM model `{model_id}` is not installed — download GGUF weights from Settings"
        ))
    }
}

impl Default for LocalLlm {
    fn default() -> Self {
        Self::new()
    }
}

/// The bytes of one token, whatever its length.
///
/// `token_to_piece_bytes` writes into a buffer the caller sizes and returns an
/// error rather than a truncation when the piece does not fit. 32 bytes covers
/// ordinary text, but a single token can legitimately be longer — a long CJK
/// run, an emoji sequence — and losing the rest of a summary to one of them is
/// not a trade worth making. The failure carries the size that would have
/// worked, so the second attempt is exact rather than a guess.
fn piece(model: &LlamaModel, token: llama_cpp_2::token::LlamaToken) -> Result<Vec<u8>, String> {
    match model.token_to_piece_bytes(token, 32, false, None) {
        Ok(bytes) => Ok(bytes),
        Err(TokenToStringError::InsufficientBufferSpace(needed)) => model
            .token_to_piece_bytes(token, needed.unsigned_abs() as usize, false, None)
            .map_err(|e| format!("llama detokenize: {e}")),
        Err(e) => Err(format!("llama detokenize: {e}")),
    }
}

/// Real llama.cpp generation — loads GGUF and samples tokens.
///
/// `backend_preference` is the user's `compute_backend` setting verbatim; what
/// it resolves to depends on this build and this machine, and both are decided
/// here rather than at the call sites.
pub fn run_llama(
    model_path: &Path,
    prompt: &str,
    max_tokens: usize,
    backend_preference: &str,
) -> Result<String, String> {
    if !model_path.is_file() {
        return Err(format!("GGUF model missing: {}", model_path.display()));
    }
    let meta = std::fs::metadata(model_path).map_err(|e| e.to_string())?;
    if meta.len() < 1_000_000 {
        return Err(
            "GGUF model file is too small to be valid weights (download may be incomplete)".into(),
        );
    }

    let backend = backend()?;
    let chosen = resolve_llm_backend(backend_preference, probe_support());
    // The backend belongs in the key: the same weights resident on the GPU and
    // resident on the CPU are two different objects, so changing the setting
    // has to reload rather than keep serving the old one.
    let key = format!("{}#{}", model_path.display(), chosen.as_str());
    // The lock is held across generation, not just the load. `LlamaModel` is not
    // `Clone` in these bindings and a context borrows it, which is the honest
    // shape: two generations sharing one model would race inside llama.cpp
    // anyway, and the old code's `.clone()` only hid that.
    let mut cache = llm_cache().lock();
    let need_load = cache.as_ref().map(|(k, _)| k != &key).unwrap_or(true);
    if need_load {
        // Dropped before the new load so two models are never resident at once —
        // on a machine that just about fits one, holding both is the difference
        // between working and being killed.
        *cache = None;
        let (model, actual) = load_model(backend, model_path, chosen)?;
        // Keyed by where the weights actually went. A GPU load that failed on a
        // busy card and fell back would otherwise sit under the GPU key for the
        // rest of the process, so the card freeing up would never be noticed.
        *cache = Some((
            format!("{}#{}", model_path.display(), actual.as_str()),
            model,
        ));
    }
    let model = &cache.as_ref().expect("just loaded").1;

    let tokens = model
        .str_to_token(prompt, AddBos::Always)
        .map_err(|e| format!("llama tokenize: {e}"))?;

    // Sized to this call, and never past what the weights were trained for: a
    // context larger than `n_ctx_train` is allocated, paid for in memory, and
    // gives worse output than the model's real window.
    let want = tokens.len() + max_tokens + 8;
    let n_ctx = want.min(model.n_ctx_train() as usize).max(64) as u32;
    if tokens.len() >= n_ctx as usize {
        // Refusing beats silently truncating: a summary of the first half of a
        // meeting reads exactly like a summary of the meeting.
        return Err(format!(
            "the prompt is {} tokens and this model holds {}",
            tokens.len(),
            n_ctx
        ));
    }
    // What is left of the window after the prompt. A long prompt that still fits
    // gets a shorter answer; asking for the full `max_tokens` anyway would run
    // off the end of the context and fail the decode mid-sentence, which reads
    // as the model breaking rather than as the transcript being long.
    let room = n_ctx as usize - tokens.len();
    let max_tokens = max_tokens.min(room);

    let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(n_ctx));
    let mut ctx = model
        .new_context(backend, ctx_params)
        .map_err(|e| format!("llama context: {e}"))?;

    let mut batch = LlamaBatch::new(n_ctx as usize, 1);
    let last = tokens.len() - 1;
    for (i, token) in tokens.iter().enumerate() {
        // Logits only for the final token: the ones before it are context, and
        // asking for all of them allocates a vocabulary-sized row per token.
        batch
            .add(*token, i as i32, &[0], i == last)
            .map_err(|e| format!("llama batch: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| format!("llama decode: {e}"))?;

    // Deterministic on purpose. A summary the user re-runs should not come back
    // different, and greedy also keeps a seed out of the equation for tests.
    let mut sampler = LlamaSampler::greedy();

    // Bytes, decoded once at the end. A multi-byte character can span two tokens,
    // so decoding each piece on its own turns every accented word in a Portuguese
    // summary into replacement characters.
    let mut out: Vec<u8> = Vec::new();
    for step in 0..max_tokens {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        if model.is_eog_token(token) {
            break;
        }
        sampler.accept(token);
        out.extend_from_slice(&piece(model, token)?);
        batch.clear();
        batch
            .add(token, tokens.len() as i32 + step as i32, &[0], true)
            .map_err(|e| format!("llama batch: {e}"))?;
        ctx.decode(&mut batch)
            .map_err(|e| format!("llama decode: {e}"))?;
    }
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

#[cfg(test)]
mod gpu_bench {
    use super::*;

    /// Generate from the local model on each backend in turn, and print what
    /// each one cost.
    ///
    /// Ignored: it needs downloaded weights, and CUDA needs the downloaded
    /// pack, neither of which CI has. Run it by hand with
    /// `cargo test --release --features gpu-vulkan -- --ignored --nocapture gpu_bench`.
    ///
    /// The prompt is long on purpose. CUDA's advantage over Vulkan is in
    /// prompt processing, and a two-word prompt measures the part that is the
    /// same on both.
    #[test]
    #[ignore]
    fn generates_on_every_backend() {
        let model = dirs::data_dir()
            .unwrap()
            .join("Vesper")
            .join("models")
            .join("qwen2.5-0.5b")
            .join("model.gguf");
        assert!(model.is_file(), "qwen2.5-0.5b is not downloaded");

        println!("devices: {:#?}", gpu_devices());
        println!("support: {:?}", probe_support());

        let transcript = "Me: we need to decide the release scope today. \
             Others: the installer is the blocker, everything else is done. \
             Me: what is left on it. Others: signing and the update feed. \
             Me: then let us cut the version once those two land."
            .repeat(12);
        let prompt = format!(
            "Summarise the following meeting in three bullet points.\n\n{transcript}\n\nSummary:"
        );

        let only =
            std::env::var("VESPER_BENCH_BACKENDS").unwrap_or_else(|_| "cpu,vulkan,cuda".into());
        for backend in only.split(',') {
            let started = std::time::Instant::now();
            match run_llama(&model, &prompt, 64, backend) {
                Ok(text) => println!(
                    "{backend:>7}: {:>8.2?}  {:?}",
                    started.elapsed(),
                    text.chars().take(80).collect::<String>()
                ),
                Err(e) => println!("{backend:>7}: failed — {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn soft_fallback_summarize_without_model() {
        let llm = LocalLlm::with_models_dir(tempdir().unwrap().path().to_path_buf());
        let i = llm
            .summarize(
                "Me: we need to ship auth. Others: agreed. TODO write tests.",
                SummaryTemplate::General,
                Locale::En,
                "qwen2.5-1.5b",
                "cpu",
            )
            .unwrap();
        assert!(!i.summary.is_empty());
    }

    #[test]
    fn hard_mode_errors_without_model() {
        let mut llm = LocalLlm::with_models_dir(tempdir().unwrap().path().to_path_buf());
        llm.soft_fallback = false;
        let err = llm
            .summarize(
                "hi",
                SummaryTemplate::General,
                Locale::En,
                "qwen2.5-1.5b",
                "cpu",
            )
            .unwrap_err();
        assert!(err.contains("not installed"));
    }

    #[test]
    fn run_llama_rejects_tiny_file() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("model.gguf");
        std::fs::write(&p, b"not-gguf").unwrap();
        let err = run_llama(&p, "Hello", 8, "cpu").unwrap_err();
        assert!(err.contains("too small") || err.contains("llama") || err.contains("GGUF"));
    }
}
