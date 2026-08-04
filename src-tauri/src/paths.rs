use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Vesper")
}

pub fn models_dir() -> PathBuf {
    app_data_dir().join("models")
}

pub fn recordings_dir() -> PathBuf {
    app_data_dir().join("recordings")
}

/// Compute backends the user chose to download, beside the models they chose
/// to download.
///
/// Not in the installer: the CUDA pack is 636 MB, most of it kernels for
/// hardware the machine does not have, and a card that can run these models at
/// all already runs them on Vulkan. It is fetched on request, like weights.
pub fn backends_dir() -> PathBuf {
    app_data_dir().join("backends")
}

/// Where `tracing` writes, next to the database.
///
/// A release build has no console at all — `main.rs` sets
/// `windows_subsystem = "windows"` — so a warning with no file behind it is
/// unrecoverable. The appender creates this directory itself, which is why it is
/// not in `ensure_app_dirs`: logging is configured before the app state exists.
pub fn logs_dir() -> PathBuf {
    app_data_dir().join("logs")
}

pub fn ensure_app_dirs() -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(models_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(recordings_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(backends_dir()).map_err(|e| e.to_string())?;
    Ok(())
}
