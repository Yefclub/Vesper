use crate::domain::download::{
    backoff, describe, is_retryable_status, plan_resume, ProgressPacer, ResumeAction, Sample,
};
use crate::paths::models_dir;
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tar::Archive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub ready: bool,
    /// The artifact exists on disk. Separate from , which also requires a
    /// verified checksum — a user who downloaded before checksums existed has a
    /// real file that is simply unvouched for, and telling them it is "not
    /// downloaded" invites a re-download they do not need.
    pub present: bool,
    pub path: String,
    pub download_url: Option<String>,
    pub size_hint_bytes: Option<u64>,
    /// Bytes of an interrupted download still on disk, when there are any.
    ///
    /// Without this the app is honest but useless about a half-finished
    /// transfer: it correctly reports the model as not installed and says
    /// nothing about the 276 MB of 491 MB already sitting beside it, which the
    /// downloader would pick up rather than re-fetch.
    ///
    /// It is "already downloaded", never "will resume": a `.part` written
    /// before the ETag sidecar existed carries no validator, so the resume goes
    /// out without `If-Range` and the server is free to answer from zero.
    pub partial_bytes: Option<u64>,
    /// Expected SHA-256 of the artifact served by `download_url`.
    /// Taken from the Hugging Face LFS oid, which is the file digest.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
    pub phase: String,
    pub bytes_per_sec: Option<u64>,
    pub eta_secs: Option<u64>,
    pub attempt: u32,
    pub resumed_from_bytes: u64,
}

impl DownloadProgress {
    /// A frame that reports where the download stands, not how fast it is moving.
    fn phase(
        model_id: &str,
        phase: &str,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        done: bool,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            downloaded_bytes,
            total_bytes,
            done,
            error: None,
            phase: phase.into(),
            bytes_per_sec: None,
            eta_secs: None,
            attempt: 1,
            resumed_from_bytes: 0,
        }
    }

    /// A frame from inside the transfer loop, already paced.
    fn transfer(
        model_id: &str,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        sample: Sample,
        attempt: u32,
        resumed_from_bytes: u64,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            downloaded_bytes,
            total_bytes,
            done: false,
            error: None,
            phase: "downloading".into(),
            bytes_per_sec: sample.bytes_per_sec,
            eta_secs: sample.eta_secs,
            attempt,
            resumed_from_bytes,
        }
    }
}

/// Catalog of local models with **direct** artifact URLs (not empty placeholders).
///
/// This is the only source of download URLs. Nothing reaching the IPC boundary can
/// point the downloader somewhere else: a model artifact is parsed by whisper.cpp and
/// llama.cpp, so an attacker-chosen file is an attacker-chosen input to a C++ parser.
pub fn list_models() -> Vec<ModelInfo> {
    let catalog = [
        (
            "whisper-tiny",
            "stt",
            "Whisper Tiny (local STT, ggml)",
            // Official ggerganov whisper.cpp release asset (direct ggml binary)
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
            77_691_713u64,
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        ),
        (
            "whisper-base",
            "stt",
            "Whisper Base (local STT, ggml)",
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
            147_951_465u64,
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        ),
        (
            "qwen2.5-0.5b",
            "llm",
            "Qwen2.5 0.5B Instruct Q4_K_M (local LLM)",
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
            491_400_032u64,
            "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db",
        ),
        (
            "qwen2.5-1.5b",
            "llm",
            "Qwen2.5 1.5B Instruct Q4_K_M (local LLM)",
            "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
            1_117_320_736u64,
            "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        ),
    ];
    catalog
        .into_iter()
        .map(|(id, kind, label, url, size, sha256)| {
            let path = model_artifact_path(id, kind);
            let present = path.is_file()
                && std::fs::metadata(&path)
                    .map(|m| m.len() > 1_000_000)
                    .unwrap_or(false);
            let ready = std::fs::read_to_string(artifact_marker_path(&path))
                .map(|recorded| recorded.trim().eq_ignore_ascii_case(sha256))
                .unwrap_or(false)
                && present;
            ModelInfo {
                id: id.into(),
                kind: kind.into(),
                label: label.into(),
                ready,
                present,
                partial_bytes: partial_bytes(&path),
                path: path.display().to_string(),
                download_url: Some(url.into()),
                size_hint_bytes: Some(size),
                sha256: sha256.into(),
            }
        })
        .collect()
}

/// Where a download accumulates before it is verified and renamed into place.
///
/// One derivation, used by the transfer and by the reporting: two would let the
/// drawer offer to continue a file the downloader never looks at.
fn part_path(artifact: &Path) -> PathBuf {
    artifact.with_extension("part")
}

/// How much of an interrupted download is on disk, if any.
///
/// A zero-length leftover reports `None` along with an absent one — there is
/// nothing to continue in either case, and offering to resume 0 bytes is the
/// same lie in a different shape.
fn partial_bytes(artifact: &Path) -> Option<u64> {
    std::fs::metadata(part_path(artifact))
        .map(|m| m.len())
        .ok()
        .filter(|n| *n > 0)
}

/// Sidecar holding the digest this code verified for an artifact.
///
/// Its absence is meaningful: it marks bytes that were never checked, which is
/// exactly the state left behind by the old download path that accepted a URL
/// from the front end. Verifying only fresh downloads would leave the artifact
/// already on disk trusted forever.
pub fn artifact_marker_path(artifact: &Path) -> PathBuf {
    let mut name = artifact.file_name().unwrap_or_default().to_os_string();
    name.push(".sha256");
    artifact.with_file_name(name)
}

/// Digest the catalog expects for a model id.
pub fn catalog_sha256(model_id: &str) -> Option<String> {
    list_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .map(|m| m.sha256)
}

/// Whether an artifact may be handed to whisper.cpp / llama.cpp.
///
/// Reads the 64-byte sidecar instead of hashing: this is called from the record
/// gate and from the settings screen, and re-hashing a gigabyte there would make
/// the UI hang. The hash itself is computed once, on download or on the
/// verify-in-place path.
pub fn artifact_is_verified(artifact: &Path, model_id: &str) -> bool {
    let plausible = artifact.is_file()
        && std::fs::metadata(artifact)
            .map(|m| m.len() > 1_000_000)
            .unwrap_or(false);
    if !plausible {
        return false;
    }
    let Some(expected) = catalog_sha256(model_id) else {
        return false;
    };
    std::fs::read_to_string(artifact_marker_path(artifact))
        .map(|recorded| recorded.trim().eq_ignore_ascii_case(&expected))
        .unwrap_or(false)
}

fn write_artifact_marker(artifact: &Path, sha256: &str) -> Result<(), String> {
    std::fs::write(artifact_marker_path(artifact), sha256).map_err(|e| e.to_string())
}

fn model_artifact_path(id: &str, kind: &str) -> PathBuf {
    let name = if kind == "stt" {
        "model.bin"
    } else {
        "model.gguf"
    };
    models_dir().join(id).join(name)
}

/// Download a model with streaming progress callbacks.
/// Handles raw binaries and `.tar` / `.tar.gz` archives.
///
/// The URL is resolved from the internal catalog by `model_id` — it is deliberately
/// not a parameter, so no caller can redirect the download.
pub async fn download_model_with_progress<F>(
    model_id: &str,
    mut on_progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(DownloadProgress) + Send,
{
    let info = list_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("unknown model {model_id}"))?;
    let url = info
        .download_url
        .clone()
        .ok_or_else(|| format!("model {model_id} has no download URL in the catalog"))?;
    let url = url.as_str();
    let dest = PathBuf::from(&info.path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // An artifact may already be on disk from a build that did not verify anything.
    // Hash it before pulling hundreds of MB again: if it is genuine, this costs a
    // few seconds and records the marker; if it is not, the download proceeds and
    // replaces it.
    if dest.is_file() {
        on_progress(DownloadProgress::phase(
            model_id,
            "verifying",
            0,
            info.size_hint_bytes,
            false,
        ));
        if let Ok(existing) = sha256_file(&dest) {
            if existing.eq_ignore_ascii_case(&info.sha256) {
                write_artifact_marker(&dest, &info.sha256)?;
                on_progress(DownloadProgress::phase(
                    model_id,
                    "done",
                    std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
                    info.size_hint_bytes,
                    true,
                ));
                return Ok(dest);
            }
        }
    }

    on_progress(DownloadProgress::phase(
        model_id,
        "connecting",
        0,
        info.size_hint_bytes,
        false,
    ));

    let client = reqwest::Client::builder()
        .user_agent("Vesper/0.1")
        .connect_timeout(Duration::from_secs(10))
        // Deliberately no total `.timeout()`: qwen2.5-1.5b is 1.1 GB, and a deadline
        // on the whole transfer is a manufactured failure on any slow link.
        // `read_timeout` resets on every successful read, so it only fires on a
        // stream that has genuinely died — which used to hang forever.
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let tmp = part_path(&dest);
    let mut pacer = ProgressPacer::new();
    let mut transfer = fetch_to_file(
        &client,
        model_id,
        url,
        &tmp,
        info.size_hint_bytes,
        &mut pacer,
        &mut on_progress,
    )
    .await?;
    let mut actual = verify_part(model_id, &tmp, &transfer, &mut on_progress).await?;

    // A `.part` from a build that predates the ETag sidecar was resumed with nothing
    // guarding the range, so a mismatch here is far more likely to be two objects
    // spliced together than an artifact that changed under a pinned URL. Taking the
    // whole thing once costs what the old code charged for *every* dropped connection,
    // and it is the difference between a slow success and telling the user their
    // download is broken. Only ever once: the second pass starts from zero, so if it
    // still mismatches the bytes on the wire genuinely are not what the catalog says.
    if !actual.eq_ignore_ascii_case(&info.sha256) && transfer.unvalidated_resume {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(part_etag_path(&tmp));
        let mut pacer = ProgressPacer::new();
        transfer = fetch_to_file(
            &client,
            model_id,
            url,
            &tmp,
            info.size_hint_bytes,
            &mut pacer,
            &mut on_progress,
        )
        .await?;
        actual = verify_part(model_id, &tmp, &transfer, &mut on_progress).await?;
    }

    let downloaded = transfer.bytes;
    let total = Some(downloaded);
    if !actual.eq_ignore_ascii_case(&info.sha256) {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(part_etag_path(&tmp));
        return Err(format!(
            "checksum mismatch for {model_id}: expected {}, got {actual} — artifact discarded",
            info.sha256
        ));
    }
    // The bytes are complete and vouched for; the resume sidecar has nothing left to
    // guard.
    let _ = std::fs::remove_file(part_etag_path(&tmp));

    // Archive handling
    let lower = url.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        on_progress(DownloadProgress::phase(
            model_id,
            "extracting",
            downloaded,
            total,
            false,
        ));
        extract_tar_gz(&tmp, dest.parent().unwrap(), &dest)?;
        let _ = std::fs::remove_file(&tmp);
    } else if lower.ends_with(".tar") {
        on_progress(DownloadProgress::phase(
            model_id,
            "extracting",
            downloaded,
            total,
            false,
        ));
        extract_tar(&tmp, dest.parent().unwrap(), &dest)?;
        let _ = std::fs::remove_file(&tmp);
    } else {
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    }

    let final_len = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if final_len < 1_000_000 {
        let _ = std::fs::remove_file(&dest);
        return Err(format!(
            "downloaded artifact too small ({final_len} bytes) — URL may not be a model file"
        ));
    }
    write_artifact_marker(&dest, &info.sha256)?;

    on_progress(DownloadProgress::phase(
        model_id,
        "done",
        final_len,
        Some(final_len),
        true,
    ));
    Ok(dest)
}

/// Sidecar holding the ETag of the object a `.part` file was cut from.
///
/// Sent back as `If-Range` on a resume so that an artifact which changed between
/// attempts comes back whole instead of spliced onto stale bytes.
fn part_etag_path(part: &Path) -> PathBuf {
    let mut name = part.file_name().unwrap_or_default().to_os_string();
    name.push(".etag");
    part.with_file_name(name)
}

fn store_part_etag(etag_path: &Path, response: &reqwest::Response) {
    match response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
    {
        Some(etag) => {
            let _ = std::fs::write(etag_path, etag);
        }
        None => {
            let _ = std::fs::remove_file(etag_path);
        }
    }
}

/// Sleep out the backoff, or surface `reason` once the budget is spent.
async fn wait_before_retry(consecutive_failures: u32, reason: String) -> Result<(), String> {
    match backoff(consecutive_failures) {
        Some(delay) => {
            tokio::time::sleep(delay).await;
            Ok(())
        }
        None => Err(reason),
    }
}

/// Announce the verifying phase and hash the `.part`.
///
/// Hashing a gigabyte takes seconds; on a runtime worker that blocks every other task
/// on the executor, including the event pump feeding the UI.
async fn verify_part(
    model_id: &str,
    tmp: &Path,
    transfer: &Transfer,
    on_progress: &mut (dyn FnMut(DownloadProgress) + Send),
) -> Result<String, String> {
    on_progress(DownloadProgress::phase(
        model_id,
        "verifying",
        transfer.bytes,
        Some(transfer.bytes),
        false,
    ));
    let hashed = tmp.to_path_buf();
    tokio::task::spawn_blocking(move || sha256_file(&hashed))
        .await
        .map_err(|e| e.to_string())?
}

/// The outcome of a completed transfer.
struct Transfer {
    bytes: u64,
    /// This run appended to a `.part` the server had no validator for, so nothing
    /// but the final checksum stands between spliced bytes and the model loader.
    unvalidated_resume: bool,
}

/// Stream `url` into `tmp`, resuming from whatever is already there and retrying a
/// dropped connection until the budget runs out.
///
/// **Private, and it stays private.** `download_model_with_progress` is the only
/// entry point and it resolves the URL from the internal catalog; a `pub` here would
/// hand the IPC boundary a way to choose where the bytes come from, and the bytes go
/// straight into a C++ parser.
async fn fetch_to_file(
    client: &reqwest::Client,
    model_id: &str,
    url: &str,
    tmp: &Path,
    size_hint: Option<u64>,
    pacer: &mut ProgressPacer,
    on_progress: &mut (dyn FnMut(DownloadProgress) + Send),
) -> Result<Transfer, String> {
    let etag_path = part_etag_path(tmp);
    let mut consecutive_failures = 0u32;
    let mut attempt = 1u32;
    let mut unvalidated_resume = false;

    loop {
        let part_len = std::fs::metadata(tmp).map(|m| m.len()).unwrap_or(0);
        let mut request = client.get(url);
        let mut validated = false;
        if part_len > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={part_len}-"));
            if let Ok(etag) = std::fs::read_to_string(&etag_path) {
                if !etag.trim().is_empty() {
                    request = request.header(reqwest::header::IF_RANGE, etag.trim());
                    validated = true;
                }
            }
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                consecutive_failures += 1;
                wait_before_retry(consecutive_failures, describe(&error)).await?;
                attempt += 1;
                continue;
            }
        };

        let status = response.status().as_u16();
        let content_range = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let (file, mut downloaded, total) = match plan_resume(
            part_len,
            status,
            response.content_length(),
            content_range.as_deref(),
        ) {
            ResumeAction::Append { from, total } => {
                // A `.part` left by a build that predates the sidecar has no ETag, so
                // the range went out unguarded and the server had nothing to check it
                // against. Appending anyway is the right trade — these URLs are pinned
                // and the catalog checksum is the backstop — but the caller has to know,
                // because a mismatch then means spliced bytes, not a changed artifact,
                // and the answer is to take the whole object rather than to give up.
                unvalidated_resume |= !validated;
                (
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(tmp)
                        .map_err(|e| e.to_string())?,
                    from,
                    total.or(size_hint),
                )
            }
            // Every byte is already there. No frame is emitted: the caller's next act
            // is the "verifying" phase, which is exactly what happens next.
            ResumeAction::AlreadyComplete { total } => {
                return Ok(Transfer {
                    bytes: total,
                    unvalidated_resume: unvalidated_resume || !validated,
                });
            }
            ResumeAction::Restart { total } => {
                store_part_etag(&etag_path, &response);
                (
                    std::fs::File::create(tmp).map_err(|e| e.to_string())?,
                    0,
                    total.or(size_hint),
                )
            }
            ResumeAction::DiscardAndRestart => {
                let _ = std::fs::remove_file(tmp);
                let _ = std::fs::remove_file(&etag_path);
                // Costs a retry slot on purpose: the next request carries no `Range`,
                // so a server that answers 416 again is broken and must not be looped on.
                consecutive_failures += 1;
                wait_before_retry(
                    consecutive_failures,
                    format!("download failed: HTTP {status}"),
                )
                .await?;
                attempt += 1;
                continue;
            }
            ResumeAction::Fail(status) => {
                let reason = format!("download failed: HTTP {status}");
                if !is_retryable_status(status) {
                    return Err(reason);
                }
                consecutive_failures += 1;
                wait_before_retry(consecutive_failures, reason).await?;
                attempt += 1;
                continue;
            }
        };

        let resumed_from = downloaded;
        // Forced: a retry boundary changes what the row says, and the bar would
        // otherwise sit still through the backoff.
        if let Some(sample) = pacer.tick(Instant::now(), downloaded, total, true) {
            on_progress(DownloadProgress::transfer(
                model_id,
                downloaded,
                total,
                sample,
                attempt,
                resumed_from,
            ));
        }

        let mut writer = std::io::BufWriter::with_capacity(1 << 20, file);
        let mut stream = response.bytes_stream();
        let mut transport_error = None;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    transport_error = Some(describe(&error));
                    break;
                }
            };
            writer.write_all(&chunk).map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;
            if let Some(sample) = pacer.tick(Instant::now(), downloaded, total, false) {
                on_progress(DownloadProgress::transfer(
                    model_id,
                    downloaded,
                    total,
                    sample,
                    attempt,
                    resumed_from,
                ));
            }
        }
        writer.flush().map_err(|e| e.to_string())?;
        drop(writer);

        let Some(reason) = transport_error else {
            return Ok(Transfer {
                bytes: downloaded,
                unvalidated_resume,
            });
        };

        // The `.part` file is never removed on this path. Those bytes are the resume
        // point, and throwing them away is what turned one dropped connection into a
        // fresh 276 MB download.
        consecutive_failures = if downloaded > resumed_from {
            0
        } else {
            consecutive_failures + 1
        };
        wait_before_retry(consecutive_failures, reason).await?;
        attempt += 1;
    }
}

/// Streaming SHA-256 of a file — models are hundreds of MB, so never read one whole.
fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hex::encode(hasher.finalize()))
}

fn extract_tar_gz(archive_path: &Path, _out_dir: &Path, dest_file: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);
    extract_first_model_entry(&mut archive, dest_file)
}

fn extract_tar(archive_path: &Path, _out_dir: &Path, dest_file: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let mut archive = Archive::new(file);
    extract_first_model_entry(&mut archive, dest_file)
}

fn extract_first_model_entry<R: std::io::Read>(
    archive: &mut Archive<R>,
    dest_file: &Path,
) -> Result<(), String> {
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?;
        let name = path.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".bin") || name.ends_with(".gguf") || name.ends_with(".onnx") {
            let mut out = std::fs::File::create(dest_file).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err("archive did not contain a .bin/.gguf model file".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn catalog_lists_stt_and_llm_with_direct_urls() {
        let models = list_models();
        assert!(models.iter().any(|m| m.kind == "stt"));
        assert!(models.iter().any(|m| m.kind == "llm"));
        for m in &models {
            let url = m.download_url.as_deref().unwrap_or("");
            assert!(
                url.starts_with("https://") && !url.ends_with(".tar"),
                "expected direct https model URL, got {url}"
            );
            assert!(url.contains("huggingface.co") || url.contains("github.com"));
        }
    }

    #[test]
    fn every_catalog_entry_declares_a_sha256() {
        for m in list_models() {
            assert_eq!(m.sha256.len(), 64, "{} has a malformed digest", m.id);
            assert!(
                m.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{} digest is not hex: {}",
                m.id,
                m.sha256
            );
            assert!(m.size_hint_bytes.unwrap_or(0) > 1_000_000, "{}", m.id);
        }
    }

    #[test]
    fn sha256_file_matches_known_digest() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("abc.bin");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_comparison_rejects_a_different_artifact() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("tampered.bin");
        std::fs::write(&p, b"not the model you asked for").unwrap();
        let expected = list_models()
            .into_iter()
            .find(|m| m.id == "whisper-tiny")
            .unwrap()
            .sha256;
        assert!(!sha256_file(&p).unwrap().eq_ignore_ascii_case(&expected));
    }

    /// The marker records that *this code* checked these bytes once. It is not a
    /// defence against someone who can already write inside the app data directory —
    /// they could rewrite the marker too. It exists to stop artifacts of unknown
    /// provenance, including everything downloaded before checksums existed, from
    /// being handed to a C++ parser.
    #[test]
    fn an_unverified_artifact_is_never_ready() {
        let dir = tempdir().unwrap();
        let artifact = dir.path().join("model.bin");
        std::fs::write(&artifact, vec![0u8; 1_100_000]).unwrap();
        let expected = catalog_sha256("whisper-tiny").unwrap();

        // Big file, no marker: this is the state left by the old download path.
        assert!(!artifact_is_verified(&artifact, "whisper-tiny"));

        // Marker from a different model must not vouch for this one.
        write_artifact_marker(&artifact, &catalog_sha256("whisper-base").unwrap()).unwrap();
        assert!(!artifact_is_verified(&artifact, "whisper-tiny"));

        write_artifact_marker(&artifact, &expected).unwrap();
        assert!(artifact_is_verified(&artifact, "whisper-tiny"));
    }

    #[test]
    fn a_marked_but_tiny_artifact_is_not_ready() {
        let dir = tempdir().unwrap();
        let artifact = dir.path().join("model.bin");
        std::fs::write(&artifact, b"too small to be a model").unwrap();
        write_artifact_marker(&artifact, &catalog_sha256("whisper-tiny").unwrap()).unwrap();
        assert!(!artifact_is_verified(&artifact, "whisper-tiny"));
    }

    /// The reporter's disk: `qwen2.5-0.5b/model.part` at 276 MB of 491 MB, and
    /// an app that says only "not installed".
    #[test]
    fn a_part_file_is_reported_as_partial_bytes() {
        let dir = tempdir().unwrap();
        let artifact = dir.path().join("model.gguf");
        std::fs::write(part_path(&artifact), vec![0u8; 4096]).unwrap();
        assert_eq!(partial_bytes(&artifact), Some(4096));
    }

    #[test]
    fn no_part_file_reports_none() {
        let dir = tempdir().unwrap();
        let artifact = dir.path().join("model.gguf");
        assert_eq!(partial_bytes(&artifact), None);

        // A zero-length leftover is not something to offer to continue.
        std::fs::write(part_path(&artifact), b"").unwrap();
        assert_eq!(partial_bytes(&artifact), None);
    }

    #[test]
    fn a_ready_model_reports_no_partial() {
        // A finished download is renamed into place and the `.part` goes with
        // it, so the answer keys off the leftover and never off the artifact.
        let dir = tempdir().unwrap();
        let artifact = dir.path().join("model.bin");
        std::fs::write(&artifact, vec![0u8; 1_100_000]).unwrap();
        write_artifact_marker(&artifact, &catalog_sha256("whisper-tiny").unwrap()).unwrap();
        assert!(artifact_is_verified(&artifact, "whisper-tiny"));
        assert_eq!(partial_bytes(&artifact), None);
    }

    #[test]
    fn artifact_paths_use_bin_and_gguf() {
        let stt = model_artifact_path("whisper-tiny", "stt");
        assert!(stt.ends_with("model.bin"));
        let llm = model_artifact_path("qwen2.5-0.5b", "llm");
        assert!(llm.ends_with("model.gguf"));
    }

    /// The one test that exercises the whole transfer path. A connection that dies
    /// mid-stream must come back where it left off — the old loop truncated the
    /// `.part` on every attempt, so a drop at 56% cost the user all 276 MB.
    #[tokio::test]
    async fn a_dropped_connection_resumes_from_the_part_file() {
        const TOTAL: usize = 1024 * 1024;
        const CUT: usize = TOTAL / 2;
        let body: Vec<u8> = (0..TOTAL).map(|i| (i % 251) as u8).collect();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = body.clone();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut heads: Vec<String> = Vec::new();
            for request in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    let read = socket.read(&mut buf).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    head.extend_from_slice(&buf[..read]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();

                if request == 0 {
                    socket
                        .write_all(
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {TOTAL}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\n\r\n")
                                .as_bytes(),
                        )
                        .await
                        .unwrap();
                    // Half a body against a declared full length, then the socket
                    // goes away: exactly the shape that printed "error decoding
                    // response body" and nothing else.
                    socket.write_all(&served[..CUT]).await.unwrap();
                } else {
                    let from: usize = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("range: bytes=")
                                .map(str::to_owned)
                        })
                        .and_then(|value| value.trim_end_matches('-').parse().ok())
                        .expect("the retry must carry a Range header");
                    socket
                        .write_all(
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{TOTAL}\r\n\r\n",
                                TOTAL - from,
                                TOTAL - 1
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                    socket.write_all(&served[from..]).await.unwrap();
                }
                socket.shutdown().await.unwrap();
                heads.push(head);
            }
            heads
        });

        let dir = tempdir().unwrap();
        let tmp = dir.path().join("model.part");
        let client = reqwest::Client::builder().build().unwrap();
        let mut pacer = ProgressPacer::new();
        let mut events: Vec<DownloadProgress> = Vec::new();
        let downloaded = fetch_to_file(
            &client,
            "test-model",
            &format!("http://{addr}/model.gguf"),
            &tmp,
            Some(TOTAL as u64),
            &mut pacer,
            &mut |p: DownloadProgress| events.push(p),
        )
        .await
        .unwrap();
        let heads = server.await.unwrap();

        assert_eq!(downloaded.bytes, TOTAL as u64);
        assert!(
            !downloaded.unvalidated_resume,
            "the server sent an ETag, so the resume was guarded by If-Range"
        );
        assert_eq!(
            std::fs::read(&tmp).unwrap(),
            body,
            "the resumed file is not byte-identical to what was served"
        );
        assert!(
            heads[1].contains(&format!("bytes={CUT}-")),
            "the retry started over instead of resuming:\n{}",
            heads[1]
        );
        assert!(
            events
                .iter()
                .any(|e| e.attempt == 2 && e.resumed_from_bytes == CUT as u64),
            "the resume never reached the UI payload"
        );
    }
}
