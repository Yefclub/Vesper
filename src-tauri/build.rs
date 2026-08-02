use std::path::PathBuf;

fn main() {
    stage_ggml_backends();
    find_libraries_beside_the_app();
    tauri_build::build()
}

/// Teach the Linux loader where the bundled libraries are.
///
/// llama and ggml are imported at load time, and on Linux nothing points at
/// them: Tauri installs bundle resources under `/usr/lib/Vesper` while the
/// executable is `/usr/bin/vesper`, and neither is a default search path. The
/// app would install cleanly and then fail to start.
///
/// `$ORIGIN` is resolved by the dynamic loader at run time, not by a shell, so
/// it survives being passed through verbatim. Both entries earn their place —
/// the resource directory is where a deb, an rpm and an AppImage all put them,
/// and `$ORIGIN` itself covers a binary run straight out of a build tree.
///
/// Windows needs none of this: a DLL beside the executable is already the first
/// place it looks, which is exactly where the bundle puts them.
fn find_libraries_beside_the_app() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/Vesper");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
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

/// The directories the sys crate left shared libraries in, named by cargo.
///
/// Two of them, for two different reasons. `bin` holds llama and ggml
/// themselves, which `dynamic-link` turns into libraries the binary imports at
/// load time — without them the app does not start at all, GPU or no GPU.
/// `backends` holds the ones `dynamic-backends` produces, which ggml opens
/// later by scanning the executable's directory.
///
/// Both come from the `links` metadata llama-cpp-sys-2 emits, which is why that
/// crate is a direct dependency. Searching the profile directory instead does
/// not work: it can hold one build output per feature set and per crate version
/// ever built there, they are told apart only by a hash, and staging the wrong
/// one puts a `llama.dll` beside the executable that its import library does
/// not match — an app that installs cleanly and then will not start.
fn library_dirs() -> Vec<PathBuf> {
    // Set only when the sys crate built shared libraries, which is to say only
    // where `dynamic-link` is on. Everywhere else there is nothing to stage.
    println!("cargo:rerun-if-env-changed=DEP_LLAMA_ROOT");
    println!("cargo:rerun-if-env-changed=DEP_LLAMA_BACKENDS_DIR");

    let mut dirs = Vec::new();
    if let Ok(root) = std::env::var("DEP_LLAMA_ROOT") {
        dirs.push(PathBuf::from(root).join("bin"));
    }
    if let Ok(backends) = std::env::var("DEP_LLAMA_BACKENDS_DIR") {
        dirs.push(PathBuf::from(backends));
    }
    dirs.retain(|dir| dir.is_dir());
    dirs
}
