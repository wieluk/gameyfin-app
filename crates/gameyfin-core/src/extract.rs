//! Unpacking a downloaded game.
//!
//! Formats are detected from the file's own bytes rather than its extension: servers and
//! providers are not consistent about naming, and unpacking the wrong way round produces a
//! confusing failure much later.
//!
//! Archive entry paths are attacker-controlled in principle, they come from a file the
//! server handed us, so every path is checked to stay inside the destination. Without
//! that, an entry named `../../.bashrc` would escape and overwrite files elsewhere.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, CoreResult};

/// Archive kinds we can unpack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    SevenZip,
    /// RAR. Unpacked with an external tool: the only complete RAR implementations carry
    /// licence terms that forbid redistributing them inside another application, so this
    /// uses whatever the system already has.
    Rar,
    /// A plain file that is not an archive, a bare executable or disk image.
    None,
}

impl ArchiveKind {
    pub fn label(self) -> &'static str {
        match self {
            ArchiveKind::Zip => "zip",
            ArchiveKind::SevenZip => "7z",
            ArchiveKind::Rar => "rar",
            ArchiveKind::None => "not an archive",
        }
    }
}

/// Identify an archive by its magic bytes.
pub fn detect(path: &Path) -> CoreResult<ArchiveKind> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 8];
    let read = file.read(&mut magic)?;
    Ok(detect_bytes(&magic[..read]))
}

fn detect_bytes(magic: &[u8]) -> ArchiveKind {
    // Local file header, or an empty/spanned archive.
    if magic.starts_with(b"PK\x03\x04") || magic.starts_with(b"PK\x05\x06") {
        return ArchiveKind::Zip;
    }
    if magic.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return ArchiveKind::SevenZip;
    }
    // "Rar!\x1a\x07\x00" is RAR 1.5-4.x; the 5.x signature adds a byte. Both start the
    // same way, which is enough to identify the format.
    if magic.starts_with(b"Rar!\x1a\x07") {
        return ArchiveKind::Rar;
    }
    ArchiveKind::None
}

/// How far extraction has got.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExtractProgress {
    pub entries_done: u64,
    pub entries_total: u64,
    pub bytes_written: u64,
    /// Total uncompressed size, when the format can tell us. Zero when unknown.
    pub bytes_total: u64,
}

impl ExtractProgress {
    /// Completion, by bytes where possible.
    ///
    /// Entry counts are a poor proxy: a game archive is frequently one enormous file, so
    /// counting entries leaves the bar at zero until it snaps to done. Bytes move
    /// smoothly regardless of how the archive is laid out.
    pub fn percent(&self) -> f64 {
        if self.bytes_total > 0 {
            return ((self.bytes_written as f64 / self.bytes_total as f64) * 100.0)
                .clamp(0.0, 100.0);
        }
        if self.entries_total == 0 {
            return 0.0;
        }
        ((self.entries_done as f64 / self.entries_total as f64) * 100.0).clamp(0.0, 100.0)
    }
}

/// Join an archive entry path onto a destination, refusing anything that escapes.
///
/// Rejects absolute paths, drive prefixes and `..` traversal. Returns `None` for an entry
/// that should be skipped rather than failing the whole extraction, because one hostile or
/// malformed name should not cost the user a completed download.
pub fn safe_join(destination: &Path, entry: &str) -> Option<PathBuf> {
    // Archives use forward slashes, but a Windows-built one may contain backslashes.
    let normalised = entry.replace('\\', "/");
    let candidate = Path::new(&normalised);

    let mut out = destination.to_path_buf();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => out.push(part),
            // Anything else is either meaningless here or an escape attempt.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    // Belt and braces: the result must still be under the destination.
    out.starts_with(destination).then_some(out)
}

/// Unpack `archive` into `destination`, reporting progress.
pub fn extract<F>(archive: &Path, destination: &Path, mut on_progress: F) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    std::fs::create_dir_all(destination)?;

    match detect(archive)? {
        ArchiveKind::Zip => extract_zip(archive, destination, &mut on_progress),
        ArchiveKind::SevenZip => extract_7z(archive, destination, &mut on_progress),
        ArchiveKind::Rar => extract_rar(archive, destination, &mut on_progress),
        ArchiveKind::None => Err(CoreError::UnsupportedArchive {
            path: archive.display().to_string(),
        }),
    }
}

fn extract_zip<F>(archive: &Path, destination: &Path, on_progress: &mut F) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    let file = BufReader::new(File::open(archive)?);
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| CoreError::Other(format!("could not read the zip archive: {e}")))?;

    let entries_total = zip.len() as u64;

    // The central directory knows every uncompressed size, so the total is available
    // before a single byte is written, which is what makes byte-based progress possible.
    let bytes_total: u64 = (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|e| e.size()))
        .sum();

    let mut written = 0u64;
    let mut entries_done = 0u64;

    // Report the starting point immediately so the UI shows a real total rather than
    // sitting blank until the first chunk lands.
    on_progress(ExtractProgress {
        entries_done: 0,
        entries_total,
        bytes_written: 0,
        bytes_total,
    });

    let mut buffer = vec![0u8; 128 * 1024];

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| CoreError::Other(format!("could not read archive entry: {e}")))?;

        let Some(target) = entry
            .enclosed_name()
            .and_then(|name| safe_join(destination, &name.to_string_lossy()))
        else {
            tracing::warn!("skipping unsafe archive entry: {}", entry.name());
            entries_done += 1;
            continue;
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = File::create(&target)?;

            // Copied in chunks rather than with `io::copy` so a single multi-gigabyte
            // entry still reports progress as it goes.
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                std::io::Write::write_all(&mut out, &buffer[..read])?;
                written += read as u64;
                on_progress(ExtractProgress {
                    entries_done,
                    entries_total,
                    bytes_written: written,
                    bytes_total,
                });
            }

            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                // Preserve the executable bit; a game's launcher is useless without it.
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
            }
        }

        entries_done += 1;
        on_progress(ExtractProgress {
            entries_done,
            entries_total,
            bytes_written: written,
            bytes_total,
        });
    }

    Ok(written)
}

fn extract_7z<F>(archive: &Path, destination: &Path, on_progress: &mut F) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    // Entry-by-entry rather than `decompress_file`, which reports nothing until it
    // finishes, leaving a long extraction showing 0% and then jumping straight to done.
    let mut reader = sevenz_rust2::SevenZReader::open(archive, sevenz_rust2::Password::empty())
        .map_err(|e| CoreError::Other(format!("could not read the 7z archive: {e}")))?;

    let entries_total = reader.archive().files.len() as u64;
    let bytes_total: u64 = reader
        .archive()
        .files
        .iter()
        .filter(|f| f.has_stream())
        .map(|f| f.size())
        .sum();

    on_progress(ExtractProgress {
        entries_done: 0,
        entries_total,
        bytes_written: 0,
        bytes_total,
    });

    let mut written = 0u64;
    let mut entries_done = 0u64;
    let destination = destination.to_path_buf();

    reader
        .for_each_entries(|entry, reader| {
            entries_done += 1;

            let Some(target) = safe_join(&destination, entry.name()) else {
                tracing::warn!("skipping unsafe archive entry: {}", entry.name());
                return Ok(true);
            };

            if entry.is_directory() {
                std::fs::create_dir_all(&target)?;
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::File::create(&target)?;

                // Copied in chunks rather than in one go: a 7z holding a single large
                // file, which is exactly what a game installer archive usually is,
                // would otherwise report nothing until the whole thing had been written.
                let mut buffer = vec![0u8; 256 * 1024];
                loop {
                    let read = reader.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    std::io::Write::write_all(&mut out, &buffer[..read])?;
                    written += read as u64;
                    on_progress(ExtractProgress {
                        entries_done,
                        entries_total,
                        bytes_written: written,
                        bytes_total,
                    });
                }
            }

            on_progress(ExtractProgress {
                entries_done,
                entries_total,
                bytes_written: written,
                bytes_total,
            });
            Ok(true)
        })
        .map_err(|e| CoreError::Other(format!("could not extract the 7z archive: {e}")))?;

    Ok(written)
}

/// External programs that can unpack a RAR, in order of preference.
///
/// `unar` and `7z` handle RAR 5 correctly and are packaged by every major distribution;
/// `unrar` is the reference implementation where it happens to be installed.
const RAR_TOOLS: &[(&str, &[&str])] = &[
    ("unar", &["-force-overwrite", "-quiet", "-output-directory"]),
    ("7zz", &["x", "-y", "-o"]),
    ("7z", &["x", "-y", "-o"]),
    ("7za", &["x", "-y", "-o"]),
    ("unrar", &["x", "-y"]),
    ("bsdtar", &["-xf"]),
];

/// Find an installed tool that can unpack RAR archives.
pub fn rar_tool() -> Option<&'static str> {
    RAR_TOOLS
        .iter()
        .map(|(name, _)| *name)
        .find(|name| crate::runtime::find_program(name).is_some())
}

/// What to install when no RAR tool is present.
pub fn rar_tool_hint() -> String {
    "RAR archives need an external tool. Install one of unar, p7zip (7z) or unrar, \
     for example `sudo dnf install unar` or `sudo apt install unar`."
        .to_string()
}

fn extract_rar<F>(archive: &Path, destination: &Path, on_progress: &mut F) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    let Some(tool) = rar_tool() else {
        return Err(CoreError::Other(rar_tool_hint()));
    };
    let (_, flags) = RAR_TOOLS
        .iter()
        .find(|(name, _)| *name == tool)
        .expect("tool came from the same table");

    let program =
        crate::runtime::find_program(tool).ok_or_else(|| CoreError::Other(rar_tool_hint()))?;

    let mut command = std::process::Command::new(&program);
    // Piped so the child does not inherit our stdio, and so failures can be reported.
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    for flag in *flags {
        // The 7-Zip family joins the output directory to its flag with no separator.
        if *flag == "-o" {
            command.arg(format!("-o{}", destination.display()));
        } else {
            command.arg(flag);
        }
    }
    match tool {
        "unar" => {
            command.arg(destination).arg(archive);
        }
        "unrar" | "bsdtar" => {
            // These extract into the working directory rather than taking an output flag.
            command.arg(archive).current_dir(destination);
        }
        _ => {
            command.arg(archive);
        }
    }

    tracing::info!(
        tool,
        ?archive,
        ?destination,
        "unpacking RAR with an external tool"
    );

    // `bsdtar` is the last entry in `RAR_TOOLS` because libarchive's RAR support is
    // partial. Reaching it means nothing better is installed, and the result can be an
    // archive that unpacks "successfully" into files that are not what it contained, so
    // say so here rather than leaving it to be inferred from a game that will not start.
    if tool == "bsdtar" {
        tracing::warn!(
            "no full RAR tool found, falling back to bsdtar; install unar or 7z if this \
             archive unpacks incorrectly"
        );
    }

    // These tools report nothing usable on stdout, so progress is sampled from the
    // destination instead: the compressed archive size is a rough but honest stand-in for
    // the total, and a moving bar beats one stuck at zero for several minutes.
    let expected = std::fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel::<ExtractProgress>();

    let sampler = {
        let finished = finished.clone();
        let destination = destination.to_path_buf();
        std::thread::spawn(move || {
            while !finished.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(400));
                let written = directory_size(&destination).unwrap_or(0);
                if tx
                    .send(ExtractProgress {
                        entries_done: 0,
                        entries_total: 0,
                        bytes_written: written,
                        bytes_total: expected,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    };

    // `wait_with_output` runs on its own thread so the child's pipes are drained
    // continuously. Polling `try_wait` while leaving them full would deadlock the moment
    // the tool wrote more than a pipe buffer's worth, which a verbose extractor does
    // almost immediately.
    let child = command
        .spawn()
        .map_err(|e| CoreError::Other(format!("could not run {tool}: {e}")))?;
    let waiter = std::thread::spawn(move || child.wait_with_output());

    // Report samples until the tool finishes.
    while !waiter.is_finished() {
        if let Ok(progress) = rx.recv_timeout(std::time::Duration::from_millis(500)) {
            on_progress(progress);
        }
    }

    finished.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = sampler.join();

    let output = waiter
        .join()
        .map_err(|_| CoreError::Other(format!("{tool} stopped unexpectedly")))?
        .map_err(|e| CoreError::Other(format!("could not run {tool}: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CoreError::Other(format!(
            "{tool} could not unpack the archive: {}",
            stderr.trim()
        )));
    }

    // The external tools give no usable progress, so this reports one step on completion.
    let written = directory_size(destination).unwrap_or(0);
    on_progress(ExtractProgress {
        entries_done: 1,
        entries_total: 1,
        bytes_written: written,
        bytes_total: written,
    });
    Ok(written)
}

/// Total size of everything under a directory.
pub fn directory_size(path: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += directory_size(&entry.path()).unwrap_or(0);
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-extract-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_zip_from_magic_bytes() {
        assert_eq!(detect_bytes(b"PK\x03\x04..."), ArchiveKind::Zip);
        assert_eq!(detect_bytes(b"PK\x05\x06"), ArchiveKind::Zip);
    }

    #[test]
    fn detects_7z_from_magic_bytes() {
        assert_eq!(
            detect_bytes(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]),
            ArchiveKind::SevenZip
        );
    }

    #[test]
    fn detects_rar_by_signature() {
        // The case from the field: a game that arrives as a RAR with no file extension.
        assert_eq!(detect_bytes(b"Rar!\x1a\x07\x00"), ArchiveKind::Rar);
        // RAR 5 differs only after the shared prefix.
        assert_eq!(detect_bytes(b"Rar!\x1a\x07\x01\x00"), ArchiveKind::Rar);
    }

    #[test]
    fn an_executable_is_not_an_archive() {
        // A bare .exe download must be recognised as such, not mis-extracted.
        assert_eq!(detect_bytes(b"MZ\x90\x00"), ArchiveKind::None);
        assert_eq!(detect_bytes(b""), ArchiveKind::None);
    }

    #[test]
    fn safe_join_keeps_normal_paths() {
        let dest = Path::new("/games/celeste");
        assert_eq!(
            safe_join(dest, "bin/Celeste.exe"),
            Some(PathBuf::from("/games/celeste/bin/Celeste.exe"))
        );
    }

    #[test]
    fn safe_join_rejects_traversal() {
        let dest = Path::new("/games/celeste");
        assert_eq!(safe_join(dest, "../../etc/passwd"), None);
        assert_eq!(safe_join(dest, "a/../../../etc/passwd"), None);
    }

    #[test]
    fn safe_join_rejects_absolute_paths() {
        let dest = Path::new("/games/celeste");
        assert_eq!(safe_join(dest, "/etc/passwd"), None);
    }

    #[test]
    fn safe_join_normalises_windows_separators() {
        let dest = Path::new("/games/celeste");
        assert_eq!(
            safe_join(dest, "bin\\x64\\Celeste.exe"),
            Some(PathBuf::from("/games/celeste/bin/x64/Celeste.exe"))
        );
    }

    #[test]
    fn safe_join_ignores_current_dir_segments() {
        let dest = Path::new("/games/celeste");
        assert_eq!(
            safe_join(dest, "./bin/./Celeste.exe"),
            Some(PathBuf::from("/games/celeste/bin/Celeste.exe"))
        );
    }

    #[test]
    fn progress_prefers_bytes_over_entry_counts() {
        // The case that broke the bar: one huge entry, so entry counts read 0% throughout.
        let p = ExtractProgress {
            entries_done: 0,
            entries_total: 1,
            bytes_written: 512,
            bytes_total: 1024,
        };
        assert_eq!(p.percent(), 50.0);
    }

    #[test]
    fn progress_falls_back_to_entries_when_sizes_are_unknown() {
        let p = ExtractProgress {
            entries_done: 5,
            entries_total: 10,
            bytes_written: 0,
            bytes_total: 0,
        };
        assert_eq!(p.percent(), 50.0);
    }

    #[test]
    fn progress_percent_is_bounded() {
        assert_eq!(ExtractProgress::default().percent(), 0.0);

        let over = ExtractProgress {
            entries_done: 3,
            entries_total: 1,
            bytes_written: 2048,
            bytes_total: 1024,
        };
        assert_eq!(over.percent(), 100.0);
    }

    #[test]
    fn extracts_a_real_zip_with_progress() {
        let dir = scratch("zip");
        let archive = dir.join("game.zip");

        {
            let file = File::create(&archive).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("readme.txt", options).unwrap();
            std::io::Write::write_all(&mut writer, b"hello").unwrap();
            writer.start_file("bin/game.exe", options).unwrap();
            std::io::Write::write_all(&mut writer, b"binary").unwrap();
            writer.finish().unwrap();
        }

        let dest = dir.join("out");
        let mut samples = Vec::new();
        let written = extract(&archive, &dest, |p| samples.push(p)).unwrap();

        assert_eq!(written, 11);
        // Progress must start from a known total, not appear only at the end.
        assert_eq!(samples.first().unwrap().bytes_total, 11);
        assert!(samples.len() > 1, "expected progress during extraction");
        assert_eq!(
            std::fs::read_to_string(dest.join("readme.txt")).unwrap(),
            "hello"
        );
        assert!(dest.join("bin/game.exe").exists());
        assert_eq!(samples.last().unwrap().percent(), 100.0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_non_archive_is_reported_clearly() {
        let dir = scratch("plain");
        let file = dir.join("game.exe");
        std::fs::write(&file, b"MZ\x90\x00 not an archive").unwrap();

        let err = extract(&file, &dir.join("out"), |_| {}).unwrap_err();
        assert!(
            matches!(err, CoreError::UnsupportedArchive { .. }),
            "got {err:?}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directory_size_sums_nested_files() {
        let dir = scratch("size");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/one.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("a/b/two.bin"), vec![0u8; 50]).unwrap();

        assert_eq!(directory_size(&dir).unwrap(), 150);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
