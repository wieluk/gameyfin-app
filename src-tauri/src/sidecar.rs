//! Helper programs shipped beside the app as Tauri sidecars: Ludusavi everywhere, umu-run
//! on Linux. One lookup, so every package format finds them the same way.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

/// Locates a bundled helper by its plain name. Bundling renames it beside the executable,
/// while a `cargo run` build keeps the target triple, so both spellings are tried.
pub fn bundled(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);

    if let Ok(resources) = app.path().resource_dir() {
        let candidate = resources.join(&file);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Every shipped format puts it beside the executable, inside the mount for an AppImage.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [dir.join(&file), dir.join(file_name(name))] {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    // The development layout, where the fetch scripts put it. A release build must never
    // look at a path baked in on the build machine.
    if cfg!(debug_assertions) {
        let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(file_name(name));
        if checkout.exists() {
            return Some(checkout);
        }
    }
    None
}

/// What the fetch scripts name a sidecar: `<name>-<triple>` plus the executable suffix.
fn file_name(name: &str) -> String {
    let triple = if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    format!("{name}-{triple}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidecar_name_carries_the_target_triple_the_fetch_scripts_use() {
        let name = file_name("umu-run");
        assert!(name.starts_with("umu-run-"), "{name}");
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(name, "umu-run-x86_64-unknown-linux-gnu");
    }
}
