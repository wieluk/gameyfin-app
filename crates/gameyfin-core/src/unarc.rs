//! Repairs FreeArc's `unarc.dll` as a repack installer unpacks it. Its search for the largest
//! free memory block overflows once a 32-bit process can reserve 2 GB, then loops forever.

use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", test))]
use std::{
    collections::{HashMap, HashSet},
    time::SystemTime,
};

/// `mov ebp, 0; mov edi, 0xFFFFFFFF`: the search starting between 0 and `UINT_MAX`.
const BOUNDS: [u8; 10] = [0xBD, 0, 0, 0, 0, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF];
/// `lea eax, [ebp + edi]`: the `a + b` that wraps, a few instructions after the bounds.
const MIDPOINT: [u8; 4] = [0x8D, 0x44, 0x3D, 0x00];
/// How far past the bounds the midpoint may sit; the call to free the last block lies between.
const MIDPOINT_WITHIN: usize = 24;
/// The top byte of `UINT_MAX` in `mov edi`, relative to the start of [`BOUNDS`].
const UPPER_BOUND_TOP_BYTE: usize = 9;
/// Lowers the upper bound to `0x7FFFFFFF`, where `a + b` can no longer wrap.
const PATCHED_TOP_BYTE: u8 = 0x7F;
/// No real `unarc.dll` comes near this, and it keeps a stray large file from being read.
const LARGEST_DLL: u64 = 16 * 1024 * 1024;

/// Offsets of each upper-bound byte in `image` that still needs lowering.
pub fn overflowing_bounds(image: &[u8]) -> Vec<usize> {
    image
        .windows(BOUNDS.len())
        .enumerate()
        .filter(|(_, window)| *window == BOUNDS)
        .map(|(at, _)| at)
        .filter(|&at| {
            let start = at + BOUNDS.len();
            let end = (start + MIDPOINT_WITHIN).min(image.len());
            image[start..end]
                .windows(MIDPOINT.len())
                .any(|window| window == MIDPOINT)
        })
        .map(|at| at + UPPER_BOUND_TOP_BYTE)
        .collect()
}

/// Lower the search's upper bound in `path`, returning how many copies were patched.
pub fn patch_file(path: &Path) -> std::io::Result<usize> {
    use std::io::{Seek, SeekFrom, Write};

    if std::fs::metadata(path)?.len() > LARGEST_DLL {
        return Ok(0);
    }
    let offsets = overflowing_bounds(&std::fs::read(path)?);
    if offsets.is_empty() {
        return Ok(0);
    }
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    for &offset in &offsets {
        file.seek(SeekFrom::Start(offset as u64))?;
        file.write_all(&[PATCHED_TOP_BYTE])?;
    }
    file.flush()?;
    Ok(offsets.len())
}

/// Where Windows programs in a prefix keep temporary files, which is where Inno Setup unpacks
/// the DLLs its script calls.
#[cfg(any(target_os = "linux", test))]
fn temp_roots(prefix: &Path) -> Vec<PathBuf> {
    let drive_c = crate::prefix::wine_root(prefix).join("drive_c");
    let mut roots = vec![drive_c.join("windows").join("temp")];
    if let Ok(users) = std::fs::read_dir(drive_c.join("users")) {
        for user in users.flatten() {
            roots.push(user.path().join("AppData").join("Local").join("Temp"));
            roots.push(user.path().join("Temp"));
        }
    }
    // Older prefixes link `Temp` to `AppData\Local\Temp`, so one folder can appear twice.
    let mut unique: Vec<PathBuf> = roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .filter(|root| root.is_dir())
        .collect();
    unique.sort();
    unique.dedup();
    unique
}

/// What has been looked at so far, so a folder full of DLLs is read once rather than per event.
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
struct Scan {
    folders: HashSet<PathBuf>,
    files: HashMap<PathBuf, (u64, Option<SystemTime>)>,
}

#[cfg(any(target_os = "linux", test))]
impl Scan {
    /// Check every DLL in the temp folders, calling `watch` on each new folder before reading it
    /// so nothing written in between is missed.
    fn run(&mut self, prefix: &Path, mut watch: impl FnMut(&Path)) {
        for root in temp_roots(prefix) {
            self.folder(&root, &mut watch);
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    self.folder(&entry.path(), &mut watch);
                }
            }
        }
    }

    fn folder(&mut self, folder: &Path, watch: &mut impl FnMut(&Path)) {
        if self.folders.insert(folder.to_path_buf()) {
            watch(folder);
        }
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dll = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"));
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !is_dll || !meta.is_file() {
                continue;
            }
            let stamp = (meta.len(), meta.modified().ok());
            if self.files.get(&path) == Some(&stamp) {
                continue;
            }
            self.files.insert(path.clone(), stamp);
            match patch_file(&path) {
                Ok(0) => {}
                Ok(count) => tracing::info!(
                    ?path,
                    count,
                    "patched FreeArc's memory search so the installer cannot hang in it"
                ),
                Err(error) => {
                    tracing::warn!(?path, %error, "could not check a DLL for FreeArc's hang")
                }
            }
        }
    }
}

/// Patches `unarc.dll` between unpacking and loading, since Wine copies the code into memory on
/// load. inotify reacts in time; the periodic rescan covers filesystems without it.
pub async fn guard(prefix: PathBuf) {
    #[cfg(target_os = "linux")]
    {
        use futures_util::StreamExt;
        use inotify::{Inotify, WatchMask};

        const RESCAN: std::time::Duration = std::time::Duration::from_secs(1);
        let mask = WatchMask::CREATE | WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO;

        let mut scan = Scan::default();
        let stream = Inotify::init().and_then(|inotify| inotify.into_event_stream(vec![0u8; 4096]));
        let mut events = match stream {
            Ok(events) => Some(events),
            Err(error) => {
                tracing::warn!(%error, "no inotify, so FreeArc's DLL is checked once a second");
                None
            }
        };

        loop {
            let mut watches = events.as_ref().map(|events| events.watches());
            scan.run(&prefix, |folder| {
                if let Some(watches) = watches.as_mut() {
                    if let Err(error) = watches.add(folder, mask) {
                        tracing::debug!(?folder, %error, "could not watch a temp folder");
                    }
                }
            });

            match events.as_mut() {
                Some(stream) => {
                    // Any event, or the rescan interval, is reason to look again.
                    let _ = tokio::time::timeout(RESCAN, stream.next()).await;
                }
                None => tokio::time::sleep(RESCAN).await,
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Windows locks a loaded DLL against writing, and no other host runs Wine installers.
        let _ = prefix;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both copies of the search in FreeArc's `unarc.dll`, byte for byte, with code between.
    fn largest_memory_block() -> Vec<u8> {
        let copy = |free_call: u8, alloc_call: u8| {
            let mut code = vec![0x8B, 0x74, 0x24, 0x20]; // mov esi, [esp + 0x20]
            code.extend([0xC7, 0x06, 0x00, 0x00, 0x00, 0x00]); // mov dword [esi], 0
            code.extend(BOUNDS); // mov ebp, 0; mov edi, 0xFFFFFFFF
            code.extend([0x89, 0x34, 0x24]); // mov [esp], esi
            code.extend([0xE8, free_call, 0x01, 0x00, 0x00]); // call free
            code.extend(MIDPOINT); // lea eax, [ebp + edi]
            code.extend([0x89, 0xC3, 0xD1, 0xEB]); // mov ebx, eax; shr ebx, 1
            code.extend([0x89, 0x5C, 0x24, 0x04, 0x89, 0x34, 0x24]); // arguments
            code.extend([0xE8, alloc_call, 0x01, 0x00, 0x00, 0x83, 0x3E]); // call alloc; cmp
            code
        };
        let mut image = vec![0x90; 32];
        image.extend(copy(0xFF, 0xB1));
        image.extend([0x90; 32]);
        image.extend(copy(0xAB, 0x5D));
        image.extend([0x90; 32]);
        image
    }

    #[test]
    fn both_copies_of_the_search_are_found_and_only_their_upper_bounds_are_named() {
        let image = largest_memory_block();
        let offsets = overflowing_bounds(&image);
        assert_eq!(offsets.len(), 2, "{offsets:?}");
        for offset in offsets {
            // The top byte of `mov edi, 0xFFFFFFFF`, which becomes 0x7FFFFFFF.
            assert_eq!(image[offset], 0xFF);
            assert_eq!(image[offset - 4], 0xBF);
        }
    }

    #[test]
    fn a_lone_all_ones_constant_is_not_taken_for_the_search() {
        // `mov edi, 0xFFFFFFFF` is common; without the midpoint after it, it is someone else's.
        let mut code = vec![0x90; 8];
        code.extend(BOUNDS);
        code.extend([0x90; 64]);
        assert!(overflowing_bounds(&code).is_empty());
        assert!(overflowing_bounds(&[]).is_empty());
        // Bounds at the very end, with no room left for anything after them.
        assert!(overflowing_bounds(&BOUNDS).is_empty());
    }

    #[test]
    fn a_patched_dll_is_left_alone_the_second_time() {
        let dir = std::env::temp_dir().join(format!("gameyfin-unarc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dll = dir.join("unarc.dll");
        let original = largest_memory_block();
        std::fs::write(&dll, &original).unwrap();

        assert_eq!(patch_file(&dll).unwrap(), 2);
        let patched = std::fs::read(&dll).unwrap();
        assert_eq!(patched.len(), original.len(), "not truncated");
        let changed: Vec<usize> = (0..patched.len())
            .filter(|&i| patched[i] != original[i])
            .collect();
        assert_eq!(changed.len(), 2, "one byte per copy");
        assert!(changed.iter().all(|&i| patched[i] == PATCHED_TOP_BYTE));

        assert_eq!(patch_file(&dll).unwrap(), 0, "already patched");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_scan_finds_a_dll_in_an_inno_temp_folder_and_watches_its_folder() {
        let dir = std::env::temp_dir().join(format!("gameyfin-unarc-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let temp = dir.join("drive_c/users/steamuser/AppData/Local/Temp/is-AB12C.tmp");
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("unarc.DLL"), largest_memory_block()).unwrap();
        std::fs::write(temp.join("setup.ini"), largest_memory_block()).unwrap();

        let mut watched = Vec::new();
        let mut scan = Scan::default();
        scan.run(&dir, |folder| watched.push(folder.to_path_buf()));

        assert!(overflowing_bounds(&std::fs::read(temp.join("unarc.DLL")).unwrap()).is_empty());
        assert!(
            !overflowing_bounds(&std::fs::read(temp.join("setup.ini")).unwrap()).is_empty(),
            "only DLLs are touched"
        );
        let temp = temp.canonicalize().unwrap();
        assert!(watched.contains(&temp), "{watched:?}");

        // A second pass watches nothing new and has nothing left to patch.
        let before = watched.len();
        scan.run(&dir, |folder| watched.push(folder.to_path_buf()));
        assert_eq!(watched.len(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn the_guard_patches_a_dll_unpacked_after_it_started_well_before_the_rescan() {
        // What Inno Setup does: a fresh `is-*.tmp` folder, then the DLL written into it.
        let dir = std::env::temp_dir().join(format!("gameyfin-unarc-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let temp = dir.join("drive_c/users/steamuser/AppData/Local/Temp");
        std::fs::create_dir_all(&temp).unwrap();

        let guard = tokio::spawn(guard(dir.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let inno = temp.join("is-7Q2LK.tmp");
        std::fs::create_dir(&inno).unwrap();
        let dll = inno.join("unarc.dll");
        std::fs::write(&dll, largest_memory_block()).unwrap();

        // Well inside the one-second rescan, so it is the watch that caught it.
        let written = std::time::Instant::now();
        while !overflowing_bounds(&std::fs::read(&dll).unwrap()).is_empty() {
            assert!(
                written.elapsed() < std::time::Duration::from_millis(500),
                "not patched in time"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        guard.abort();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
