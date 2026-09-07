//! Picking the executable to launch from an extracted game directory.
//!
//! The old Gameyfin desktop client collects every `.exe` under the folder and, if there is
//! more than one, shows the user a list (`services/launcher_resolver.py`). For a typical
//! game that list is dozens of entries long, redistributables, crash handlers, tooling,
//! so the user has to know which one is the game.
//!
//! This module scores candidates instead, and only asks when the result is genuinely
//! unclear. The scoring is deliberately conservative: it prefers being unsure over being
//! confidently wrong, because launching the wrong binary can run an installer.

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

/// Filename stems that identify a setup program rather than the game.
///
/// These are excluded from launch candidates, but they are exactly what has to be *found*
/// when an archive turns out to contain an installer instead of a ready-to-run game.
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

/// Whether a filename looks like a setup program *for the game*.
///
/// Redistributables are installers too, `vcredist_x64.exe`, `dxwebsetup.exe`,
/// `oalinst.exe`, but offering one as "the setup program" sends the user to install
/// DirectX instead of their game.
pub fn looks_like_installer(path: &Path) -> bool {
    let Some(stem) = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
    else {
        return false;
    };

    // Note: EXCLUDED_STEMS is deliberately *not* consulted here. It exists to keep setup
    // programs out of the *launch* candidates, and it contains "setup", exactly what
    // this function is looking for.
    if is_redistributable(&stem) {
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

    // `setup.exe`, but also `setup_game_1.2.exe` and `GameSetup.exe`.
    INSTALLER_STEMS.iter().any(|marker| {
        stem == *marker || stem.starts_with(&format!("{marker}_")) || stem.ends_with(marker)
    })
}

/// Whether a filename names an uninstaller.
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

    // `unins000.exe` is Inno Setup's; the others cover NSIS and common variants.
    is_exe && (stem.starts_with("unins") || stem == "uninstall" || stem == "uninstaller")
}

/// Find an uninstaller in a game's install directory.
///
/// Running the game's own uninstaller removes registry entries and shortcuts that simply
/// deleting the folder would leave behind.
pub fn find_uninstaller(root: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && looks_like_uninstaller(p))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Find setup programs under a directory.
///
/// Some libraries hold an archive *containing* an installer: extracting it produces a
/// setup program, not a playable game. Surfacing those lets the app offer to run one
/// instead of leaving the user with a folder and no launchable file.
pub fn find_installers(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    walk(root, 0, 3, &mut |path| {
        if looks_like_installer(&path) {
            found.push(path);
        }
    })?;
    // Shallower first: the top-level setup.exe is the one to run.
    found.sort_by_key(|p| {
        (
            p.strip_prefix(root)
                .map(|r| r.components().count())
                .unwrap_or(0),
            p.clone(),
        )
    });
    Ok(found)
}

/// Walk a game directory, skipping folders that never hold anything runnable.
fn walk(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    visit: &mut impl FnMut(PathBuf),
) -> std::io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if EXCLUDED_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(&path, depth + 1, max_depth, visit)?;
        } else if file_type.is_file() {
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

/// The outcome of scanning a directory.
#[derive(Debug, Clone, PartialEq)]
pub enum Detection {
    /// One candidate stands clearly apart; safe to launch without asking.
    Confident(PathBuf),
    /// Several plausible candidates, best first, ask the user.
    Ambiguous(Vec<Candidate>),
    /// Nothing executable was found.
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

/// Scan `root` for launch candidates.
///
/// `title` is the game's name, used to reward a binary that resembles it.
pub fn detect(root: &Path, title: &str) -> std::io::Result<Detection> {
    let mut candidates = Vec::new();
    // Games nest a few levels at most; deeper is tooling or engine content.
    walk(root, 0, 5, &mut |path| {
        if let Some(score) = score_file(root, &path, title) {
            candidates.push(Candidate { path, score });
        }
    })?;

    if candidates.is_empty() {
        return Ok(Detection::None);
    }

    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            // Stable, predictable ordering for equal scores.
            .then_with(|| a.path.cmp(&b.path))
    });

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

/// Whether a file looks launchable on either platform.
///
/// On Windows the extension decides. On Linux there is no reliable extension, so this
/// accepts Windows executables (played through Proton) and extensionless files, which is
/// what native Linux builds normally ship.
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

/// Reduce a name to comparable letters and digits.
fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Whether a file starts with the `MZ` signature every Windows executable carries.
///
/// Wine's own diagnosis of a file that is not a PE is `ShellExecuteEx failed: Bad format`,
/// buried under a page of startup chatter, which reads as "Wine is broken" rather than
/// "that file is not a program". Checking two bytes first turns it into a sentence the
/// user can act on.
///
/// Deliberately shallow: this rejects what is obviously not a program (a truncated
/// download, a text file, a stub) rather than validating the PE format. A file that passes
/// here can still fail to run for reasons only Wine can determine.
pub fn looks_like_windows_program(path: &std::path::Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut signature = [0u8; 2];
    match file.read_exact(&mut signature) {
        Ok(()) => &signature == b"MZ",
        // Shorter than two bytes: certainly not a program.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn recognises_setup_programs() {
        assert!(looks_like_installer(Path::new("/g/setup.exe")));
        assert!(looks_like_installer(Path::new("/g/Setup.EXE")));
        assert!(looks_like_installer(Path::new("/g/install.exe")));
        assert!(looks_like_installer(Path::new("/g/setup_game_1.2.exe")));
        assert!(looks_like_installer(Path::new("/g/GameSetup.exe")));
        assert!(looks_like_installer(Path::new("/g/thing.msi")));
    }

    #[test]
    fn redistributables_are_not_the_games_installer() {
        // Straight from a real library: FTL ships these in `_Redist`.
        assert!(!looks_like_installer(Path::new(
            "/g/_Redist/dxwebsetup.exe"
        )));
        assert!(!looks_like_installer(Path::new(
            "/g/_Redist/vcredist_x86.exe"
        )));
        assert!(!looks_like_installer(Path::new("/g/_Redist/oalinst.exe")));
        assert!(!looks_like_installer(Path::new(
            "/g/UE4PrereqSetup_x64.exe"
        )));
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
    fn a_game_binary_is_not_an_installer() {
        assert!(!looks_like_installer(Path::new("/g/Celeste.exe")));
        assert!(!looks_like_installer(Path::new("/g/game.x86_64")));
        assert!(!looks_like_installer(Path::new("/g/readme.txt")));
    }

    #[test]
    fn recognises_uninstallers() {
        // Straight from a real install: GOG ships Inno Setup's uninstaller.
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
