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

    let (stt, llm) = if gpu && vram_mb >= 8_000 {
        notes.push(format!(
            "{} MB of video memory — large local models will fit.",
            vram_mb
        ));
        ("whisper-large-v3-turbo", "qwen2.5-3b")
    } else if gpu && vram_mb >= 5_000 {
        notes.push(format!(
            "{} MB of video memory — mid-sized models fit.",
            vram_mb
        ));
        ("whisper-small", "qwen2.5-1.5b")
    } else if gpu {
        notes.push(format!(
            "{} MB of video memory — small models only, but on the GPU.",
            vram_mb
        ));
        ("whisper-base", "qwen2.5-1.5b")
    } else if cores >= 8 {
        notes.push("No usable GPU — running on the CPU, which has cores to spare.".into());
        ("whisper-base", "qwen2.5-0.5b")
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
    let mut gpu_name = None;
    let mut vram_bytes = 0u64;
    for device in crate::llm::local::gpu_devices() {
        match device.backend.as_str() {
            "CUDA" => cuda_available = true,
            "Vulkan" => vulkan_available = true,
            _ => {}
        }
        // The largest card, not the first. A laptop reports its integrated chip
        // alongside the discrete one, and the discrete one is what will run.
        if device.total_bytes > vram_bytes {
            vram_bytes = device.total_bytes;
            gpu_name = Some(device.description);
        }
    }

    // nvidia-smi best-effort (Windows/Linux). Still worth asking on a build
    // with no CUDA backend: it is what names the card in the report, and the
    // name is what tells the user their GPU was seen and not used.
    let mut cuda_device_name = None;
    if let Ok(out) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
    {
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
        assert_eq!(r.recommended_stt_model, "whisper-base");
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
        assert_eq!(r.recommended_llm_model, "qwen2.5-3b");
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
        assert_eq!(r.recommended_llm_model, "qwen2.5-1.5b");
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
