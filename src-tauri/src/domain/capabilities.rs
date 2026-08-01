//! Local capability detection + recommended defaults (pure merge of probe results).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CapabilityReport {
    pub cpu_cores: u32,
    pub cuda_available: bool,
    pub cuda_device_name: Option<String>,
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
}

/// Build a capability report + recommended local models from probe data.
pub fn recommend_from_probe(probe: &CapabilityProbe) -> CapabilityReport {
    let cores = probe.cpu_cores.max(1);
    let mut notes = Vec::new();
    let (stt, llm, backend) = if probe.cuda_available {
        notes.push("CUDA GPU detected — prefer GPU-accelerated local models when available.".into());
        (
            "whisper-base".to_string(),
            "qwen2.5-1.5b".to_string(),
            "cuda".to_string(),
        )
    } else {
        notes.push("No CUDA GPU found — using CPU-friendly defaults (whisper-tiny / Qwen 0.5B).".into());
        (
            "whisper-tiny".to_string(),
            "qwen2.5-0.5b".to_string(),
            "cpu".to_string(),
        )
    };
    if cores < 4 {
        notes.push("Few CPU cores — keep local models small for live transcription.".into());
    }
    CapabilityReport {
        cpu_cores: cores,
        cuda_available: probe.cuda_available,
        cuda_device_name: probe.cuda_device_name.clone(),
        recommended_stt_model: stt,
        recommended_llm_model: llm,
        recommended_backend: backend,
        notes,
    }
}

/// Platform probe: CPU cores + best-effort CUDA detection via env / nvidia-smi.
pub fn probe_host() -> CapabilityProbe {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let mut cuda_available = false;
    let mut cuda_device_name = None;

    // Env override for testing / forced off
    if std::env::var("VESPER_FORCE_NO_CUDA").is_ok() {
        return CapabilityProbe {
            cpu_cores,
            cuda_available: false,
            cuda_device_name: None,
        };
    }
    if let Ok(name) = std::env::var("VESPER_FORCE_CUDA_NAME") {
        return CapabilityProbe {
            cpu_cores,
            cuda_available: true,
            cuda_device_name: Some(name),
        };
    }

    // nvidia-smi best-effort (Windows/Linux)
    if let Ok(out) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !text.is_empty() && !text.to_ascii_lowercase().contains("not found") {
                cuda_available = true;
                cuda_device_name = Some(text.lines().next().unwrap_or("CUDA GPU").to_string());
            }
        }
    }

    CapabilityProbe {
        cpu_cores,
        cuda_available,
        cuda_device_name,
    }
}

pub fn detect_capabilities() -> CapabilityReport {
    recommend_from_probe(&probe_host())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_only_recommends_tiny() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 8,
            cuda_available: false,
            cuda_device_name: None,
        });
        assert!(!r.cuda_available);
        assert_eq!(r.recommended_stt_model, "whisper-tiny");
        assert_eq!(r.recommended_backend, "cpu");
        assert!(!r.notes.is_empty());
    }

    #[test]
    fn cuda_recommends_larger() {
        let r = recommend_from_probe(&CapabilityProbe {
            cpu_cores: 16,
            cuda_available: true,
            cuda_device_name: Some("RTX 4090".into()),
        });
        assert!(r.cuda_available);
        assert_eq!(r.recommended_stt_model, "whisper-base");
        assert_eq!(r.recommended_backend, "cuda");
        assert_eq!(r.cuda_device_name.as_deref(), Some("RTX 4090"));
    }
}
