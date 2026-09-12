//! Icons for game shortcuts. A Windows executable carries the game's own icon, the one
//! Explorer shows; without a usable one the cover art stands in.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};

/// The largest edge an icon is kept at: all an `.ico` entry can hold, and as large as a
/// desktop draws one.
pub const SIZE: u32 = 256;

/// Below this an executable's icon blurs when a menu enlarges it, and the cover reads better.
const SMALLEST_USEFUL: u32 = 48;

/// Resource type ids, from the PE format.
const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;

/// A resource section larger than this is not read: icons are small, and some games embed
/// their data as resources.
const MAX_RESOURCES: u32 = 64 * 1024 * 1024;

/// The first icon in a Windows executable, as Explorer shows it, at its largest size.
/// `None` for anything else, or when every icon is too small to enlarge.
pub fn from_executable(path: &Path) -> Option<RgbaImage> {
    let ico = icon_file(path)?;
    let image = image::load_from_memory_with_format(&ico, ImageFormat::Ico).ok()?;
    if image.width().max(image.height()) < SMALLEST_USEFUL {
        return None;
    }
    Some(square(image))
}

/// Cover art, whole, centred on a transparent square. Cropping a portrait cover to a
/// square would cut off its title.
pub fn from_artwork(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes).ok().map(square)
}

/// Where a game's shortcut icon is kept, in the format this platform's shortcuts take.
pub fn path_for(dir: &Path, game_id: i64) -> PathBuf {
    dir.join(format!(
        "{game_id}.{}",
        if cfg!(windows) { "ico" } else { "png" }
    ))
}

pub fn write(image: &RgbaImage, path: &Path) -> std::io::Result<()> {
    let format = if cfg!(windows) {
        ImageFormat::Ico
    } else {
        ImageFormat::Png
    };
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, format)
        .map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes.into_inner())
}

/// Scaled down to fit [`SIZE`] when larger, then centred on a transparent square.
fn square(image: DynamicImage) -> RgbaImage {
    let image = if image.width().max(image.height()) > SIZE {
        image.resize(SIZE, SIZE, FilterType::Lanczos3)
    } else {
        image
    };
    let edge = image.width().max(image.height());
    let mut canvas = RgbaImage::new(edge, edge);
    image::imageops::overlay(
        &mut canvas,
        &image.to_rgba8(),
        i64::from((edge - image.width()) / 2),
        i64::from((edge - image.height()) / 2),
    );
    canvas
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The resource section of a PE file, with the address it is loaded at.
struct Resources {
    bytes: Vec<u8>,
    /// Where the section starts in memory, which data entries are addressed against.
    section_rva: u32,
    /// Where the resource directory starts within `bytes`.
    root: usize,
}

impl Resources {
    /// Reads only the headers and the resource section, never the whole file: game
    /// executables run to hundreds of megabytes.
    fn read(path: &Path) -> Option<Self> {
        let mut file = std::fs::File::open(path).ok()?;
        let mut headers = Vec::new();
        (&mut file).take(64 * 1024).read_to_end(&mut headers).ok()?;
        if headers.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(&headers, 0x3c)? as usize;
        if headers.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let coff = pe + 4;
        let sections = u16_at(&headers, coff + 2)? as usize;
        let optional = coff + 20;
        let optional_size = u16_at(&headers, coff + 16)? as usize;
        let directories = match u16_at(&headers, optional)? {
            0x10b => optional + 96,
            0x20b => optional + 112,
            _ => return None,
        };
        if u32_at(&headers, directories - 4)? <= 2 {
            return None;
        }
        let rva = u32_at(&headers, directories + 16)?;
        if rva == 0 {
            return None;
        }

        (0..sections).find_map(|index| {
            let header = optional + optional_size + index * 40;
            let virtual_size = u32_at(&headers, header + 8)?;
            let section_rva = u32_at(&headers, header + 12)?;
            let raw_size = u32_at(&headers, header + 16)?;
            let raw_offset = u32_at(&headers, header + 20)?;
            let end = section_rva.checked_add(virtual_size.max(raw_size))?;
            if !(section_rva..end).contains(&rva) || raw_size > MAX_RESOURCES {
                return None;
            }
            let mut bytes = Vec::new();
            file.seek(SeekFrom::Start(u64::from(raw_offset))).ok()?;
            (&mut file)
                .take(u64::from(raw_size))
                .read_to_end(&mut bytes)
                .ok()?;
            Some(Self {
                bytes,
                section_rva,
                root: (rva - section_rva) as usize,
            })
        })
    }

    /// A directory's entries as (name or id, target). Named entries have the high bit set
    /// in the first field, subdirectories in the second; offsets are from the root.
    fn entries(&self, directory: usize) -> Option<Vec<(u32, u32)>> {
        let at = self.root.checked_add(directory)?;
        let count =
            usize::from(u16_at(&self.bytes, at + 12)?) + usize::from(u16_at(&self.bytes, at + 14)?);
        (0..count)
            .map(|i| {
                let entry = at + 16 + i * 8;
                Some((u32_at(&self.bytes, entry)?, u32_at(&self.bytes, entry + 4)?))
            })
            .collect()
    }

    fn find(&self, directory: usize, id: u32) -> Option<usize> {
        let (_, target) = self
            .entries(directory)?
            .into_iter()
            .find(|&(name, _)| name == id)?;
        subdirectory(target)
    }

    /// Follows the first entry down to the data, which skips the choice of language.
    fn first_data(&self, mut directory: usize) -> Option<&[u8]> {
        // The tree is three levels deep; the bound stops a looping one.
        for _ in 0..3 {
            let (_, target) = *self.entries(directory)?.first()?;
            match subdirectory(target) {
                Some(next) => directory = next,
                None => return self.data(target as usize),
            }
        }
        None
    }

    fn data(&self, entry: usize) -> Option<&[u8]> {
        let at = self.root.checked_add(entry)?;
        let rva = u32_at(&self.bytes, at)?;
        let size = u32_at(&self.bytes, at + 4)? as usize;
        let start = rva.checked_sub(self.section_rva)? as usize;
        self.bytes.get(start..start.checked_add(size)?)
    }
}

fn subdirectory(target: u32) -> Option<usize> {
    (target & 0x8000_0000 != 0).then_some((target & 0x7fff_ffff) as usize)
}

/// The executable's first icon group, reassembled as an `.ico` holding only its largest
/// image. Choosing here rather than in the decoder, which prefers colour depth over size.
fn icon_file(path: &Path) -> Option<Vec<u8>> {
    let resources = Resources::read(path)?;
    let groups = resources.find(0, RT_GROUP_ICON)?;
    let icons = resources.find(0, RT_ICON)?;

    // Explorer shows the first group, whatever its id.
    let group = resources.first_data(groups)?;
    let count = usize::from(u16_at(group, 4)?);
    let (entry, image) = (0..count)
        .filter_map(|i| {
            let entry = group.get(6 + i * 14..6 + (i + 1) * 14)?;
            let id = u32::from(u16_at(entry, 12)?);
            let image = resources.first_data(resources.find(icons, id)?)?;
            Some((entry, image))
        })
        // A width or height of 0 means 256.
        .max_by_key(|(entry, _)| {
            let edge = |b: u8| if b == 0 { 256 } else { u32::from(b) };
            (
                edge(entry[0]) * edge(entry[1]),
                u16_at(entry, 6).unwrap_or(0),
            )
        })?;

    let mut ico = Vec::with_capacity(22 + image.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    ico.extend_from_slice(&entry[..8]);
    ico.extend_from_slice(&u32::try_from(image.len()).ok()?.to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(image);
    Some(ico)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gameyfin-icon-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(edge: u32, colour: Rgba<u8>) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        RgbaImage::from_pixel(edge, edge, colour)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    /// A PE32 whose only section is resources: one icon group holding a PNG per size, the
    /// layout a resource compiler produces.
    fn exe_with_icons(sizes: &[u32]) -> Vec<u8> {
        const SECTION_RVA: u32 = 0x1000;
        let images: Vec<Vec<u8>> = sizes
            .iter()
            .enumerate()
            .map(|(i, &edge)| png(edge, Rgba([i as u8 * 60, 0, 0, 255])))
            .collect();
        let n = images.len() as u32;

        let icon_dir = 32;
        let languages = icon_dir + 16 + 8 * n;
        let group_dir = languages + 24 * n;
        let group_language = group_dir + 24;
        let data_entries = group_language + 24;
        let blobs = data_entries + 16 * (n + 1);

        let mut group = vec![0, 0, 1, 0, n as u8, 0];
        for (i, &edge) in sizes.iter().enumerate() {
            let b = if edge >= 256 { 0 } else { edge as u8 };
            group.extend_from_slice(&[b, b, 0, 0, 1, 0, 32, 0]);
            group.extend_from_slice(&(images[i].len() as u32).to_le_bytes());
            group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
        }

        let directory = |out: &mut Vec<u8>, entries: &[(u32, u32)]| {
            out.extend_from_slice(&[0; 14]);
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for (name, target) in entries {
                out.extend_from_slice(&name.to_le_bytes());
                out.extend_from_slice(&target.to_le_bytes());
            }
        };
        const DIR: u32 = 0x8000_0000;

        let mut rsrc = Vec::new();
        directory(
            &mut rsrc,
            &[(RT_ICON, DIR | icon_dir), (RT_GROUP_ICON, DIR | group_dir)],
        );
        let ids: Vec<(u32, u32)> = (0..n)
            .map(|i| (i + 1, DIR | (languages + 24 * i)))
            .collect();
        directory(&mut rsrc, &ids);
        for i in 0..n {
            directory(&mut rsrc, &[(0x409, data_entries + 16 * i)]);
        }
        directory(&mut rsrc, &[(1, DIR | group_language)]);
        directory(&mut rsrc, &[(0x409, data_entries + 16 * n)]);

        let mut offset = blobs;
        for blob in images.iter().chain([&group]) {
            rsrc.extend_from_slice(&(SECTION_RVA + offset).to_le_bytes());
            rsrc.extend_from_slice(&(blob.len() as u32).to_le_bytes());
            rsrc.extend_from_slice(&[0; 8]);
            offset += blob.len() as u32;
        }
        assert_eq!(rsrc.len() as u32, blobs);
        for blob in images.iter().chain([&group]) {
            rsrc.extend_from_slice(blob);
        }

        let mut exe = vec![0u8; 0x200];
        exe[..2].copy_from_slice(b"MZ");
        exe[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        exe[0x40..0x44].copy_from_slice(b"PE\0\0");
        exe[0x44..0x46].copy_from_slice(&0x14cu16.to_le_bytes());
        exe[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
        exe[0x54..0x56].copy_from_slice(&224u16.to_le_bytes());
        exe[0x58..0x5a].copy_from_slice(&0x10bu16.to_le_bytes());
        exe[0xb4..0xb8].copy_from_slice(&16u32.to_le_bytes());
        exe[0xc8..0xcc].copy_from_slice(&SECTION_RVA.to_le_bytes());
        exe[0xcc..0xd0].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        let section = 0x58 + 224;
        exe[section..section + 5].copy_from_slice(b".rsrc");
        exe[section + 8..section + 12].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        exe[section + 12..section + 16].copy_from_slice(&SECTION_RVA.to_le_bytes());
        exe[section + 16..section + 20].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        exe[section + 20..section + 24].copy_from_slice(&0x200u32.to_le_bytes());
        exe.extend_from_slice(&rsrc);
        exe
    }

    #[test]
    fn the_largest_image_of_the_executables_icon_is_used() {
        let dir = scratch("largest");
        let exe = dir.join("game.exe");
        std::fs::write(&exe, exe_with_icons(&[16, 256, 48])).unwrap();

        let icon = from_executable(&exe).expect("an icon");
        assert_eq!(icon.dimensions(), (256, 256));
        // The second image, the 256 pixel one, is the one painted with red 60.
        assert_eq!(icon.get_pixel(128, 128), &Rgba([60, 0, 0, 255]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_executable_with_only_tiny_icons_gives_way_to_the_cover() {
        let dir = scratch("tiny");
        let exe = dir.join("old.exe");
        std::fs::write(&exe, exe_with_icons(&[16, 32])).unwrap();
        assert!(from_executable(&exe).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn anything_but_a_windows_program_has_no_icon() {
        let dir = scratch("not-pe");
        for (name, bytes) in [
            ("game.x86_64", b"\x7fELF\x02\x01\x01\0".as_slice()),
            ("start.sh", b"#!/bin/sh\n".as_slice()),
            ("stub.exe", b"MZ".as_slice()),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            assert!(from_executable(&path).is_none(), "{name}");
        }
        assert!(from_executable(&dir.join("missing.exe")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_truncated_executable_is_rejected_rather_than_read_past() {
        let dir = scratch("truncated");
        let whole = exe_with_icons(&[64]);
        for cut in [0x100, 0x230, whole.len() - 10] {
            let path = dir.join(format!("cut-{cut}.exe"));
            std::fs::write(&path, &whole[..cut]).unwrap();
            assert!(from_executable(&path).is_none(), "cut at {cut:#x}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_portrait_cover_is_letterboxed_whole_onto_a_square() {
        let mut bytes = Cursor::new(Vec::new());
        RgbaImage::from_pixel(600, 800, Rgba([0, 0, 255, 255]))
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();

        let icon = from_artwork(bytes.get_ref()).expect("an icon");
        assert_eq!(icon.dimensions(), (SIZE, SIZE));
        assert_eq!(
            icon.get_pixel(128, 128)[3],
            255,
            "the cover fills the middle"
        );
        assert_eq!(icon.get_pixel(128, 0)[3], 255, "and the full height");
        assert_eq!(icon.get_pixel(2, 128)[3], 0, "the sides are transparent");
        assert!(from_artwork(b"not an image").is_none());
    }

    #[test]
    fn a_written_icon_reads_back() {
        let dir = scratch("write");
        let path = path_for(&dir.join("nested"), 12);
        let image = RgbaImage::from_pixel(64, 64, Rgba([1, 2, 3, 255]));
        write(&image, &path).unwrap();
        let back = image::open(&path).unwrap().to_rgba8();
        assert_eq!(back.get_pixel(10, 10), &Rgba([1, 2, 3, 255]));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
