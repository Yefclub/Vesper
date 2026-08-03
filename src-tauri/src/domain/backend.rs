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

    /// A value nobody recognises degrades to the automatic answer. Settings are
    /// the user's own file; a typo there must not stop transcription.
    #[test]
    fn an_unknown_preference_behaves_like_auto() {
        assert_eq!(resolve_backend("metal", nvidia()), ComputeBackend::Cuda);
        assert_eq!(resolve_backend("", amd()), ComputeBackend::Vulkan);
    }
}
