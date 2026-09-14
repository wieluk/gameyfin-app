//! Unpacking a zip while it downloads, including the ones Gameyfin builds on the fly with a data
//! descriptor after every entry, which `zip`'s own stream reader cannot follow.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use flate2::bufread::DeflateDecoder;
use flate2::Crc;

use crate::download::Cancel;
use crate::error::{CoreError, CoreResult};
use crate::extract::safe_join;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL: u32 = 0x0605_4b50;
const ZIP64_END_OF_CENTRAL: u32 = 0x0606_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;

const FLAG_ENCRYPTED: u16 = 1;
const FLAG_DESCRIPTOR: u16 = 1 << 3;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
/// A 32-bit size holding this has its real value in the zip64 extra field.
const ZIP64_MARKER: u64 = u32::MAX as u64;
const ZIP64_EXTRA: u16 = 0x0001;

const BUFFER: usize = 256 * 1024;

/// Whether a zip's first entry reads front to back: deflated, or stored with its size. An
/// encrypted one needs the password, which only the file extractor takes.
pub fn is_streamable(head: &[u8]) -> bool {
    if head.len() < 10 || head[..4] != LOCAL_HEADER.to_le_bytes() {
        return false;
    }
    let (flags, method) = (u16_at(head, 6), u16_at(head, 8));
    flags & FLAG_ENCRYPTED == 0
        && (method == DEFLATED || (method == STORED && flags & FLAG_DESCRIPTOR == 0))
}

/// Unpacks a zip read front to back and returns the bytes written. Ending before the central
/// directory is an error, never a partial game.
pub fn extract_stream<R: Read>(reader: R, destination: &Path, cancel: &Cancel) -> CoreResult<u64> {
    std::fs::create_dir_all(destination)?;
    let mut input = BufReader::with_capacity(BUFFER, reader);
    let mut buffer = vec![0u8; BUFFER];
    let mut total = 0u64;
    // Where each entry went, for the Unix modes only the central directory records.
    let mut placed = HashMap::new();

    loop {
        stop_if_cancelled(cancel)?;
        match read_u32(&mut input).map_err(truncated)? {
            LOCAL_HEADER => {}
            CENTRAL_HEADER => {
                finish_central_directory(&mut input, &placed)?;
                return Ok(total);
            }
            END_OF_CENTRAL => return Ok(total),
            _ => return Err(cannot_stream("the download is not a zip")),
        }

        let header = read_local_header(&mut input).map_err(truncated)?;
        if header.flags & FLAG_ENCRYPTED != 0 {
            return Err(cannot_stream(format!("{} is encrypted", header.name)));
        }
        let descriptor = header.flags & FLAG_DESCRIPTOR != 0;
        let is_dir = header.name.ends_with('/') || header.name.ends_with('\\');
        let target = safe_join(destination, &header.name);

        let mut out: Box<dyn Write> = match &target {
            None => {
                tracing::warn!("skipping unsafe archive entry: {}", header.name);
                Box::new(std::io::sink())
            }
            Some(path) if is_dir => {
                std::fs::create_dir_all(path)?;
                Box::new(std::io::sink())
            }
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Box::new(BufWriter::with_capacity(BUFFER, File::create(path)?))
            }
        };

        let body = match (header.method, descriptor) {
            (DEFLATED, _) => inflate(&mut input, &mut *out, &mut buffer, cancel)?,
            (STORED, false) => copy_stored(
                &mut input,
                header.compressed,
                &mut *out,
                &mut buffer,
                cancel,
            )?,
            (method, _) => {
                return Err(cannot_stream(format!(
                    "{} uses compression method {method}",
                    header.name
                )))
            }
        };
        out.flush()?;
        drop(out);

        let expected = if descriptor {
            read_descriptor(&mut input, &body, header.zip64).map_err(truncated)?
        } else {
            Sizes {
                crc: header.crc,
                compressed: header.compressed,
                uncompressed: header.uncompressed,
            }
        };
        if expected != body {
            return Err(CoreError::Other(format!(
                "{} arrived damaged: its checksum or size does not match.",
                header.name
            )));
        }

        if let Some(path) = target.filter(|_| !is_dir) {
            placed.insert(header.name, path);
        }
        total += body.uncompressed;
    }
}

struct LocalHeader {
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u64,
    uncompressed: u64,
    zip64: bool,
    name: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Sizes {
    crc: u32,
    compressed: u64,
    uncompressed: u64,
}

fn read_local_header<R: Read>(input: &mut R) -> std::io::Result<LocalHeader> {
    let mut fixed = [0u8; 26];
    input.read_exact(&mut fixed)?;
    let mut compressed = u64::from(u32_at(&fixed, 14));
    let mut uncompressed = u64::from(u32_at(&fixed, 18));
    let mut name = vec![0u8; usize::from(u16_at(&fixed, 22))];
    input.read_exact(&mut name)?;
    let mut extra = vec![0u8; usize::from(u16_at(&fixed, 24))];
    input.read_exact(&mut extra)?;
    let zip64 = apply_zip64(&extra, &mut uncompressed, &mut compressed);

    Ok(LocalHeader {
        flags: u16_at(&fixed, 2),
        method: u16_at(&fixed, 4),
        crc: u32_at(&fixed, 10),
        compressed,
        uncompressed,
        zip64,
        name: String::from_utf8_lossy(&name).into_owned(),
    })
}

/// Swaps each size set to the marker for its zip64 value, and says whether that field was there.
fn apply_zip64(extra: &[u8], uncompressed: &mut u64, compressed: &mut u64) -> bool {
    let mut rest = extra;
    while rest.len() >= 4 {
        let id = u16_at(rest, 0);
        let len = usize::from(u16_at(rest, 2));
        let Some(data) = rest.get(4..4 + len) else {
            break;
        };
        if id == ZIP64_EXTRA {
            let mut values = data
                .chunks_exact(8)
                .map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]));
            for size in [uncompressed, compressed] {
                if *size == ZIP64_MARKER {
                    if let Some(value) = values.next() {
                        *size = value;
                    }
                }
            }
            return true;
        }
        rest = &rest[4 + len..];
    }
    false
}

fn inflate<B: BufRead>(
    input: B,
    out: &mut dyn Write,
    buffer: &mut [u8],
    cancel: &Cancel,
) -> CoreResult<Sizes> {
    // The bufread decoder consumes exactly the deflate data, leaving the descriptor unread.
    let mut decoder = DeflateDecoder::new(input);
    let mut crc = Crc::new();
    loop {
        stop_if_cancelled(cancel)?;
        let read = decoder.read(buffer).map_err(damaged)?;
        if read == 0 {
            break;
        }
        crc.update(&buffer[..read]);
        out.write_all(&buffer[..read])?;
    }
    Ok(Sizes {
        crc: crc.sum(),
        compressed: decoder.total_in(),
        uncompressed: decoder.total_out(),
    })
}

fn copy_stored<R: Read>(
    input: &mut R,
    size: u64,
    out: &mut dyn Write,
    buffer: &mut [u8],
    cancel: &Cancel,
) -> CoreResult<Sizes> {
    let mut crc = Crc::new();
    let mut remaining = size;
    while remaining > 0 {
        stop_if_cancelled(cancel)?;
        let want = buffer
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let read = input.read(&mut buffer[..want])?;
        if read == 0 {
            return Err(truncated(std::io::ErrorKind::UnexpectedEof.into()));
        }
        crc.update(&buffer[..read]);
        out.write_all(&buffer[..read])?;
        remaining -= read as u64;
    }
    Ok(Sizes {
        crc: crc.sum(),
        compressed: size,
        uncompressed: size,
    })
}

fn read_descriptor<R: Read>(input: &mut R, body: &Sizes, zip64: bool) -> std::io::Result<Sizes> {
    let first = read_u32(input)?;
    // The signature is optional, so without it the first four bytes are the checksum.
    let crc = if first == DESCRIPTOR {
        read_u32(input)?
    } else {
        first
    };
    let (compressed, uncompressed) =
        if descriptor_is_wide(body.compressed, body.uncompressed, zip64) {
            (read_u64(input)?, read_u64(input)?)
        } else {
            (u64::from(read_u32(input)?), u64::from(read_u32(input)?))
        };
    Ok(Sizes {
        crc,
        compressed,
        uncompressed,
    })
}

/// Java widens a descriptor's sizes once one no longer fits 32 bits; zip64 headers always do.
fn descriptor_is_wide(compressed: u64, uncompressed: u64, zip64: bool) -> bool {
    zip64 || compressed >= ZIP64_MARKER || uncompressed >= ZIP64_MARKER
}

/// Reads the closing directory for its Unix modes, and so a body cut off inside it fails.
fn finish_central_directory<R: Read>(
    input: &mut R,
    placed: &HashMap<String, PathBuf>,
) -> CoreResult<()> {
    loop {
        let mut fixed = [0u8; 42];
        input.read_exact(&mut fixed).map_err(truncated)?;
        let mut name = vec![0u8; usize::from(u16_at(&fixed, 24))];
        input.read_exact(&mut name).map_err(truncated)?;
        let skip = u64::from(u16_at(&fixed, 26)) + u64::from(u16_at(&fixed, 28));
        std::io::copy(&mut input.by_ref().take(skip), &mut std::io::sink())?;
        // The high byte of "version made by" names the system the attributes belong to.
        restore_mode(
            placed,
            &String::from_utf8_lossy(&name),
            fixed[1],
            u32_at(&fixed, 34),
        );

        match read_u32(input).map_err(truncated)? {
            CENTRAL_HEADER => {}
            END_OF_CENTRAL | ZIP64_END_OF_CENTRAL => return Ok(()),
            _ => {
                return Err(CoreError::Other(
                    "The download ends in a damaged zip directory.".into(),
                ))
            }
        }
    }
}

#[cfg(unix)]
fn restore_mode(placed: &HashMap<String, PathBuf>, name: &str, host: u8, external: u32) {
    const UNIX: u8 = 3;
    let mode = external >> 16;
    if host == UNIX && mode != 0 {
        if let Some(path) = placed.get(name) {
            crate::extract::set_mode(path, mode);
        }
    }
}

#[cfg(not(unix))]
fn restore_mode(_placed: &HashMap<String, PathBuf>, _name: &str, _host: u8, _external: u32) {}

fn stop_if_cancelled(cancel: &Cancel) -> CoreResult<()> {
    if cancel.is_cancelled() {
        Err(CoreError::Cancelled)
    } else {
        Ok(())
    }
}

fn cannot_stream(reason: impl Into<String>) -> CoreError {
    CoreError::CannotStream(reason.into())
}

fn truncated(e: std::io::Error) -> CoreError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        CoreError::Other("The download ended before the whole game arrived.".into())
    } else {
        e.into()
    }
}

fn damaged(e: std::io::Error) -> CoreError {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof => truncated(e),
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
            CoreError::Other(format!("The download arrived damaged: {e}"))
        }
        _ => e.into(),
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u32<R: Read + ?Sized>(input: &mut R) -> std::io::Result<u32> {
    let mut bytes = [0u8; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64<R: Read + ?Sized>(input: &mut R) -> std::io::Result<u64> {
    let mut bytes = [0u8; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-zipstream-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[derive(Clone, Copy)]
    struct Style {
        deflate: bool,
        descriptor: bool,
        signature: bool,
        bad_crc: bool,
    }

    /// What Java's `ZipOutputStream` writes, which is how Gameyfin serves a folder.
    const ON_THE_FLY: Style = Style {
        deflate: true,
        descriptor: true,
        signature: true,
        bad_crc: false,
    };

    fn push16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn zip(entries: &[(&str, &[u8])], style: Style) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let payload = if style.deflate {
                let mut encoder =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(data).unwrap();
                encoder.finish().unwrap()
            } else {
                data.to_vec()
            };
            let mut crc = Crc::new();
            crc.update(data);
            let crc = if style.bad_crc {
                crc.sum() ^ 1
            } else {
                crc.sum()
            };
            let flags = if style.descriptor { FLAG_DESCRIPTOR } else { 0 };
            let method = if style.deflate { DEFLATED } else { STORED };
            let sizes = [crc, payload.len() as u32, data.len() as u32];
            let in_header = if style.descriptor { [0; 3] } else { sizes };

            push32(&mut out, LOCAL_HEADER);
            push16(&mut out, 20);
            push16(&mut out, flags);
            push16(&mut out, method);
            push32(&mut out, 0);
            for value in in_header {
                push32(&mut out, value);
            }
            push16(&mut out, name.len() as u16);
            push16(&mut out, 0);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&payload);
            if style.descriptor {
                if style.signature {
                    push32(&mut out, DESCRIPTOR);
                }
                for value in sizes {
                    push32(&mut out, value);
                }
            }

            push32(&mut central, CENTRAL_HEADER);
            // Made on Unix, so the external attributes carry a mode.
            push16(&mut central, 0x031e);
            push16(&mut central, 20);
            push16(&mut central, flags);
            push16(&mut central, method);
            push32(&mut central, 0);
            for value in sizes {
                push32(&mut central, value);
            }
            push16(&mut central, name.len() as u16);
            for _ in 0..4 {
                push16(&mut central, 0);
            }
            push32(&mut central, 0o100_755 << 16);
            push32(&mut central, offset);
            central.extend_from_slice(name.as_bytes());
        }
        let (at, size, count) = (out.len() as u32, central.len() as u32, entries.len() as u16);
        out.extend_from_slice(&central);
        push32(&mut out, END_OF_CENTRAL);
        push16(&mut out, 0);
        push16(&mut out, 0);
        push16(&mut out, count);
        push16(&mut out, count);
        push32(&mut out, size);
        push32(&mut out, at);
        push16(&mut out, 0);
        out
    }

    /// Hands over a few bytes per read, as a slow connection does.
    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.0.len()).min(7);
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    fn unpack(bytes: &[u8], dir: &Path) -> CoreResult<u64> {
        extract_stream(Trickle(bytes), dir, &Cancel::new())
    }

    #[test]
    fn a_zip_written_on_the_fly_unpacks_into_its_folders() {
        let game: Vec<u8> = (0..300_000u32).map(|i| (i % 97) as u8).collect();
        let bytes = zip(
            &[
                ("Game/game.exe", &game),
                ("Game/data/level.dat", b"level one"),
            ],
            ON_THE_FLY,
        );
        let dir = scratch("on-the-fly");

        assert_eq!(unpack(&bytes, &dir).unwrap(), game.len() as u64 + 9);
        assert_eq!(std::fs::read(dir.join("Game/game.exe")).unwrap(), game);
        assert_eq!(
            std::fs::read(dir.join("Game/data/level.dat")).unwrap(),
            b"level one"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("Game/game.exe"))
                .unwrap()
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "the central directory's mode is applied");
        }
    }

    #[test]
    fn a_descriptor_without_its_signature_is_read() {
        let style = Style {
            signature: false,
            ..ON_THE_FLY
        };
        let bytes = zip(&[("a.txt", b"hello"), ("b.txt", b"world")], style);
        let dir = scratch("no-signature");
        assert_eq!(unpack(&bytes, &dir).unwrap(), 10);
        assert_eq!(std::fs::read(dir.join("b.txt")).unwrap(), b"world");
    }

    #[test]
    fn entries_with_their_sizes_in_the_header_unpack_too() {
        for deflate in [true, false] {
            let style = Style {
                deflate,
                descriptor: false,
                ..ON_THE_FLY
            };
            let bytes = zip(&[("a.txt", b"hello")], style);
            let dir = scratch(&format!("sized-{deflate}"));
            assert_eq!(unpack(&bytes, &dir).unwrap(), 5, "deflate: {deflate}");
            assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"hello");
        }
    }

    #[test]
    fn a_checksum_mismatch_fails() {
        let style = Style {
            bad_crc: true,
            ..ON_THE_FLY
        };
        let bytes = zip(&[("a.txt", b"hello")], style);
        let error = unpack(&bytes, &scratch("bad-crc")).unwrap_err();
        assert!(error.to_string().contains("damaged"), "{error}");
    }

    #[test]
    fn a_body_cut_off_before_the_directory_ends_is_incomplete() {
        let bytes = zip(&[("a.txt", b"hello"), ("b.txt", b"world")], ON_THE_FLY);
        for cut in [20, 38, 60, bytes.len() - 30, bytes.len() - 22] {
            let result = unpack(&bytes[..cut], &scratch(&format!("cut-{cut}")));
            assert!(
                result.is_err(),
                "a body cut at {cut} of {} passed",
                bytes.len()
            );
        }
    }

    #[test]
    fn an_entry_escaping_the_folder_is_skipped_and_the_rest_unpacks() {
        let bytes = zip(&[("../evil.txt", b"nope"), ("ok.txt", b"fine")], ON_THE_FLY);
        let dir = scratch("traversal");
        unpack(&bytes, &dir).unwrap();
        assert!(!dir.with_file_name("evil.txt").exists());
        assert_eq!(std::fs::read(dir.join("ok.txt")).unwrap(), b"fine");
    }

    #[test]
    fn a_stored_entry_without_its_size_cannot_stream() {
        let style = Style {
            deflate: false,
            ..ON_THE_FLY
        };
        let bytes = zip(&[("a.txt", b"hello")], style);
        let result = unpack(&bytes, &scratch("stored-descriptor"));
        assert!(
            matches!(result, Err(CoreError::CannotStream(_))),
            "{result:?}"
        );
    }

    #[test]
    fn a_cancel_stops_the_unpack() {
        let bytes = zip(&[("a.txt", b"hello")], ON_THE_FLY);
        let cancel = Cancel::new();
        cancel.cancel();
        let result = extract_stream(&bytes[..], &scratch("cancel"), &cancel);
        assert!(matches!(result, Err(CoreError::Cancelled)));
    }

    #[test]
    fn a_zip_streams_unless_encrypted_or_stored_without_its_size() {
        let entries: &[(&str, &[u8])] = &[("a.txt", b"hello")];
        for (deflate, descriptor, expected) in [
            (true, true, true),
            (true, false, true),
            (false, false, true),
            (false, true, false),
        ] {
            let style = Style {
                deflate,
                descriptor,
                ..ON_THE_FLY
            };
            assert_eq!(
                is_streamable(&zip(entries, style)),
                expected,
                "deflate {deflate}, descriptor {descriptor}"
            );
        }
        let mut encrypted = zip(entries, ON_THE_FLY);
        encrypted[6] |= FLAG_ENCRYPTED as u8;
        assert!(!is_streamable(&encrypted));
        assert!(!is_streamable(b"PK\x03\x04"));
    }

    #[test]
    fn only_zip_and_tar_are_unpacked_while_downloading() {
        use crate::extract::{streamable_kind, ArchiveKind, TarCompression};

        let zipped = zip(&[("a.txt", b"hello")], ON_THE_FLY);
        assert_eq!(streamable_kind(&zipped), Some(ArchiveKind::Zip));
        assert_eq!(
            streamable_kind(&[0x1F, 0x8B, 8, 0]),
            Some(ArchiveKind::Tar(TarCompression::Gzip))
        );
        let mut plain_tar = vec![0u8; 512];
        plain_tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(
            streamable_kind(&plain_tar),
            Some(ArchiveKind::Tar(TarCompression::None))
        );
        assert_eq!(
            streamable_kind(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0, 4]),
            None
        );
        assert_eq!(streamable_kind(b"Rar!\x1a\x07\x01\x00"), None);
        assert_eq!(streamable_kind(b"MZ\x90\x00"), None);
    }

    #[test]
    fn descriptor_sizes_widen_only_when_needed() {
        assert!(!descriptor_is_wide(10, 20, false));
        assert!(descriptor_is_wide(10, ZIP64_MARKER, false));
        assert!(descriptor_is_wide(ZIP64_MARKER + 5, 20, false));
        assert!(descriptor_is_wide(10, 20, true));
    }

    #[test]
    fn zip64_sizes_replace_the_marker() {
        let mut extra = Vec::new();
        push16(&mut extra, ZIP64_EXTRA);
        push16(&mut extra, 16);
        extra.extend_from_slice(&(5u64 << 30).to_le_bytes());
        extra.extend_from_slice(&(4u64 << 30).to_le_bytes());
        let (mut uncompressed, mut compressed) = (ZIP64_MARKER, ZIP64_MARKER);
        assert!(apply_zip64(&extra, &mut uncompressed, &mut compressed));
        assert_eq!((uncompressed, compressed), (5 << 30, 4 << 30));

        let (mut uncompressed, mut compressed) = (7, 3);
        assert!(!apply_zip64(&[], &mut uncompressed, &mut compressed));
        assert_eq!((uncompressed, compressed), (7, 3));
    }
}
