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

pub fn ensure_app_dirs() -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(models_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(recordings_dir()).map_err(|e| e.to_string())?;
    Ok(())
}
