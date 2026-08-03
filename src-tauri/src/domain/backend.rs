//! Which compute backend a model load should ask for.
//!
//! Pure, so the decision is testable on a machine with no GPU — which is most
//! machines that will ever run `cargo test`, including CI.

use serde::{Deserialize, Serialize};

/// The backend actually used for one model load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputeBackend {
    Cpu,
    Cuda,
    Vulkan,
}

impl ComputeBackend {
    /// The name that goes in the log line and in the cache key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
            Self::Vulkan => "vulkan",
        }
    }

    pub fn is_gpu(self) -> bool {
        !matches!(self, Self::Cpu)
    }
}

/// What the build can actually do, and what the machine actually has.
///
/// Separate from the user's preference on purpose: a setting that says `cuda` on
/// a build without the CUDA feature is a wish, not a capability, and resolving
/// the two in one place is what stops each call site inventing its own answer.
#[derive(Debug, Clone, Copy, Default)]
pub struct BackendSupport {
    pub cuda_built: bool,
    pub vulkan_built: bool,
    pub cuda_present: bool,
    pub vulkan_present: bool,
}

impl BackendSupport {
    fn can(self, backend: ComputeBackend) -> bool {
        match backend {
            ComputeBackend::Cpu => true,
            ComputeBackend::Cuda => self.cuda_built && self.cuda_present,
            ComputeBackend::Vulkan => self.vulkan_built && self.vulkan_present,
        }
    }
}

/// Resolve the user's setting against what is really available.
///
/// `auto` prefers CUDA over Vulkan on NVIDIA: both work there, but CUDA is
/// markedly faster at prompt processing, and prompt processing is what
/// summarising a long transcript is made of.
///
/// Only `cpu` pins the processor. A GPU that was asked for and cannot be
/// reached falls through to the best one that can, because what the setting
/// names is a way to the card rather than an end in itself: someone whose
/// stored `cuda` outlived the build that carried it wants their GPU, and
/// dropping them to the processor makes the app slow for a reason they cannot
/// see. `resolve_stt_backend` below already worked this way for whisper, and
/// the two disagreeing was an inconsistency rather than a design.
///
/// The substitution is not silent: the caller logs what it resolved to, and
/// the settings panel shows a stored value this build cannot honour.
pub fn resolve_backend(preference: &str, support: BackendSupport) -> ComputeBackend {
    // The one answer nothing overrides. It is how someone diagnosing a driver
    // problem takes the GPU out of the picture.
    if preference == "cpu" {
        return ComputeBackend::Cpu;
    }
    // `auto`, and anything a future build or a hand-edited row might carry,
    // name nothing in particular and land straight on the automatic order.
    // Settings are the user's own file; a typo there should degrade rather than
    // refuse to transcribe.
    let wanted = match preference {
        "cuda" => Some(ComputeBackend::Cuda),
        "vulkan" => Some(ComputeBackend::Vulkan),
        _ => None,
    };
    if let Some(backend) = wanted {
        if support.can(backend) {
            return backend;
        }
    }
    if support.can(ComputeBackend::Cuda) {
        ComputeBackend::Cuda
    } else if support.can(ComputeBackend::Vulkan) {
        ComputeBackend::Vulkan
    } else {
        ComputeBackend::Cpu
    }
}

/// What llama can reach, which is not what whisper can reach.
///
/// Vulkan is struck out, and the manifest already declines to compile it for
/// llama — this is the belt to that pair of braces, and the place the reason
/// is written down.
///
/// whisper-rs-sys and llama-cpp-sys-2 each vendor their own ggml. A process
/// where both of them initialise Vulkan dies with an access violation while
/// loading tensors. Measured rather than guessed: removing `whisper-rs/vulkan`
/// from the same build makes llama offload all its layers and generate
/// normally, and putting it back crashes again. Two models and two
/// quantisations behave identically, so it is the pair of backends and not the
/// weights — and the warning that looks like a clue,
/// `token_embd.weight (q5_0) cannot be used with preferred buffer type`, is
/// printed by the working CUDA path too.
///
/// Whisper is the one that keeps Vulkan, because transcription is the work
/// this application actually does: it runs for the length of the meeting,
/// while summarising happens once at the end. llama reaches a GPU through the
/// optional CUDA pack, which is a separate ggml loaded at runtime and does not
/// collide.
pub fn resolve_llm_backend(preference: &str, support: BackendSupport) -> ComputeBackend {
    let no_vulkan = BackendSupport {
        vulkan_built: false,
        vulkan_present: false,
        ..support
    };
    resolve_backend(preference, no_vulkan)
}

/// What whisper can reach, which is not what llama can reach.
///
/// `whisper-rs-sys` links CUDA as a load-time import and offers no dynamic
/// backend loading, so a build with its `cuda` feature does not start at all on
/// a machine without the NVIDIA runtime. Whisper therefore ships Vulkan only,
/// and an `auto` that picked CUDA for the LLM still picks Vulkan here.
pub fn resolve_stt_backend(preference: &str, support: BackendSupport) -> ComputeBackend {
    let no_cuda = BackendSupport {
        cuda_built: false,
        cuda_present: false,
        ..support
    };
    // No special case for an explicit `cuda` any more. With CUDA struck from
    // what this engine can do, `resolve_backend` falls through to Vulkan on its
    // own, for the same reason: the GPU is there, just reached through the
    // other API.
    resolve_backend(preference, no_cuda)
}

/// What this build was compiled with.
///
/// A `cfg!` and not a probe: the features are decided when the binary is made,
/// and a build without them cannot use the hardware however present it is.
///
/// It is the floor, not the whole answer. CUDA can arrive as a downloaded
/// backend module long after the binary was built, so the caller raises
/// `cuda_built` when ggml reports a CUDA device — a device only appears once
/// its backend has registered.
pub fn built_backends() -> (bool, bool) {
    (cfg!(feature = "gpu-cuda"), cfg!(feature = "gpu-vulkan"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nvidia() -> BackendSupport {
        BackendSupport {
            cuda_built: true,
            vulkan_built: true,
            cuda_present: true,
            vulkan_present: true,
        }
    }

    fn amd() -> BackendSupport {
        BackendSupport {
            cuda_built: true,
            vulkan_built: true,
            cuda_present: false,
            vulkan_present: true,
        }
    }

    fn headless() -> BackendSupport {
        BackendSupport {
            cuda_built: true,
            vulkan_built: true,
            ..Default::default()
        }
    }

    #[test]
    fn auto_prefers_cuda_where_it_exists() {
        assert_eq!(resolve_backend("auto", nvidia()), ComputeBackend::Cuda);
        assert_eq!(resolve_backend("auto", amd()), ComputeBackend::Vulkan);
        assert_eq!(resolve_backend("auto", headless()), ComputeBackend::Cpu);
    }

    /// The setting has to be able to say no. Someone diagnosing a GPU driver
    /// problem needs CPU to mean CPU.
    #[test]
    fn cpu_is_honoured_even_with_a_gpu_present() {
        assert_eq!(resolve_backend("cpu", nvidia()), ComputeBackend::Cpu);
    }

    /// Asking for CUDA on an AMD card takes Vulkan. The setting names a way to
    /// the card and the card is there, so dropping to the CPU would answer a
    /// question nobody asked.
    #[test]
    fn an_impossible_choice_takes_the_other_gpu_not_the_cpu() {
        assert_eq!(resolve_backend("cuda", amd()), ComputeBackend::Vulkan);
        assert_eq!(
            resolve_backend("vulkan", nvidia()),
            ComputeBackend::Vulkan,
            "vulkan is possible on nvidia and must be honoured"
        );
        // The same rule the other way round, which is the half a CUDA-only
        // build would exercise.
        let cuda_only = BackendSupport {
            cuda_built: true,
            vulkan_built: false,
            cuda_present: true,
            vulkan_present: false,
        };
        assert_eq!(resolve_backend("vulkan", cuda_only), ComputeBackend::Cuda);
    }

    /// The exact shape this shipped with: `cuda` stored by an older build, on a
    /// machine whose card is real and whose current binary carries Vulkan only.
    #[test]
    fn a_stored_cuda_on_a_vulkan_build_still_uses_the_gpu() {
        let vulkan_build = BackendSupport {
            cuda_built: false,
            vulkan_built: true,
            cuda_present: false,
            vulkan_present: true,
        };
        assert_eq!(
            resolve_backend("cuda", vulkan_build),
            ComputeBackend::Vulkan
        );
    }

    /// A build without the feature cannot use the hardware, however present it
    /// is — this is what stops a CPU-only build claiming a GPU.
    #[test]
    fn hardware_without_the_feature_is_not_a_backend() {
        let driver_only = BackendSupport {
            cuda_built: false,
            vulkan_built: false,
            cuda_present: true,
            vulkan_present: true,
        };
        assert_eq!(resolve_backend("auto", driver_only), ComputeBackend::Cpu);
        assert_eq!(resolve_backend("cuda", driver_only), ComputeBackend::Cpu);
        assert_eq!(resolve_backend("vulkan", driver_only), ComputeBackend::Cpu);
    }

    /// llama never gets Vulkan: offloading to it crashes the process on the
    /// catalog's default model. CUDA works, so an NVIDIA machine still gets a
    /// GPU; everything else falls to the processor.
    #[test]
    fn llama_uses_cuda_or_the_cpu_and_never_vulkan() {
        assert_eq!(resolve_llm_backend("auto", nvidia()), ComputeBackend::Cuda);
        assert_eq!(
            resolve_llm_backend("vulkan", nvidia()),
            ComputeBackend::Cuda
        );
        assert_eq!(resolve_llm_backend("auto", amd()), ComputeBackend::Cpu);
        assert_eq!(resolve_llm_backend("vulkan", amd()), ComputeBackend::Cpu);
        assert_eq!(resolve_llm_backend("cpu", nvidia()), ComputeBackend::Cpu);
    }

    /// Whisper never gets CUDA, because a whisper built with it will not start
    /// on a machine that has no NVIDIA runtime.
    #[test]
    fn whisper_uses_vulkan_on_nvidia_rather_than_cuda() {
        assert_eq!(
            resolve_stt_backend("auto", nvidia()),
            ComputeBackend::Vulkan
        );
        assert_eq!(
            resolve_stt_backend("cuda", nvidia()),
            ComputeBackend::Vulkan
        );
        assert_eq!(resolve_stt_backend("cpu", nvidia()), ComputeBackend::Cpu);
        assert_eq!(resolve_stt_backend("auto", headless()), ComputeBackend::Cpu);
    }

    /// The shape a downloaded backend makes: nothing CUDA was compiled in, and
    /// a CUDA device is there anyway because the pack registered it. Reading
    /// the compile-time flag alone told such a machine its own card was
    /// unreachable and quietly left it on Vulkan.
    #[test]
    fn a_backend_that_arrived_after_the_build_is_still_a_backend() {
        let downloaded = BackendSupport {
            cuda_built: true,
            vulkan_built: true,
            cuda_present: true,
            vulkan_present: true,
        };
        assert_eq!(resolve_backend("cuda", downloaded), ComputeBackend::Cuda);
        assert_eq!(resolve_backend("auto", downloaded), ComputeBackend::Cuda);
    }

    /// A value nobody recognises degrades to the automatic answer. Settings are
    /// the user's own file; a typo there must not stop transcription.
    #[test]
    fn an_unknown_preference_behaves_like_auto() {
        assert_eq!(resolve_backend("metal", nvidia()), ComputeBackend::Cuda);
        assert_eq!(resolve_backend("", amd()), ComputeBackend::Vulkan);
    }
}
