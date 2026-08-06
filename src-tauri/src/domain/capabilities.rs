//! Local capability detection + recommended defaults (pure merge of probe results).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CapabilityReport {
    pub cpu_cores: u32,
    pub cuda_available: bool,
    pub cuda_device_name: Option<String>,
    /// Additive, with a default, because a report deserialised from a version
    /// that predates GPU support has to keep loading.
    #[serde(default)]
    pub vulkan_available: bool,
    /// The card the models would actually run on, named by its driver.
    #[serde(default)]
    pub gpu_name: Option<String>,
    #[serde(default)]
    pub vram_mb: u64,
    pub recommended_stt_model: String,
    pub recommended_llm_model: String,
    pub recommended_backend: String,
    pub notes: Vec<String>,
}

/// Probe results injected by platform code (or tests).
#[derive(Debug, Clone, Default)]
pub struct CapabilityProbe {
    pub cpu_cores: u32,
    pub cuda_available: bool,
    pub cuda_device_name: Option<String>,
    pub vulkan_available: bool,
    pub gpu_name: Option<String>,
    pub vram_bytes: u64,
}

/// Below this, a GPU is not worth sizing models up for: the weights would spill
/// back to system memory and run slower than they would have on the CPU alone.
const USABLE_VRAM_MB: u64 = 3_000;

/// The large tier, set below what an 8 GB card reports rather than at it.
///
/// Observed on one: through Vulkan the same RTX 4070 reports 7948 MB and
/// through CUDA about 8188 MB. A boundary at 8000 therefore moved the machine
/// between tiers depending on which backend happened to be loaded — the card
/// did not change, the API asking did. Thresholds have to sit away from the
/// sizes cards actually report, not on them.
const LARGE_VRAM_MB: u64 = 7_000;

/// Build a capability report + recommended local models from probe data.
///
/// The ladder is set by what has to be resident at once — the weights plus the
/// context — and not by how fast the card is. A 4 GB card that holds the model
/// beats a 16 GB card that was never asked to.
pub fn recommend_from_probe(probe: &CapabilityProbe) -> CapabilityReport {
    let cores = probe.cpu_cores.max(1);
    let vram_mb = probe.vram_bytes / (1024 * 1024);
    // A GPU with no usable memory is not a GPU for this purpose. The
    // integrated chips that report a few hundred megabytes are the case this
    // catches, and they are exactly where the naive answer is worst.
    let gpu = (probe.cuda_available || probe.vulkan_available) && vram_mb >= USABLE_VRAM_MB;
    let mut notes = Vec::new();

    let (stt, llm) = if gpu && vram_mb >= LARGE_VRAM_MB {
        notes.push(format!(
            "{} MB of video memory — large local models will fit.",
            vram_mb
        ));
        ("whisper-large-v3-turbo", "mistral-7b-instruct")
    } else if gpu && vram_mb >= 5_000 {
        notes.push(format!(
            "{} MB of video memory — mid-sized models fit.",
            vram_mb
        ));
        ("whisper-small", "qwen3-4b-instruct")
    } else if gpu {
        notes.push(format!(
            "{} MB of video memory — small models only, but on the GPU.",
            vram_mb
        ));
        // The light tier, and the note beside it is the reason. This branch
        // starts at 3 GB, and the middle model is 2.5 GB of weights before the
        // context and the runtime ask for anything — the rule this function
        // sizes by is what has to be resident at once, not what fits on paper.
        ("whisper-small", "qwen2.5-0.5b")
    } else if cores >= 8 {
        notes.push("No usable GPU — running on the CPU, which has cores to spare.".into());
        ("whisper-small", "qwen2.5-0.5b")
    } else {
        notes.push("No usable GPU — using the smallest local models.".into());
        ("whisper-tiny", "qwen2.5-0.5b")
    };

    if let Some(name) = &probe.gpu_name {
        notes.push(format!("Graphics: {name}."));
    }
    // `auto` rather than the name of the backend that was found. What this
    // binary can reach is decided at build time, and a setting that says `cuda`
    // on a build without it resolves to the CPU — so naming the winner here
    // would be the one answer guaranteed to be wrong on a Vulkan-only build.
    let backend = if gpu { "auto" } else { "cpu" };
    if cores < 4 {
        notes.push("Few CPU cores — keep local models small for live transcription.".into());
    }
    CapabilityReport {
        cpu_cores: cores,
        cuda_available: probe.cuda_available,
        cuda_device_name: probe.cuda_device_name.clone(),
        vulkan_available: probe.vulkan_available,
        gpu_name: probe.gpu_name.clone(),
        vram_mb,
        recommended_stt_model: stt.to_string(),
        recommended_llm_model: llm.to_string(),
        recommended_backend: backend.to_string(),
        notes,
    }
}

/// Platform probe: CPU cores, GPUs ggml can reach, and best-effort CUDA naming.
pub fn probe_host() -> CapabilityProbe {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    // Env override for testing / forced off. Forces every accelerator off, not
    // only CUDA: a seam that left Vulkan on would stop being a way to reproduce
    // a CPU-only machine.
    if std::env::var("VESPER_FORCE_NO_CUDA").is_ok() {
        return CapabilityProbe {
            cpu_cores,
            ..Default::default()
        };
    }
    if let Ok(name) = std::env::var("VESPER_FORCE_CUDA_NAME") {
        return CapabilityProbe {
            cpu_cores,
            cuda_available: true,
            cuda_device_name: Some(name.clone()),
            gpu_name: Some(name),
            ..Default::default()
        };
    }

    // What ggml can actually reach. `nvidia-smi` answers "there is an NVIDIA
    // card in this machine"; this answers "this build can put a model on a GPU,
    // and it has this much memory", which is the question a recommendation is
    // really asking.
    let mut vulkan_available = false;
    let mut cuda_available = false;
    let devices = crate::llm::local::gpu_devices();
    for device in &devices {
        match device.backend.as_str() {
            "CUDA" => cuda_available = true,
            "Vulkan" => vulkan_available = true,
            _ => {}
        }
    }
    // A discrete card if there is one, and only then the largest.
    //
    // Size alone picks wrong on the exact machine this was written on: a laptop
    // with an RTX 4070 reports 7.8 GB of real video memory beside an integrated
    // Intel Arc reporting 18 GB — which is not memory the chip has, it is a
    // slice of the machine's RAM it is allowed to borrow. Taking the bigger
    // number named the weaker device and pushed the model recommendation two
    // tiers past what the real card can hold.
    let best = devices
        .iter()
        .filter(|d| d.discrete)
        .max_by_key(|d| d.total_bytes)
        .or_else(|| devices.iter().max_by_key(|d| d.total_bytes));
    let mut gpu_name = best.map(|d| d.description.clone());
    // Only a discrete card's memory sizes the models. An integrated chip shares
    // system RAM, so its number says nothing about what will stay resident.
    let vram_bytes = best.filter(|d| d.discrete).map_or(0, |d| d.total_bytes);

    // nvidia-smi best-effort (Windows/Linux). Still worth asking on a build
    // with no CUDA backend: it is what names the card in the report, and the
    // name is what tells the user their GPU was seen and not used.
    let mut cuda_device_name = None;
    let mut smi = std::process::Command::new("nvidia-smi");
    smi.args(["--query-gpu=name", "--format=csv,noheader"]);
    // `nvidia-smi` is a console program, and a GUI process spawning one on
    // Windows gets a console window for as long as it runs. It flashed on
    // screen every time the settings panel opened, which is every time this
    // probe runs.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        smi.creation_flags(CREATE_NO_WINDOW);
    }
    if let Ok(out) = smi.output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !text.is_empty() && !text.to_ascii_lowercase().contains("not found") {
                cuda_device_name = Some(text.lines().next().unwrap_or("CUDA GPU").to_string());
            }
        }
    }
    if gpu_name.is_none() {
        gpu_name = cuda_device_name.clone();
    }

    CapabilityProbe {
        cpu_cores,
        cuda_available,
        cuda_device_name,
        vulkan_available,
        gpu_name,
        vram_bytes,
    }
}

pub fn detect_capabilities() -> CapabilityReport {
    recommend_from_probe(&probe_host())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn cpu_only_recommends_tiny() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 2,
            ..Default::default()
        });
        assert!(!r.cuda_available);
        assert_eq!(r.recommended_stt_model, "whisper-tiny");
        assert_eq!(r.recommended_backend, "cpu");
        assert!(!r.notes.is_empty());
    }

    /// Plenty of cores is still not a GPU, but it does carry a bigger whisper.
    #[test]
    fn many_cores_without_a_gpu_moves_up_one_step_only() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 16,
            ..Default::default()
        });
        assert_eq!(r.recommended_stt_model, "whisper-small");
        assert_eq!(r.recommended_llm_model, "qwen2.5-0.5b");
        assert_eq!(r.recommended_backend, "cpu");
    }

    #[test]
    fn a_large_card_gets_the_large_models() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 16,
            cuda_available: true,
            cuda_device_name: Some("RTX 4090".into()),
            gpu_name: Some("RTX 4090".into()),
            vram_bytes: 24 * GB,
            ..Default::default()
        });
        assert_eq!(r.recommended_stt_model, "whisper-large-v3-turbo");
        assert_eq!(r.recommended_llm_model, "mistral-7b-instruct");
        assert_eq!(r.vram_mb, 24 * 1024);
        assert_eq!(r.cuda_device_name.as_deref(), Some("RTX 4090"));
    }

    #[test]
    fn a_mid_card_gets_mid_models() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 8,
            vulkan_available: true,
            gpu_name: Some("Radeon RX 6600".into()),
            vram_bytes: 6 * GB,
            ..Default::default()
        });
        assert_eq!(r.recommended_stt_model, "whisper-small");
        assert_eq!(r.recommended_llm_model, "qwen3-4b-instruct");
        assert_eq!(r.recommended_backend, "auto");
    }

    /// The recommendation never names a backend, because which one this binary
    /// can reach is a build-time fact and `auto` is the only value that
    /// survives being written into settings by one build and read by another.
    #[test]
    fn a_gpu_is_recommended_as_auto_and_not_by_name() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 8,
            cuda_available: true,
            vram_bytes: 12 * GB,
            ..Default::default()
        });
        assert_eq!(r.recommended_backend, "auto");
    }

    /// The laptop this was written on reports an RTX 4070 with 7.8 GB beside an
    /// integrated Intel Arc claiming 18 GB — which is not memory the chip has,
    /// it is a slice of system RAM it may borrow. Sizing by the larger number
    /// named the weaker device and reported eighteen gigabytes of video memory
    /// that do not exist.
    ///
    /// The assertion is on the number carried through, not on the tier it lands
    /// in: which models a given size deserves is the neighbouring test's
    /// business, and tying both to one boundary made moving that boundary look
    /// like breaking this.
    #[test]
    fn an_integrated_chip_does_not_outrank_the_real_card() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 22,
            vulkan_available: true,
            gpu_name: Some("NVIDIA GeForce RTX 4070 Laptop GPU".into()),
            vram_bytes: 7948 * 1024 * 1024,
            ..Default::default()
        });
        assert_eq!(r.vram_mb, 7948);
        assert_eq!(
            r.gpu_name.as_deref(),
            Some("NVIDIA GeForce RTX 4070 Laptop GPU")
        );
    }

    /// The same card seen through two APIs must land in the same tier. Vulkan
    /// reports 7948 MB for an RTX 4070 and CUDA about 8188 MB; a boundary
    /// between those two numbers moved the machine's recommendation when a
    /// downloaded backend registered, which is a change nobody made.
    #[test]
    fn one_card_reported_two_ways_stays_in_one_tier() {
        let tier = |mb: u64| {
            recommend_from_probe(&CapabilityProbe {
                cpu_cores: 22,
                vulkan_available: true,
                vram_bytes: mb * 1024 * 1024,
                ..Default::default()
            })
            .recommended_llm_model
        };
        assert_eq!(tier(7_948), tier(8_188));
    }

    /// An integrated chip with a sliver of memory would run a large model
    /// slower than the CPU would, because the weights spill straight back out.
    #[test]
    fn a_gpu_with_no_usable_memory_is_not_treated_as_a_gpu() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 8,
            vulkan_available: true,
            gpu_name: Some("Intel UHD Graphics".into()),
            vram_bytes: GB / 2,
            ..Default::default()
        });
        assert_eq!(r.recommended_backend, "cpu");
        assert_eq!(r.recommended_llm_model, "qwen2.5-0.5b");
    }
}
