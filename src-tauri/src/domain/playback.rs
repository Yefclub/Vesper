//! Which stored `audio_path` may be handed to the player, and which may not.

use std::path::{Path, PathBuf};

/// Why a meeting's stored recording cannot be played.
///
/// Two variants rather than one because they are two different events. `Missing`
/// is ordinary — an imported meeting whose audio was never retained, a file the
/// user deleted. `Outside` is a row pointing
/// somewhere it has no business pointing, which is worth a line in the log even
/// though the window shows the same thing for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unplayable {
    Missing,
    Outside,
}

/// Resolve a meeting's stored recording, refusing anything that is not inside
/// the app's own recordings directory.
///
/// `audio_path` is a string in a row on the user's disk and the WebView asks for
/// playback by meeting id, so this is the only place that decides which file the
/// player is allowed to open. The same reasoning as `db::delete_recording`, with
/// the consequence reversed: there a tampered row would delete somebody's file,
/// here it would read one out to the window.
///
/// The comparison runs on canonicalised paths, which is the whole point.
/// `Path::starts_with` matches components without resolving them, so
/// `<recordings>/../../.ssh/id_ed25519` satisfies it; and canonicalising also
/// follows symlinks, so a link planted inside the directory is judged by where
/// it lands rather than by where it sits.
///
/// Failing to canonicalise is `Missing`, not an error: it means nothing is there
/// to open, which is the ordinary case for a meeting whose file has gone. That
/// makes the check fail closed — every path this cannot fully resolve is
/// refused.
pub fn playable_recording(root: &Path, stored: &str) -> Result<PathBuf, Unplayable> {
    let (Ok(root), Ok(resolved)) = (root.canonicalize(), Path::new(stored).canonicalize()) else {
        return Err(Unplayable::Missing);
    };
    if !resolved.starts_with(&root) {
        return Err(Unplayable::Outside);
    }
    // Canonicalising already proved something is there; this proves it is a file.
    // A row naming the directory itself would otherwise reach the player as a
    // source that can never load.
    if !resolved.is_file() {
        return Err(Unplayable::Missing);
    }
    Ok(simplified(resolved))
}

/// Drop Windows' verbatim prefix from a canonical path.
///
/// `std::fs::canonicalize` answers in the `\\?\C:\…` form, and that string
/// travels through `convertFileSrc` into a URL the WebView requests. Plain drive
/// paths are the shape every other Tauri asset URL carries, so the player is not
/// the one place that finds out whether the verbatim form survives the round
/// trip. A UNC answer (`\\?\UNC\…`) means something else without its prefix and
/// is left as it is; on any other platform the prefix never appears and this
/// returns what it was given.
fn simplified(path: PathBuf) -> PathBuf {
    path.to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        // `C:\…` rather than `UNC\…`: the second byte of a drive path is a colon.
        .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
        .map(PathBuf::from)
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink as symlink_file;
    #[cfg(windows)]
    use std::os::windows::fs::symlink_file;

    #[test]
    fn a_recording_in_the_directory_resolves() {
        let dir = tempdir().unwrap();
        let wav = dir.path().join("m1.wav");
        std::fs::write(&wav, vec![0u8; 16]).unwrap();

        let played = playable_recording(dir.path(), &wav.display().to_string()).unwrap();
        assert_eq!(played, simplified(wav.canonicalize().unwrap()));
    }

    #[test]
    fn a_traversal_out_of_the_directory_is_refused() {
        // The shape `Path::starts_with` accepts on its own: every component of
        // this path is prefixed by the root, and it still names a file beside it.
        let parent = tempdir().unwrap();
        let root = parent.path().join("recordings");
        std::fs::create_dir_all(&root).unwrap();
        let outsider = parent.path().join("secrets.wav");
        std::fs::write(&outsider, b"not ours").unwrap();

        let traversal = root.join("..").join("secrets.wav");
        assert_eq!(
            playable_recording(&root, &traversal.display().to_string()),
            Err(Unplayable::Outside),
        );
    }

    #[test]
    fn a_symlink_pointing_out_of_the_directory_is_refused() {
        let parent = tempdir().unwrap();
        let root = parent.path().join("recordings");
        std::fs::create_dir_all(&root).unwrap();
        let outsider = parent.path().join("secrets.wav");
        std::fs::write(&outsider, b"not ours").unwrap();

        // Windows only creates symlinks under Developer Mode or elevation. There
        // is nothing to assert about a link the OS refused to make.
        let link = root.join("m1.wav");
        if symlink_file(&outsider, &link).is_err() {
            return;
        }
        assert_eq!(
            playable_recording(&root, &link.display().to_string()),
            Err(Unplayable::Outside),
        );
    }

    #[test]
    fn a_file_that_is_not_there_is_missing_rather_than_an_error() {
        let dir = tempdir().unwrap();
        let gone = dir.path().join("deleted.wav");
        assert_eq!(
            playable_recording(dir.path(), &gone.display().to_string()),
            Err(Unplayable::Missing),
        );
        // A row naming the directory rather than a recording in it.
        assert_eq!(
            playable_recording(dir.path(), &dir.path().display().to_string()),
            Err(Unplayable::Missing),
        );
    }

    #[test]
    fn a_recordings_directory_that_does_not_exist_refuses_everything() {
        // Fail closed: with no root to compare against there is no containment
        // to prove, and the answer must not be "allowed".
        let dir = tempdir().unwrap();
        let wav = dir.path().join("m1.wav");
        std::fs::write(&wav, vec![0u8; 16]).unwrap();

        assert_eq!(
            playable_recording(&dir.path().join("absent"), &wav.display().to_string()),
            Err(Unplayable::Missing),
        );
    }

    #[test]
    fn a_resolved_path_carries_no_verbatim_prefix() {
        // What comes out of here is percent-encoded into an asset URL, so the
        // shape matters beyond the comparison it was canonicalised for.
        let dir = tempdir().unwrap();
        let wav = dir.path().join("m1.wav");
        std::fs::write(&wav, vec![0u8; 16]).unwrap();

        let played = playable_recording(dir.path(), &wav.display().to_string()).unwrap();
        assert!(
            !played.to_string_lossy().starts_with(r"\\?\"),
            "{}",
            played.display()
        );
    }
}
