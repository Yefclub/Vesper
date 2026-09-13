use crate::domain::profile::Profile;
use std::path::PathBuf;
use std::sync::OnceLock;

static PROFILE: OnceLock<Profile> = OnceLock::new();

/// Fix which copy of the app this process is. `run()` calls it before the log
/// file, the database or the keychain is opened, because the answer decides
/// whose files those are.
pub fn set_profile(profile: Profile) {
    let _ = PROFILE.set(profile);
}

/// The profile `set_profile` fixed. Standard when nothing did, which is every
/// test.
pub fn profile() -> Profile {
    PROFILE.get().copied().unwrap_or(Profile::Standard)
}

pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(profile().data_dir_name())
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
