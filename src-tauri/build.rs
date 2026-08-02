use std::path::{Path, PathBuf};

fn main() {
    stage_ggml_backends();
    tauri_build::build()
}

/// Where the bundler will look for ggml's backend libraries.
const STAGING: &str = "gpu-backends";

/// Copy ggml's loadable backends somewhere the installer can find them.
///
/// With `dynamic-backends` on, Vulkan, CUDA and every CPU variant are separate
/// shared libraries that ggml opens at runtime from the executable's own
/// directory. Cargo leaves them in the sys crate's `OUT_DIR`, which is a path
/// inside the build tree — fine for `cargo run`, and gone the moment the app is
/// installed on someone else's machine.
///
/// Staging them into a fixed directory is what lets `tauri.conf.json` name them
/// as resources, and this runs during the cargo build, which is before the
/// bundler collects them.
fn stage_ggml_backends() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let staging = manifest.join(STAGING);
    // Cleared every time. A stale `ggml-vulkan.dll` left over from an earlier
    // build would otherwise be shipped inside a CPU-only installer, where it
    // would be loaded and then fail against a driver nobody tested with.
    let _ = std::fs::remove_dir_all(&staging);

    std::fs::create_dir_all(&staging).expect("creating the backend staging directory");

    let mut staged: Vec<String> = Vec::new();
    for source in library_dirs() {
        let Ok(entries) = std::fs::read_dir(&source) else {
            continue;
        };
        for entry in entries.flatten() {
            let from = entry.path();
            let is_library = from
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "dll" | "so" | "dylib"));
            if !is_library {
                continue;
            }
            let to = staging.join(entry.file_name());
            // Failing loudly: a bundle missing these starts and then cannot load
            // any model at all, which is a far worse thing to discover.
            std::fs::copy(&from, &to)
                .unwrap_or_else(|e| panic!("copying {} to {}: {e}", from.display(), to.display()));
            staged.push(entry.file_name().to_string_lossy().to_string());
        }
    }

    // Always written, and this is the reason the directory is never empty: the
    // bundler resolves `gpu-backends/*` as a glob and treats a pattern that
    // matches nothing as a hard error, which would break every build without a
    // GPU feature — that is, the default one.
    //
    // It also earns its place. Next to the installed executable it answers, with
    // no tooling, the first question any report about speed raises: what could
    // this copy of the app actually have used?
    staged.sort();
    let manifest_text = if staged.is_empty() {
        "This build carries no loadable ggml backends.\n\
         Local models run on the CPU backend linked into the binary.\n"
            .to_string()
    } else {
        format!(
            "Loadable ggml backends shipped with this build:\n\n{}\n",
            staged.join("\n")
        )
    };
    std::fs::write(staging.join("BACKENDS.txt"), manifest_text)
        .expect("writing the backend manifest");
}

/// The directories the sys crate left shared libraries in.
///
/// Two of them, for two different reasons. `bin` holds llama and ggml
/// themselves, which `dynamic-link` turns into libraries the binary imports at
/// load time — without them the app does not start at all, GPU or no GPU.
/// `backends` holds the ones `dynamic-backends` produces, which ggml opens
/// later by scanning the executable's directory.
///
/// Found rather than read from `DEP_LLAMA_BACKENDS_DIR`: that variable is only
/// handed to a *direct* dependent of the crate declaring `links`, and this
/// crate reaches llama-cpp-sys-2 through llama-cpp-2.
fn library_dirs() -> Vec<PathBuf> {
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return Vec::new();
    };
    // `OUT_DIR` is `<profile>/build/<pkg>-<hash>/out`, so the sibling build
    // directories are two levels up. Derived rather than assumed: the profile
    // directory moves with `--target` and with `CARGO_TARGET_DIR`.
    let Some(build_root) = PathBuf::from(out_dir)
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&build_root) else {
        return Vec::new();
    };

    // One profile directory holds a `llama-cpp-sys-2-<hash>` output per feature
    // set ever built there, and they all look alike from here. Picking by
    // timestamp does not work: a cached build leaves its directory untouched, so
    // a CPU-only package built after a GPU one would inherit `ggml-vulkan` and
    // ship a backend it was never linked for.
    //
    // The features this build was asked for are the discriminator, and cargo
    // hands them over directly.
    let want_vulkan = std::env::var_os("CARGO_FEATURE_GPU_VULKAN").is_some();
    let want_cuda = std::env::var_os("CARGO_FEATURE_GPU_CUDA").is_some();

    let mut chosen = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_llama = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("llama-cpp-sys-2-"));
        if !is_llama || !holds_ggml(&path.join("out").join("bin")) {
            continue;
        }
        let backends = path.join("out").join("backends");
        if holds_prefix(&backends, "ggml-vulkan") == want_vulkan
            && holds_prefix(&backends, "ggml-cuda") == want_cuda
        {
            chosen = Some(path);
            break;
        }
    }

    let Some(chosen) = chosen else {
        // Not fatal on its own: a build with no GPU feature and no shared
        // libraries has nothing to stage, and `BACKENDS.txt` will say so.
        return Vec::new();
    };
    let out = chosen.join("out");
    ["bin", "backends"]
        .into_iter()
        .map(|name| out.join(name))
        .filter(|dir| holds_ggml(dir))
        .collect()
}

/// Whether a directory is one of ggml's own output directories.
///
/// Matching on the `ggml` prefix rather than on a specific file: which
/// libraries exist depends on the features, and the point here is only to
/// avoid picking up some unrelated crate's build output.
fn holds_ggml(path: &Path) -> bool {
    holds_prefix(path, "ggml")
}

/// By prefix and not by full name, because the extension is `.dll`, `.so` or
/// `.dylib` depending on where this runs.
fn holds_prefix(path: &Path, prefix: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(prefix))
    })
}
