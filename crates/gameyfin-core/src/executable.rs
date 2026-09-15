//! Picking the executable to launch from an extracted game directory by scoring candidates,
//! conservatively: it prefers asking over being confidently wrong.

use std::path::{Path, PathBuf};

/// Directories that never contain the game itself.
const EXCLUDED_DIRS: &[&str] = &[
    "_commonredist",
    "commonredist",
    "_redist",
    "redist",
    "redistributable",
    "redistributables",
    "_redistributables",
    "directx",
    "_directx",
    "dotnet",
    "vcredist",
    "$plugins",
    "__installer",
    "dxsetup",
    "prerequisites",
    "_prerequisites",
];

/// Filename stems that are support tooling rather than the game.
const EXCLUDED_STEMS: &[&str] = &[
    "unins000",
    "uninstall",
    "uninstaller",
    "setup",
    "install",
    "installer",
    "vcredist_x86",
    "vcredist_x64",
    "dxsetup",
    "dotnetfx",
    "oalinst",
    "crashreporter",
    "crashhandler",
    "crashpad_handler",
    "unitycrashhandler32",
    "unitycrashhandler64",
    "ue4prereqsetup_x64",
    "ueprereqsetup_x64",
    "notification_helper",
];

/// Subfolder levels searched below the game's folder. Release names and engine layouts
/// (`Binaries/Win64`) nest deep; past these it is assets, not programs.
const LAUNCH_DEPTH: usize = 8;
const SETUP_DEPTH: usize = 6;
/// Uninstallers sit at the top, or in a folder of their own just below it.
const UNINSTALLER_DEPTH: usize = 1;

/// Words that name an update program, as releases ship beside the game's setup.
const PATCH_WORDS: &[&str] = &["patch", "update", "hotfix"];

/// Filename stems that identify a setup program rather than the game.
const INSTALLER_STEMS: &[&str] = &["setup", "install", "installer", "autorun"];

/// Whether a filename names a runtime redistributable rather than a game.
fn is_redistributable(stem: &str) -> bool {
    const MARKERS: &[&str] = &[
        "vcredist",
        "vc_redist",
        "directx",
        "dxweb",
        "dxsetup",
        "dotnet",
        "ndp4",
        "oalinst",
        "openal",
        "physx",
        "xnafx",
        "ue4prereq",
        "ueprereq",
    ];
    MARKERS.iter().any(|marker| stem.contains(marker))
}

/// Whether a filename looks like a setup program *for the game* (redistributables excluded).
pub fn looks_like_installer(path: &Path) -> bool {
    let Some(stem) = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
    else {
        return false;
    };

    // EXCLUDED_STEMS is deliberately not consulted here: it contains "setup", which is
    // exactly what this function looks for. "uninstall" ends in "install".
    if is_redistributable(&stem) || stem.starts_with("unins") {
        return false;
    }
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if extension == "msi" {
        return true;
    }
    if extension != "exe" {
        return false;
    }

    INSTALLER_STEMS.iter().any(|marker| {
        stem == *marker || stem.starts_with(&format!("{marker}_")) || stem.ends_with(marker)
    })
}

/// Whether a program looks like an update for the game, such as `PATCH.exe` or
/// `Game.Update.v1.2.exe`. `Updater.exe` is a launcher's own tool, so whole words only.
pub fn looks_like_patch(path: &Path) -> bool {
    let Some(stem) = path.file_stem().map(|s| s.to_string_lossy()) else {
        return false;
    };
    let lower = stem.to_ascii_lowercase();
    let runnable = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("msi"));
    runnable
        && !is_redistributable(&lower)
        && !lower.starts_with("unins")
        && crate::setups::words(&stem)
            .iter()
            .any(|w| PATCH_WORDS.contains(&w.as_str()))
}

pub fn looks_like_uninstaller(path: &Path) -> bool {
    let Some(stem) = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
    else {
        return false;
    };
    let is_exe = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));

    is_exe && (stem.starts_with("unins") || stem == "uninstall" || stem == "uninstaller")
}

/// Find an uninstaller in a game's install directory, so registry entries and shortcuts
/// that deleting the folder would leave behind get cleaned up.
pub fn find_uninstaller(root: &Path) -> Option<PathBuf> {
    let mut found = Vec::new();
    walk(root, 0, UNINSTALLER_DEPTH, &mut |path| {
        if looks_like_uninstaller(&path) {
            found.push(path);
        }
    })
    .ok()?;
    found.sort_by_key(|p| (depth_below(root, p), p.clone()));
    found.into_iter().next()
}

fn depth_below(root: &Path, path: &Path) -> usize {
    path.strip_prefix(root)
        .map(|r| r.components().count())
        .unwrap_or(0)
}

/// Find setup programs under a directory, for archives that contain an installer rather
/// than a ready-to-run game.
pub fn find_installers(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    find_programs(root, looks_like_installer)
}

/// Setup and update programs in a download. Not for an installed game, whose own
/// `update.exe` is part of it.
pub fn find_download_setups(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    find_programs(root, |path| {
        looks_like_installer(path) || looks_like_patch(path)
    })
}

/// Every program in a download, redistributables included, for picking one the names did not
/// give away.
pub fn find_download_programs(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    walk_folders(root, 0, SETUP_DEPTH, false, &mut |path| {
        let runnable = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("msi"));
        if runnable && !looks_like_uninstaller(&path) {
            found.push(path);
        }
    })?;
    found.sort_by_key(|p| (depth_below(root, p), p.clone()));
    Ok(found)
}

fn find_programs(root: &Path, wanted: impl Fn(&Path) -> bool) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    walk(root, 0, SETUP_DEPTH, &mut |path| {
        if wanted(&path) {
            found.push(path);
        }
    })?;
    // Shallower first: the top-level setup.exe is the one to run.
    found.sort_by_key(|p| (depth_below(root, p), p.clone()));
    Ok(found)
}

/// Walk a game directory, skipping folders that never hold anything runnable. Only the top
/// folder must be readable: one locked subfolder would otherwise hide every program in the game.
fn walk(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    visit: &mut impl FnMut(PathBuf),
) -> std::io::Result<()> {
    walk_folders(dir, depth, max_depth, true, visit)
}

/// `skip_excluded` leaves out redistributable and prerequisite folders.
fn walk_folders(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    skip_excluded: bool,
    visit: &mut impl FnMut(PathBuf),
) -> std::io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if skip_excluded && EXCLUDED_DIRS.contains(&name.as_str()) {
                continue;
            }
            if let Err(e) = walk_folders(&path, depth + 1, max_depth, skip_excluded, visit) {
                tracing::debug!(?path, error = %e, "skipping a folder that cannot be read");
            }
        } else if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            // A linked file, such as `game -> game.x86_64`, is followed; a linked folder is
            // not, since it can loop.
            visit(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub score: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Detection {
    /// One candidate stands clearly apart; safe to launch without asking.
    Confident(PathBuf),
    /// Several plausible candidates, best first, ask the user.
    Ambiguous(Vec<Candidate>),
    None,
}

impl Detection {
    pub fn confident(&self) -> Option<&Path> {
        match self {
            Detection::Confident(p) => Some(p),
            _ => None,
        }
    }
}

/// How much better the top candidate must be before it is chosen unattended.
const DECISIVE_MARGIN: i32 = 15;

/// Every launch candidate under `root`, best first, so a picker can offer the runner-up.
pub fn candidates(root: &Path, title: &str) -> std::io::Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    walk(root, 0, LAUNCH_DEPTH, &mut |path| {
        if let Some(score) = score_file(root, &path, title) {
            candidates.push(Candidate { path, score });
        }
    })?;

    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            // Stable, predictable ordering for equal scores.
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(candidates)
}

/// The one candidate worth launching unattended. `title` rewards a binary resembling it.
pub fn detect(root: &Path, title: &str) -> std::io::Result<Detection> {
    let mut candidates = candidates(root, title)?;

    if candidates.is_empty() {
        return Ok(Detection::None);
    }

    if candidates.len() == 1 {
        return Ok(Detection::Confident(candidates.remove(0).path));
    }
    if candidates[0].score - candidates[1].score >= DECISIVE_MARGIN {
        return Ok(Detection::Confident(candidates.remove(0).path));
    }

    Ok(Detection::Ambiguous(candidates))
}

/// Score a file, or `None` if it is not a launch candidate at all.
fn score_file(root: &Path, path: &Path, title: &str) -> Option<i32> {
    if !is_executable(path) {
        return None;
    }

    let stem = path.file_stem()?.to_string_lossy().to_ascii_lowercase();

    if EXCLUDED_STEMS.contains(&stem.as_str()) {
        return None;
    }

    let mut score = 0;

    // Shallower is better: the launcher usually sits at the top of the game folder.
    let depth = path
        .strip_prefix(root)
        .ok()?
        .components()
        .count()
        .saturating_sub(1);
    score += match depth {
        0 => 30,
        1 => 18,
        2 => 8,
        _ => 0,
    };

    // Resemblance to the game's title is the strongest positive signal.
    let normalized_title = normalize(title);
    let normalized_stem = normalize(&stem);
    if !normalized_title.is_empty() {
        if normalized_stem == normalized_title {
            score += 40;
        } else if normalized_stem.contains(&normalized_title)
            || normalized_title.contains(&normalized_stem)
        {
            score += 22;
        }
    }

    // Common conventions for the real entry point.
    if stem == "start" || stem == "launch" || stem == "launcher" || stem == "play" {
        score += 10;
    }

    // Editors and dedicated servers ship alongside the game but are not it.
    if stem.ends_with("-server") || stem.ends_with("_server") || stem.contains("dedicated") {
        score -= 25;
    }
    if stem.contains("editor") || stem.contains("config") || stem.contains("settings") {
        score -= 15;
    }

    Some(score)
}

/// Whether a file looks launchable on either platform (Windows executables run via Proton).
fn is_executable(path: &Path) -> bool {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
    {
        Some(ext) => matches!(ext.as_str(), "exe" | "x86_64" | "x86" | "sh" | "appimage"),
        None => {
            // Extensionless: only interesting if the OS marks it executable.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(path)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            }
            #[cfg(not(unix))]
            {
                false
            }
        }
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Whether a file starts with the `MZ` signature, checked up front so a non-PE file gets an
/// actionable message instead of Wine's buried `Bad format`. Shallow: not a PE validation.
pub fn looks_like_windows_program(path: &std::path::Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut signature = [0u8; 2];
    match file.read_exact(&mut signature) {
        Ok(()) => &signature == b"MZ",
        Err(_) => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsArch {
    X86,
    X86_64,
    Arm64,
    Other,
}

/// Read the machine field of a PE header: `MZ`, the offset at 0x3c, then `PE\0\0`.
///
/// `None` when the file is not a PE at all, which is not the same as an unknown machine.
pub fn windows_arch(path: &std::path::Path) -> Option<WindowsArch> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    let mut mz = [0u8; 2];
    file.read_exact(&mut mz).ok()?;
    if &mz != b"MZ" {
        return None;
    }

    file.seek(SeekFrom::Start(0x3c)).ok()?;
    let mut offset = [0u8; 4];
    file.read_exact(&mut offset).ok()?;
    file.seek(SeekFrom::Start(u32::from_le_bytes(offset) as u64))
        .ok()?;

    let mut header = [0u8; 6];
    file.read_exact(&mut header).ok()?;
    if &header[..4] != b"PE\0\0" {
        return None;
    }

    Some(match u16::from_le_bytes([header[4], header[5]]) {
        0x014c => WindowsArch::X86,
        0x8664 => WindowsArch::X86_64,
        0xaa64 => WindowsArch::Arm64,
        _ => WindowsArch::Other,
    })
}

/// Whether a program is 32-bit, which is what decides if it needs 32-bit libraries.
pub fn is_32bit_windows_program(path: &std::path::Path) -> bool {
    windows_arch(path) == Some(WindowsArch::X86)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with just enough of a PE header to read its machine field.
    fn fake_pe(dir: &std::path::Path, name: &str, machine: u16) -> PathBuf {
        let mut bytes = vec![0u8; 0x100];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn the_pe_machine_field_tells_32_bit_from_64_bit() {
        let dir = std::env::temp_dir().join(format!("gameyfin-pe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let setup = fake_pe(&dir, "setup.exe", 0x014c);
        let game = fake_pe(&dir, "game.exe", 0x8664);
        assert_eq!(windows_arch(&setup), Some(WindowsArch::X86));
        assert_eq!(windows_arch(&game), Some(WindowsArch::X86_64));
        assert!(is_32bit_windows_program(&setup));
        assert!(!is_32bit_windows_program(&game));

        // Not a PE at all, and a file too short to hold a header.
        let text = dir.join("readme.txt");
        std::fs::write(&text, b"not a program").unwrap();
        assert_eq!(windows_arch(&text), None);
        let stub = dir.join("stub.exe");
        std::fs::write(&stub, b"MZ").unwrap();
        assert_eq!(windows_arch(&stub), None);
        assert!(!is_32bit_windows_program(&stub));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("gameyfin-exe-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, rel: &str) -> &Self {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            self
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_the_games_own_setup_looks_like_an_installer() {
        // The false cases are what FTL ships in `_Redist`.
        for (path, expected) in [
            ("/g/setup.exe", true),
            ("/g/Setup.EXE", true),
            ("/g/install.exe", true),
            ("/g/setup_game_1.2.exe", true),
            ("/g/GameSetup.exe", true),
            ("/g/thing.msi", true),
            ("/g/Uninstall.exe", false),
            ("/g/unins000.exe", false),
            ("/g/_Redist/dxwebsetup.exe", false),
            ("/g/_Redist/vcredist_x86.exe", false),
            ("/g/_Redist/oalinst.exe", false),
            ("/g/UE4PrereqSetup_x64.exe", false),
            ("/g/Celeste.exe", false),
            ("/g/game.x86_64", false),
            ("/g/readme.txt", false),
        ] {
            assert_eq!(looks_like_installer(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn update_programs_are_named_by_whole_words() {
        for (path, expected) in [
            ("/g/PATCH.exe", true),
            ("/g/Escape.From.Duckov.Update.v2.3.30-TENOKE.exe", true),
            ("/g/GamePatch.exe", true),
            ("/g/hotfix_2.msi", true),
            ("/g/Updater.exe", false),
            ("/g/Patch.bin", false),
            ("/g/dxwebupdate.exe", false),
            ("/g/Celeste.exe", false),
        ] {
            assert_eq!(looks_like_patch(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn a_download_offers_its_patches_but_an_install_does_not() {
        let t = Tree::new("duckov");
        t.file("setup.exe")
            .file("fg-01.bin")
            .file("MD5/QuickSFV.exe")
            .file("patches/Escape.From.Duckov.Update.v1.0.33-TENOKE/Update/PATCH.exe")
            .file("patches/Escape.From.Duckov.Update.v1.0.33-TENOKE/Update/Patch.bin")
            .file("patches/Escape.From.Duckov.Update.v2.3.30-TENOKE/Update/PATCH.exe");

        let download = find_download_setups(t.path()).unwrap();
        assert_eq!(download.len(), 3);
        assert_eq!(download[0], t.path().join("setup.exe"));
        assert_eq!(find_installers(t.path()).unwrap().len(), 1);
    }

    #[test]
    fn every_program_in_a_download_can_be_offered() {
        let t = Tree::new("all-programs");
        t.file("setup.exe")
            .file("fg-01.bin")
            .file("MD5/QuickSFV.exe")
            .file("_CommonRedist/vcredist_x64.exe")
            .file("tools/thing.msi")
            .file("unins000.exe");
        let names: Vec<String> = find_download_programs(t.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["setup.exe", "QuickSFV.exe", "vcredist_x64.exe", "thing.msi"]
        );
    }

    #[test]
    fn a_redistributable_folder_is_skipped_entirely() {
        let t = Tree::new("ftl-redist");
        t.file("FTLGame.exe")
            .file("_Redist/dxwebsetup.exe")
            .file("_Redist/vcredist_x86.exe");

        // Nothing here is the game's own setup, so nothing should be offered.
        assert!(find_installers(t.path()).unwrap().is_empty());
    }

    #[test]
    fn recognises_uninstallers() {
        // GOG ships Inno Setup's uninstaller.
        assert!(looks_like_uninstaller(Path::new("/g/unins000.exe")));
        assert!(looks_like_uninstaller(Path::new("/g/Uninstall.exe")));
        assert!(!looks_like_uninstaller(Path::new("/g/unins000.dat")));
        assert!(!looks_like_uninstaller(Path::new("/g/WallWorld.exe")));
    }

    #[test]
    fn finds_the_uninstaller_in_an_install() {
        let t = Tree::new("uninstaller");
        t.file("WallWorld.exe")
            .file("unins000.exe")
            .file("unins000.dat");

        let found = find_uninstaller(t.path()).unwrap();
        assert_eq!(found.file_name().unwrap(), "unins000.exe");
    }

    #[test]
    fn an_uninstaller_one_folder_down_is_found() {
        let t = Tree::new("uninstaller-nested");
        t.file("Game.exe").file("Uninstall/uninstall.exe");
        let found = find_uninstaller(t.path()).unwrap();
        assert_eq!(found, t.path().join("Uninstall").join("uninstall.exe"));
    }

    #[test]
    fn a_deeply_nested_game_is_still_found() {
        let t = Tree::new("deep");
        t.file("Release/Game v1.0/Game/Binaries/Win64/Shipping/Game.exe");
        let d = detect(t.path(), "Game").unwrap();
        assert_eq!(d.confident().unwrap().file_name().unwrap(), "Game.exe");
    }

    #[test]
    fn setups_inside_release_folders_are_found() {
        let t = Tree::new("deep-setups");
        t.file("Release/GOG/Game/Installer/setup.exe");
        assert_eq!(find_installers(t.path()).unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_locked_folder_does_not_hide_the_rest() {
        use std::os::unix::fs::PermissionsExt;

        let t = Tree::new("locked");
        t.file("Celeste.exe").file("locked/secret.exe");
        let locked = t.path().join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let found = candidates(t.path(), "Celeste");
        // Restored first, so the tree can be cleaned up whatever the assertion says.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(found
            .unwrap()
            .iter()
            .any(|c| c.path.ends_with("Celeste.exe")));
    }

    #[test]
    fn no_uninstaller_is_not_an_error() {
        let t = Tree::new("no-uninstaller");
        t.file("Celeste.exe");
        assert!(find_uninstaller(t.path()).is_none());
    }

    #[test]
    fn finds_installers_shallowest_first() {
        let t = Tree::new("installers");
        t.file("setup.exe")
            .file("bin/install.exe")
            .file("Celeste.exe");

        let found = find_installers(t.path()).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].file_name().unwrap(), "setup.exe");
    }

    #[test]
    fn installers_in_redistributable_folders_are_ignored() {
        // vcredist's own setup must never be offered as the game's installer.
        let t = Tree::new("redist-installers");
        t.file("_CommonRedist/setup.exe").file("setup.exe");

        let found = find_installers(t.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], t.path().join("setup.exe"));
    }

    #[test]
    fn a_lone_executable_is_confident() {
        let t = Tree::new("lone");
        t.file("Celeste.exe");
        let d = detect(t.path(), "Celeste").unwrap();
        assert_eq!(d.confident().unwrap().file_name().unwrap(), "Celeste.exe");
    }

    #[test]
    fn title_match_beats_redistributables() {
        let t = Tree::new("redist");
        t.file("Celeste.exe")
            .file("_CommonRedist/vcredist_x64.exe")
            .file("DirectX/DXSETUP.exe");
        // Excluded directories are not even scanned.
        let d = detect(t.path(), "Celeste").unwrap();
        assert_eq!(d.confident().unwrap().file_name().unwrap(), "Celeste.exe");
    }

    #[test]
    fn uninstallers_are_never_candidates() {
        let t = Tree::new("uninst");
        t.file("Celeste.exe").file("unins000.exe").file("setup.exe");
        let d = detect(t.path(), "Celeste").unwrap();
        assert_eq!(d.confident().unwrap().file_name().unwrap(), "Celeste.exe");
    }

    #[test]
    fn shallower_wins_when_names_are_equal() {
        let t = Tree::new("depth");
        t.file("Game.exe").file("bin/x64/Game.exe");
        let d = detect(t.path(), "Some Other Title").unwrap();
        assert_eq!(d.confident().unwrap(), t.path().join("Game.exe"));
    }

    #[test]
    fn dedicated_server_is_demoted() {
        let t = Tree::new("server");
        t.file("Valheim.exe").file("valheim_server.exe");
        let d = detect(t.path(), "Valheim").unwrap();
        assert_eq!(d.confident().unwrap().file_name().unwrap(), "Valheim.exe");
    }

    #[test]
    fn two_indistinguishable_games_are_ambiguous() {
        let t = Tree::new("ambiguous");
        // Neither resembles the title and both sit at the root.
        t.file("AlphaLauncher.exe").file("BetaTool.exe");
        match detect(t.path(), "Completely Different").unwrap() {
            Detection::Ambiguous(c) => assert_eq!(c.len(), 2),
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn empty_directory_detects_nothing() {
        let t = Tree::new("empty");
        assert_eq!(detect(t.path(), "Anything").unwrap(), Detection::None);
    }

    #[test]
    fn non_executables_are_ignored() {
        let t = Tree::new("data");
        t.file("readme.txt").file("data.pak").file("art.png");
        assert_eq!(detect(t.path(), "Game").unwrap(), Detection::None);
    }

    #[test]
    fn punctuation_and_case_do_not_defeat_title_matching() {
        let t = Tree::new("punct");
        t.file("ReturnOfTheObraDinn.exe")
            .file("crashpad_handler.exe");
        let d = detect(t.path(), "Return of the Obra Dinn!").unwrap();
        assert_eq!(
            d.confident().unwrap().file_name().unwrap(),
            "ReturnOfTheObraDinn.exe"
        );
    }

    #[test]
    fn ambiguous_candidates_are_ranked_best_first() {
        let t = Tree::new("ranked");
        t.file("ZzzTool.exe").file("bin/Hades.exe");
        match detect(t.path(), "Hades").unwrap() {
            Detection::Ambiguous(c) => assert!(c[0].path.ends_with("Hades.exe")),
            // A decisive margin is also an acceptable outcome here.
            Detection::Confident(p) => assert!(p.ends_with("Hades.exe")),
            other => panic!("unexpected {other:?}"),
        }
    }
}
