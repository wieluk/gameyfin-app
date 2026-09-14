//! Unpacking a downloaded game. Formats come from the file's own bytes, since naming is
//! inconsistent, and every entry path is checked to stay inside the destination.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, CoreResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    SevenZip,
    /// RAR. Unpacked with an external tool: complete RAR implementations cannot be redistributed.
    Rar,
    /// A tar, optionally compressed. How Linux builds of a game usually arrive.
    Tar(TarCompression),
    /// A plain file that is not an archive, a bare executable or disk image.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TarCompression {
    None,
    Gzip,
    Xz,
    Zstd,
    Bzip2,
}

impl ArchiveKind {
    pub fn label(self) -> &'static str {
        match self {
            ArchiveKind::Zip => "zip",
            ArchiveKind::SevenZip => "7z",
            ArchiveKind::Rar => "rar",
            ArchiveKind::Tar(TarCompression::None) => "tar",
            ArchiveKind::Tar(TarCompression::Gzip) => "tar.gz",
            ArchiveKind::Tar(TarCompression::Xz) => "tar.xz",
            ArchiveKind::Tar(TarCompression::Zstd) => "tar.zst",
            ArchiveKind::Tar(TarCompression::Bzip2) => "tar.bz2",
            ArchiveKind::None => "not an archive",
        }
    }
}

pub fn detect(path: &Path) -> CoreResult<ArchiveKind> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 8];
    let read = file.read(&mut magic)?;
    let kind = detect_bytes(&magic[..read]);
    if kind != ArchiveKind::None {
        return Ok(kind);
    }

    // A plain tar has no signature at offset zero; the `ustar` marker is at byte 257. Checked last.
    if file.seek(SeekFrom::Start(TAR_MAGIC_OFFSET)).is_ok() {
        let mut marker = [0u8; 5];
        if file.read_exact(&mut marker).is_ok() && &marker == b"ustar" {
            return Ok(ArchiveKind::Tar(TarCompression::None));
        }
    }

    Ok(ArchiveKind::None)
}

const TAR_MAGIC_OFFSET: u64 = 257;

pub(crate) fn detect_bytes(magic: &[u8]) -> ArchiveKind {
    // Local file header, or an empty/spanned archive.
    if magic.starts_with(b"PK\x03\x04") || magic.starts_with(b"PK\x05\x06") {
        return ArchiveKind::Zip;
    }
    if magic.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return ArchiveKind::SevenZip;
    }
    // RAR 1.5-4.x and 5.x share this prefix, which is enough to identify the format.
    if magic.starts_with(b"Rar!\x1a\x07") {
        return ArchiveKind::Rar;
    }
    // Compression wrappers; the contents are assumed to be a tar.
    if magic.starts_with(&[0x1F, 0x8B]) {
        return ArchiveKind::Tar(TarCompression::Gzip);
    }
    if magic.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        return ArchiveKind::Tar(TarCompression::Xz);
    }
    if magic.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return ArchiveKind::Tar(TarCompression::Zstd);
    }
    if magic.starts_with(b"BZh") {
        return ArchiveKind::Tar(TarCompression::Bzip2);
    }
    ArchiveKind::None
}

/// The archive a download can be unpacked from as it arrives. A 7z keeps its index at the end
/// and a rar needs an external tool, so both wait until the file is complete.
pub fn streamable_kind(head: &[u8]) -> Option<ArchiveKind> {
    let marker = TAR_MAGIC_OFFSET as usize;
    match detect_bytes(head) {
        ArchiveKind::Zip => crate::zip_stream::is_streamable(head).then_some(ArchiveKind::Zip),
        kind @ ArchiveKind::Tar(_) => Some(kind),
        _ => (head.get(marker..marker + 5) == Some(&b"ustar"[..]))
            .then_some(ArchiveKind::Tar(TarCompression::None)),
    }
}

/// Unpacks a download as it arrives, for a kind [`streamable_kind`] accepted.
pub fn unpack_stream<R: Read>(
    kind: ArchiveKind,
    reader: R,
    destination: &Path,
    cancel: &crate::download::Cancel,
) -> CoreResult<u64> {
    match kind {
        ArchiveKind::Zip => crate::zip_stream::extract_stream(reader, destination, cancel),
        ArchiveKind::Tar(compression) => unpack_tar(reader, destination, compression, |_| {
            if cancel.is_cancelled() {
                Err(CoreError::Cancelled)
            } else {
                Ok(())
            }
        }),
        other => Err(CoreError::CannotStream(format!(
            "a {} cannot be unpacked while it downloads",
            other.label()
        ))),
    }
}

/// Whether a download is BitTorrent metainfo rather than the game: the torrent provider
/// answers with a 40 KB `.torrent` that opens as nothing.
pub fn is_torrent_metainfo(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut head = [0u8; 1024];
    let Ok(read) = file.read(&mut head) else {
        return false;
    };
    is_torrent_bytes(&head[..read])
}

pub(crate) fn is_torrent_bytes(head: &[u8]) -> bool {
    // Bencode has no magic number, so this leans on the shape: a metainfo file is one
    // dictionary, and every torrent has an `info` key whatever else it carries.
    head.first() == Some(&b'd') && head.windows(6).any(|w| w == b"4:info" || w == b"8:announ")
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExtractProgress {
    pub entries_done: u64,
    pub entries_total: u64,
    pub bytes_written: u64,
    /// Total uncompressed size, when the format can tell us. Zero when unknown.
    pub bytes_total: u64,
}

impl ExtractProgress {
    /// Completion, by bytes where possible: a game archive is often one huge file, so entry
    /// counts leave the bar stuck at zero.
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

/// Join an archive entry path onto a destination, returning `None` (skip the entry, don't
/// fail the extraction) for absolute paths, drive prefixes or `..` traversal.
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

/// Keep only the executable bit an archive asked for: setuid, setgid and group or world
/// write bits have no place in a game's files.
#[cfg(unix)]
pub(crate) fn set_mode(target: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;

    let executable = if mode & 0o111 != 0 { 0o111 } else { 0 };
    let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o644 | executable));
}

pub fn extract<F>(archive: &Path, destination: &Path, on_progress: F) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    extract_with(archive, destination, None, on_progress)
}

/// Unpack an archive that may be encrypted. The optional password applies to every format
/// that has one and is harmless for an archive that turns out not to need it.
pub fn extract_with<F>(
    archive: &Path,
    destination: &Path,
    password: Option<&str>,
    mut on_progress: F,
) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    std::fs::create_dir_all(destination)?;

    match detect(archive)? {
        ArchiveKind::Zip => extract_zip(archive, destination, password, &mut on_progress),
        ArchiveKind::SevenZip => extract_7z(archive, destination, password, &mut on_progress),
        ArchiveKind::Rar => extract_rar(archive, destination, &mut on_progress),
        ArchiveKind::Tar(compression) => {
            extract_tar(archive, destination, compression, &mut on_progress)
        }
        ArchiveKind::None => Err(CoreError::UnsupportedArchive {
            path: archive.display().to_string(),
        }),
    }
}

fn extract_zip<F>(
    archive: &Path,
    destination: &Path,
    password: Option<&str>,
    on_progress: &mut F,
) -> CoreResult<u64>
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
        let mut entry = match password {
            Some(password) => zip.by_index_decrypt(index, password.as_bytes()),
            None => zip.by_index(index),
        }
        .map_err(|e| match e {
            // Worth saying plainly: the difference between a corrupt download and a
            // missing password is not obvious from "could not read archive entry".
            zip::result::ZipError::UnsupportedArchive(zip::result::ZipError::PASSWORD_REQUIRED) => {
                CoreError::Other(
                    "this archive is password protected. Set the password in Settings, under \
                 Downloads, and try again."
                        .to_string(),
                )
            }
            e => CoreError::Other(format!("could not read archive entry: {e}")),
        })?;

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

            // Chunked so a single multi-gigabyte entry still reports progress.
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
                set_mode(&target, mode);
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

fn extract_7z<F>(
    archive: &Path,
    destination: &Path,
    password: Option<&str>,
    on_progress: &mut F,
) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    // Entry-by-entry rather than `decompress_file`, which reports nothing until it
    // finishes, leaving a long extraction showing 0% and then jumping straight to done.
    let key = match password {
        Some(password) => sevenz_rust2::Password::from(password),
        None => sevenz_rust2::Password::empty(),
    };
    let mut reader = sevenz_rust2::SevenZReader::open(archive, key)
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

                // Chunked: a 7z holding one large file would otherwise report nothing until done.
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

/// A reader that counts what passes through it. Wrapped around the file rather than the
/// decompressed stream, whose total nothing knows in advance.
struct Counting<R> {
    inner: R,
    read: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.read
            .fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(read)
    }
}

/// Unpack a tar file. Progress counts bytes read from the file, since a tar has no index.
fn extract_tar<F>(
    archive: &Path,
    destination: &Path,
    compression: TarCompression,
    on_progress: &mut F,
) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    let bytes_total = std::fs::metadata(archive)?.len();
    let consumed = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let file = Counting {
        inner: BufReader::new(File::open(archive)?),
        read: consumed.clone(),
    };
    unpack_tar(file, destination, compression, |entries_done| {
        on_progress(ExtractProgress {
            entries_done,
            entries_total: 0,
            bytes_written: consumed.load(std::sync::atomic::Ordering::Relaxed),
            bytes_total,
        });
        Ok(())
    })
}

/// Unpack a tar from any reader, decompressing on the way. `after_chunk` runs as data lands and
/// can stop the unpack by returning an error.
fn unpack_tar<'a, R: Read + 'a>(
    reader: R,
    destination: &Path,
    compression: TarCompression,
    mut after_chunk: impl FnMut(u64) -> CoreResult<()>,
) -> CoreResult<u64> {
    let stream: Box<dyn Read + 'a> = match compression {
        TarCompression::None => Box::new(reader),
        TarCompression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(reader)),
        TarCompression::Xz => Box::new(liblzma::read::XzDecoder::new(reader)),
        TarCompression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(reader)),
        TarCompression::Zstd => Box::new(
            zstd::stream::read::Decoder::new(reader)
                .map_err(|e| CoreError::Other(format!("could not read the zstd stream: {e}")))?,
        ),
    };

    let mut tar = tar::Archive::new(stream);
    // Ownership and timestamps mean nothing here; the executable bit is set explicitly below.
    tar.set_preserve_permissions(false);
    tar.set_preserve_mtime(false);
    after_chunk(0)?;

    let entries = tar
        .entries()
        .map_err(|e| CoreError::Other(format!("could not read the tar archive: {e}")))?;

    let mut written = 0u64;
    let mut entries_done = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];

    for entry in entries {
        // A bad first header means the compressed data was never a tar at all.
        let mut entry = entry.map_err(|e| match entries_done {
            0 => CoreError::CannotStream(format!("not a tar archive: {e}")),
            _ => CoreError::Other(format!("could not read a tar entry: {e}")),
        })?;
        let name = entry
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let Some(target) = safe_join(destination, &name) else {
            tracing::warn!("skipping unsafe archive entry: {name}");
            entries_done += 1;
            continue;
        };

        let kind = entry.header().entry_type();

        if kind.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if kind.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = File::create(&target)?;

            // Chunked so a single multi-gigabyte entry still reports progress.
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                std::io::Write::write_all(&mut out, &buffer[..read])?;
                written += read as u64;
                after_chunk(entries_done)?;
            }

            #[cfg(unix)]
            if let Ok(mode) = entry.header().mode() {
                set_mode(&target, mode);
            }
        } else if kind.is_symlink() {
            symlink_entry(destination, &target, &entry);
        } else {
            // Character devices, fifos and the like have no business in a game archive,
            // and creating them needs privileges this app does not want.
            tracing::warn!("skipping {name}: not a file, directory or link");
        }

        entries_done += 1;
        after_chunk(entries_done)?;
    }

    Ok(written)
}

/// Recreate a symlink from a tar entry (Linux builds ship these for shared libraries), if
/// it stays inside the destination.
fn symlink_entry<R: Read>(destination: &Path, target: &Path, entry: &tar::Entry<'_, R>) {
    let Ok(Some(link)) = entry.link_name() else {
        return;
    };

    // A relative link resolves against the link's own directory, not the destination root.
    let base = target.parent().unwrap_or(destination);
    let Some(resolved) = resolve_link(destination, base, &link) else {
        tracing::warn!(
            "skipping link {} -> {}: it points outside the folder",
            target.display(),
            link.display()
        );
        return;
    };

    let _ = std::fs::remove_file(target);
    place_link(&link, &resolved, target);
}

/// Resolve a link target without touching the disk. Allows `..`, which shared-library links
/// use, but never an absolute target or one outside `destination`.
fn resolve_link(destination: &Path, base: &Path, link: &Path) -> Option<PathBuf> {
    let normalised = link.to_string_lossy().replace('\\', "/");

    let mut out = base.to_path_buf();
    for component in Path::new(&normalised).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                // Never above the destination, whatever the archive claims.
                if out == destination || !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    out.starts_with(destination).then_some(out)
}

#[cfg(unix)]
fn place_link(link: &Path, _resolved: &Path, target: &Path) {
    // Recreated as written so the game sees the layout its archive described.
    if let Err(e) = std::os::unix::fs::symlink(link, target) {
        tracing::warn!("could not create link {}: {e}", target.display());
    }
}

/// On Windows, where creating a symlink needs elevated rights, copy the target instead.
#[cfg(not(unix))]
fn place_link(_link: &Path, resolved: &Path, target: &Path) {
    if let Err(e) = std::fs::copy(resolved, target) {
        tracing::warn!(
            "could not copy {} for link {}: {e}",
            resolved.display(),
            target.display()
        );
    }
}

/// External programs that can unpack a RAR, in order of preference.
#[cfg(not(windows))]
const RAR_TOOLS: &[(&str, &[&str])] = &[
    ("unar", &["-force-overwrite", "-quiet", "-output-directory"]),
    ("7zz", &["x", "-y", "-o"]),
    ("7z", &["x", "-y", "-o"]),
    ("7za", &["x", "-y", "-o"]),
    ("unrar", &["x", "-y"]),
    ("bsdtar", &["-xf"]),
];

/// The same, on Windows. None are on `PATH`, so they are found in their install folders
/// (see [`crate::runtime::find_program`]); `tar.exe` is last because its RAR support is partial.
#[cfg(windows)]
const RAR_TOOLS: &[(&str, &[&str])] = &[
    ("7z", &["x", "-y", "-o"]),
    ("7zz", &["x", "-y", "-o"]),
    ("NanaZipC", &["x", "-y", "-o"]),
    ("unrar", &["x", "-y"]),
    ("unar", &["-force-overwrite", "-quiet", "-output-directory"]),
    ("bsdtar", &["-xf"]),
    ("tar", &["-xf"]),
];

/// Tools that unpack into the working directory instead of taking an output flag.
const EXTRACTS_INTO_WORKING_DIR: &[&str] = &["unrar", "bsdtar", "tar"];

/// Every installed tool from [`RAR_TOOLS`], in preference order: installed is not able, and
/// Debian's `p7zip` ships without the RAR codec.
fn installed_rar_tools() -> Vec<(&'static str, &'static [&'static str])> {
    RAR_TOOLS
        .iter()
        .filter(|(name, _)| crate::runtime::find_program(name).is_some())
        .map(|(name, flags)| (*name, *flags))
        .collect()
}

/// What to install when no RAR tool is present. RAR cannot be unpacked in-process (the
/// complete implementations forbid bundling), so this names a tool the user installs.
pub fn rar_tool_hint() -> String {
    if cfg!(windows) {
        return "RAR archives need 7-Zip or WinRAR. Install 7-Zip from https://7-zip.org \
                (or WinRAR from https://rarlab.com), then try again. Gameyfin finds it \
                automatically, with nothing added to PATH."
            .to_string();
    }

    // `unar` first: it needs no separate codec, which is exactly what the packaged 7z on
    // Debian and Ubuntu is missing.
    format!(
        "RAR archives need an external tool. Install unar, for example `{}`. The 7z in \
         Debian's and Ubuntu's p7zip package cannot read RAR on its own; its codec is the \
         separate p7zip-rar package.",
        crate::runtime::install_command("unar")
    )
}

fn extract_rar<F>(archive: &Path, destination: &Path, on_progress: &mut F) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    let tools = installed_rar_tools();
    if tools.is_empty() {
        return Err(CoreError::Other(rar_tool_hint()));
    }

    // Every tool is tried: a codec-less p7zip found first must not hide an installed unar.
    let has_unar = tools.iter().any(|(name, _)| *name == "unar");
    let mut first_failure: Option<String> = None;
    for (tool, flags) in tools {
        match run_rar_tool(tool, flags, archive, destination, on_progress) {
            Ok(written) => return Ok(written),
            Err(e) => {
                tracing::warn!(tool, error = %e, "RAR tool could not unpack the archive");
                first_failure.get_or_insert_with(|| e.to_string());
            }
        }
    }

    // "Unsupported Method" from a codec-less 7z reads as a broken archive on its own, so
    // what to install is spelled out alongside it.
    let reported = first_failure.unwrap_or_else(rar_tool_hint);
    Err(CoreError::Other(if has_unar {
        reported
    } else {
        format!("{reported}. {}", rar_tool_hint())
    }))
}

/// Unpack a RAR with one named tool. See [`extract_rar`], which tries these in turn.
fn run_rar_tool<F>(
    tool: &str,
    flags: &[&str],
    archive: &Path,
    destination: &Path,
    on_progress: &mut F,
) -> CoreResult<u64>
where
    F: FnMut(ExtractProgress),
{
    let program =
        crate::runtime::find_program(tool).ok_or_else(|| CoreError::Other(rar_tool_hint()))?;

    let mut command = std::process::Command::new(&program);
    // Piped so the child does not inherit our stdio, and so failures can be reported.
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        // Nobody reads a console whose output we are capturing; it only flashes up.
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::process::CREATE_NO_WINDOW);
    }
    for flag in flags {
        // The 7-Zip family joins the output directory to its flag with no separator.
        if *flag == "-o" {
            command.arg(format!("-o{}", destination.display()));
        } else {
            command.arg(flag);
        }
    }
    if tool == "unar" {
        command.arg(destination).arg(archive);
    } else if EXTRACTS_INTO_WORKING_DIR.contains(&tool) {
        command.arg(archive).current_dir(destination);
    } else {
        command.arg(archive);
    }

    tracing::info!(
        tool,
        ?archive,
        ?destination,
        "unpacking RAR with an external tool"
    );

    // Last in `RAR_TOOLS` because libarchive's RAR support is partial: it can unpack
    // "successfully" into files that are not what the archive held. Better said than inferred.
    if tool == "bsdtar" || tool == "tar" {
        tracing::warn!(
            "falling back to bsdtar for a RAR; install unar if this archive unpacks \
             incorrectly"
        );
    }

    // These tools print no usable progress, so the destination is sampled against the
    // compressed size as a rough total.
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

    // On its own thread so the pipes drain: a verbose extractor fills a pipe buffer at once,
    // and polling `try_wait` without reading would deadlock there.
    let child = command
        .spawn()
        .map_err(|e| CoreError::Other(format!("could not run {tool}: {e}")))?;
    let waiter = std::thread::spawn(move || child.wait_with_output());

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

/// Total size of everything under a directory. Links are counted, never followed: an
/// archive may contain one that points back at a parent, and following it would not return.
pub fn directory_size(path: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.path().symlink_metadata()?;
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
    fn a_torrent_file_is_recognised() {
        // What the server's torrent provider actually answers with.
        assert!(is_torrent_bytes(
            b"d8:announce30:http://tracker.example/announce4:infod4:name7:Celestee"
        ));
        // Key order is only conventional, so `info` alone has to be enough.
        assert!(is_torrent_bytes(
            b"d10:created by7:Gameyfin4:infod6:lengthi9ee"
        ));
    }

    #[test]
    fn an_archive_is_not_mistaken_for_a_torrent() {
        assert!(!is_torrent_bytes(b"PK\x03\x04rest of a zip"));
        assert!(!is_torrent_bytes(&[]));
        // Starts like bencode but carries neither key.
        assert!(!is_torrent_bytes(b"deadbeef"));
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
        // A game that arrives as a RAR with no file extension.
        assert_eq!(detect_bytes(b"Rar!\x1a\x07\x00"), ArchiveKind::Rar);
        // RAR 5 differs only after the shared prefix.
        assert_eq!(detect_bytes(b"Rar!\x1a\x07\x01\x00"), ArchiveKind::Rar);
    }

    #[test]
    fn a_relative_link_that_stays_inside_is_kept() {
        // `../lib/libfoo.so.1` is an ordinary shared-library link and must be allowed.
        let dest = Path::new("/games/celeste");
        assert_eq!(
            resolve_link(dest, &dest.join("bin"), Path::new("../lib/libfoo.so.1")),
            Some(PathBuf::from("/games/celeste/lib/libfoo.so.1"))
        );
        assert_eq!(
            resolve_link(dest, dest, Path::new("./libfoo.so.1.2.3")),
            Some(PathBuf::from("/games/celeste/libfoo.so.1.2.3"))
        );
    }

    #[test]
    fn a_link_out_of_the_destination_is_refused() {
        let dest = Path::new("/games/celeste");
        assert_eq!(
            resolve_link(dest, dest, Path::new("../../etc/passwd")),
            None
        );
        assert_eq!(resolve_link(dest, dest, Path::new("/etc/passwd")), None);
        // One level up from the root is already outside, even without naming a target.
        assert_eq!(resolve_link(dest, dest, Path::new("..")), None);
    }

    #[test]
    fn detects_the_tar_wrappers() {
        assert_eq!(
            detect_bytes(&[0x1F, 0x8B, 0x08, 0x00]),
            ArchiveKind::Tar(TarCompression::Gzip)
        );
        assert_eq!(
            detect_bytes(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]),
            ArchiveKind::Tar(TarCompression::Xz)
        );
        assert_eq!(
            detect_bytes(&[0x28, 0xB5, 0x2F, 0xFD, 0x00]),
            ArchiveKind::Tar(TarCompression::Zstd)
        );
        assert_eq!(
            detect_bytes(b"BZh9"),
            ArchiveKind::Tar(TarCompression::Bzip2)
        );
    }

    #[test]
    fn a_plain_tar_is_recognised_by_the_marker_inside_its_header() {
        // Nothing identifies a tar at offset zero, so this only works by seeking; a
        // detector that reads the first eight bytes alone reports "not an archive".
        let dir = scratch("tar-detect");
        let archive = dir.join("game.tar");
        {
            let mut builder = tar::Builder::new(File::create(&archive).unwrap());
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "readme.txt", &b"hello"[..])
                .unwrap();
            builder.finish().unwrap();
        }

        assert_eq!(
            detect(&archive).unwrap(),
            ArchiveKind::Tar(TarCompression::None)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracts_a_real_tar_gz_with_progress() {
        let dir = scratch("targz");
        let archive = dir.join("game.tar.gz");

        {
            let encoder = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::default(),
            );
            let mut builder = tar::Builder::new(encoder);
            for (name, body) in [
                ("readme.txt", &b"hello"[..]),
                ("bin/game.sh", &b"binary"[..]),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, name, body).unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
        }

        let dest = dir.join("out");
        let mut samples = Vec::new();
        let written = extract(&archive, &dest, |p| samples.push(p)).unwrap();

        assert_eq!(written, 11);
        assert_eq!(
            std::fs::read_to_string(dest.join("readme.txt")).unwrap(),
            "hello"
        );
        assert!(dest.join("bin/game.sh").exists());
        // Progress is counted against the compressed file, so it starts from a real
        // total and ends having consumed all of it.
        assert_eq!(
            samples.first().unwrap().bytes_total,
            std::fs::metadata(&archive).unwrap().len()
        );
        assert_eq!(samples.last().unwrap().percent(), 100.0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tar_entry_cannot_escape_the_destination() {
        let dir = scratch("tar-escape");
        let archive = dir.join("evil.tar");
        {
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o644);
            // Written into the header directly: the `tar` builder refuses to *create* a
            // traversing entry, which is exactly the entry a hostile archive contains.
            let name = b"../escaped.txt";
            header.as_mut_bytes()[..name.len()].copy_from_slice(name);
            header.set_cksum();

            let mut builder = tar::Builder::new(File::create(&archive).unwrap());
            builder.append(&header, &b"evil"[..]).unwrap();
            builder.finish().unwrap();
        }

        let dest = dir.join("out");
        extract(&archive, &dest, |_| {}).unwrap();
        assert!(!dir.join("escaped.txt").exists());

        std::fs::remove_dir_all(&dir).unwrap();
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
        // One huge entry, so entry counts would read 0% throughout.
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
    #[cfg(unix)]
    fn directory_size_does_not_follow_a_link_that_loops() {
        // An archive may hold a link pointing back at its own parent, which is inside the
        // destination and so allowed; following it would recurse forever.
        let dir = scratch("loop");
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::write(dir.join("game/one.bin"), vec![0u8; 10]).unwrap();
        std::os::unix::fs::symlink("..", dir.join("game/up")).unwrap();

        assert!(directory_size(&dir).unwrap() < 1024);
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
