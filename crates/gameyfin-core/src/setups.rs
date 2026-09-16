//! Ordering the setup programs in one download: the game, then its patches, then DLC. Names
//! are all there is to go on, so a plan only counts as confident when every file is accounted for.

use std::cmp::Ordering;
use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum SetupRole {
    Game,
    Patch,
    Dlc,
    /// Neither the game nor something recognisably for it, such as a disc menu.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSetup {
    /// As given, usually relative to the folder that was scanned.
    pub path: String,
    pub role: SetupRole,
    /// A patch to a version the game installer already is.
    pub superseded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetupPlan {
    /// In the order they should run.
    pub setups: Vec<PlannedSetup>,
    /// One game installer and nothing unexplained beside it, so it may run unattended.
    pub confident: bool,
}

impl SetupPlan {
    /// What an unattended install runs: everything but superseded patches and the unexplained.
    pub fn runnable(&self) -> impl Iterator<Item = &PlannedSetup> {
        self.setups
            .iter()
            .filter(|s| s.role != SetupRole::Other && !s.superseded)
    }
}

/// Words that name the file's kind rather than what it installs.
const NOISE: &[&str] = &[
    "setup",
    "install",
    "installer",
    "autorun",
    "gog",
    "galaxy",
    "v",
];
const PATCH_WORDS: &[&str] = &["patch", "patches", "update", "updates", "hotfix"];
const DLC_WORDS: &[&str] = &[
    "dlc",
    "dlcs",
    "addon",
    "addons",
    "expansion",
    "expansions",
    "soundtrack",
    "ost",
    "artbook",
    "bonus",
    "extras",
];

struct Parsed<'a> {
    path: &'a str,
    stem: String,
    /// The whole path, lowercase, for versions carried by a folder name.
    lower: String,
    depth: usize,
    /// Words of the file name with the noise dropped.
    words: Vec<String>,
    marker: Option<SetupRole>,
    autorun: bool,
}

pub fn plan(setups: &[String], title: &str) -> SetupPlan {
    let parsed: Vec<Parsed> = setups.iter().map(|path| parse(path)).collect();
    let title_key: String = words(title).concat();

    let mut roles: Vec<Option<SetupRole>> = vec![None; parsed.len()];
    // Unclaimed by any rule, which is what keeps a plan from being confident.
    let mut unexplained = vec![false; parsed.len()];

    // The title is the strongest signal: `setup_wall_world_1.2` is the game, and
    // `setup_wall_world_deep_threat_1.2` is something for it, whatever else it says.
    for (i, p) in parsed.iter().enumerate() {
        if let Some(rest) = after_title(&p.words, &title_key) {
            roles[i] = Some(match (slug(rest).is_empty(), p.marker) {
                (true, None) => SetupRole::Game,
                (_, Some(marker)) => marker,
                (false, None) => SetupRole::Dlc,
            });
        } else if let Some(marker) = p.marker {
            roles[i] = Some(marker);
        }
    }

    // Without a title match, the game is the plain setup every other plain name extends.
    if !roles.contains(&Some(SetupRole::Game)) {
        let plain: Vec<usize> = (0..parsed.len())
            .filter(|&i| roles[i].is_none() && !parsed[i].autorun)
            .collect();
        let slugs: Vec<Vec<String>> = plain.iter().map(|&i| slug(&parsed[i].words)).collect();
        if let Some(base) = base_of(&plain, &slugs, &parsed) {
            let base_slug = slugs[plain.iter().position(|&i| i == base).unwrap_or(0)].clone();
            for (&i, slug) in plain.iter().zip(&slugs) {
                roles[i] = Some(if i == base {
                    SetupRole::Game
                } else if !base_slug.is_empty() && slug.starts_with(&base_slug) {
                    SetupRole::Dlc
                } else {
                    unexplained[i] = true;
                    SetupRole::Other
                });
            }
        } else {
            // Several equally likely games: all offered as such, none chosen.
            for &i in &plain {
                roles[i] = Some(SetupRole::Game);
            }
        }
    }

    let mut planned: Vec<(PlannedSetup, &Parsed)> = parsed
        .iter()
        .zip(roles)
        .enumerate()
        .map(|(i, (p, role))| {
            let role = role.unwrap_or_else(|| {
                // An autorun beside a real setup is a disc menu, not a second installer.
                unexplained[i] = !p.autorun;
                SetupRole::Other
            });
            (
                PlannedSetup {
                    path: p.path.to_string(),
                    role,
                    superseded: false,
                },
                p,
            )
        })
        .collect();

    let games = planned
        .iter()
        .filter(|(s, _)| s.role == SetupRole::Game)
        .count();
    if games == 1 {
        let base = planned
            .iter()
            .find(|(s, _)| s.role == SetupRole::Game)
            .and_then(|(_, p)| versions(&p.stem).into_iter().next());
        if let Some(base) = base {
            for (setup, p) in planned.iter_mut() {
                if setup.role == SetupRole::Patch {
                    setup.superseded =
                        patch_version(p).is_some_and(|to| compare(&to, &base).is_le());
                }
            }
        }
    }

    planned.sort_by(|(a, pa), (b, pb)| {
        a.role
            .cmp(&b.role)
            .then_with(|| match a.role {
                SetupRole::Game => pa.depth.cmp(&pb.depth),
                SetupRole::Patch => match (patch_version(pa), patch_version(pb)) {
                    (Some(x), Some(y)) => compare(&x, &y),
                    _ => Ordering::Equal,
                },
                _ => Ordering::Equal,
            })
            .then_with(|| natural(pa.path).cmp(&natural(pb.path)))
    });

    SetupPlan {
        confident: games == 1 && !unexplained.contains(&true),
        setups: planned.into_iter().map(|(s, _)| s).collect(),
    }
}

fn parse(path: &str) -> Parsed<'_> {
    let as_path = Path::new(path);
    let stem = as_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let all = words(&stem);
    let folders: Vec<Vec<String>> = as_path
        .parent()
        .into_iter()
        .flat_map(|p| p.components())
        .map(|c| words(&c.as_os_str().to_string_lossy()))
        .collect();

    let has = |list: &[&str], words: &[String]| {
        words.iter().any(|w| list.contains(&w.as_str()))
            || (list == DLC_WORDS && words.windows(2).any(|w| w[0] == "add" && w[1] == "on"))
    };
    // A patch marker wins: a DLC's own update is still an update.
    let marker = if has(PATCH_WORDS, &all) || folders.iter().any(|f| has(PATCH_WORDS, f)) {
        Some(SetupRole::Patch)
    } else if has(DLC_WORDS, &all) || folders.iter().any(|f| has(DLC_WORDS, f)) {
        Some(SetupRole::Dlc)
    } else {
        None
    };

    Parsed {
        path,
        depth: folders.len(),
        autorun: all.iter().any(|w| w == "autorun"),
        words: all
            .into_iter()
            .filter(|w| !NOISE.contains(&w.as_str()))
            .collect(),
        stem: stem.to_lowercase(),
        lower: path.to_lowercase(),
        marker,
    }
}

/// Lowercase words, split at punctuation, case changes and letter-digit boundaries.
pub(crate) fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut previous: Option<char> = None;
    for c in text.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            previous = None;
            continue;
        }
        if let Some(p) = previous {
            let boundary = (p.is_lowercase() && c.is_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit());
            if boundary && !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        }
        current.extend(c.to_lowercase());
        previous = Some(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The words after a prefix spelling the title, compared without spaces so `tom_clancys`
/// still matches "Tom Clancy's".
fn after_title<'w>(words: &'w [String], title_key: &str) -> Option<&'w [String]> {
    if title_key.is_empty() {
        return None;
    }
    let mut joined = String::new();
    for (i, word) in words.iter().enumerate() {
        joined.push_str(word);
        if joined == title_key {
            return Some(&words[i + 1..]);
        }
        if joined.len() >= title_key.len() {
            return None;
        }
    }
    None
}

/// The identifying words: everything before the version starts.
fn slug(words: &[String]) -> Vec<String> {
    words
        .iter()
        .take_while(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .cloned()
        .collect()
}

/// The plain setup whose name all the others extend, or the shallowest nameless `setup.exe`.
fn base_of(plain: &[usize], slugs: &[Vec<String>], parsed: &[Parsed]) -> Option<usize> {
    let shortest = slugs.iter().map(Vec::len).min()?;
    let shortest_slugs: Vec<usize> = (0..plain.len())
        .filter(|&k| slugs[k].len() == shortest)
        .collect();
    let chosen = match shortest_slugs.as_slice() {
        [only] => *only,
        several => {
            // Same name at different depths: the top-level one is the game.
            let depth = |k: usize| parsed[plain[k]].depth;
            let top = several.iter().map(|&k| depth(k)).min()?;
            let at_top: Vec<usize> = several
                .iter()
                .copied()
                .filter(|&k| depth(k) == top)
                .collect();
            match at_top.as_slice() {
                [only] if slugs[*only].is_empty() => *only,
                _ => return None,
            }
        }
    };
    Some(plain[chosen])
}

/// Dotted version numbers in a name, in order. A bare number is a build or a bitness.
fn versions(stem: &str) -> Vec<Vec<u64>> {
    let mut found = Vec::new();
    let mut current = String::new();
    for c in stem.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() || (c == '.' && !current.is_empty()) {
            current.push(c);
            continue;
        }
        let trimmed = current.trim_end_matches('.');
        if trimmed.contains('.') {
            found.push(trimmed.split('.').filter_map(|n| n.parse().ok()).collect());
        }
        current.clear();
    }
    found
}

/// The version a patch brings the game to: GOG names them `patch_x_1.0_(1)_to_1.1_(2)`.
fn patch_target(stem: &str) -> Option<Vec<u64>> {
    // The last one, so a title containing the word does not confuse it.
    let at = ["_to_", " to ", "-to-"]
        .iter()
        .filter_map(|sep| stem.rfind(sep))
        .max()?;
    versions(&stem[at..]).into_iter().next()
}

/// The version a patch brings the game to: named after `to` in its file, else the last version
/// anywhere in its path, as in `Game.Update.v1.3.10/PATCH.exe`.
fn patch_version(parsed: &Parsed) -> Option<Vec<u64>> {
    patch_target(&parsed.stem).or_else(|| versions(&parsed.lower).pop())
}

fn compare(a: &[u64], b: &[u64]) -> Ordering {
    let len = a.len().max(b.len());
    (0..len)
        .map(|i| {
            a.get(i)
                .copied()
                .unwrap_or(0)
                .cmp(&b.get(i).copied().unwrap_or(0))
        })
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal)
}

/// Sorts `dlc_2` before `dlc_10`.
fn natural(path: &str) -> Vec<(u8, u64, String)> {
    words(path)
        .into_iter()
        .map(|w| match w.parse::<u64>() {
            Ok(n) => (0, n, String::new()),
            Err(_) => (1, 0, w),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(setups: &[&str], title: &str) -> SetupPlan {
        let owned: Vec<String> = setups.iter().map(|s| s.to_string()).collect();
        plan(&owned, title)
    }

    fn order(plan: &SetupPlan) -> Vec<(&str, SetupRole)> {
        plan.setups
            .iter()
            .map(|s| (s.path.as_str(), s.role))
            .collect()
    }

    #[test]
    fn words_split_where_a_person_would() {
        assert_eq!(
            words("setup_wall_world_1.2.4_(64bit)"),
            ["setup", "wall", "world", "1", "2", "4", "64", "bit"]
        );
        assert_eq!(words("GameSetupDLC2"), ["game", "setup", "dlc", "2"]);
        assert!(words("").is_empty());
    }

    #[test]
    fn a_gog_game_with_dlc_and_patches_runs_in_order() {
        use SetupRole::*;
        let plan = planned(
            &[
                "setup_wall_world_deep_threat_1.2.4.513_(64bit)_(67993).exe",
                "patch_wall_world_1.2.4.513_(67993)_to_1.2.5_(68100).exe",
                "setup_wall_world_1.2.4.513_(64bit)_(67993).exe",
            ],
            "Wall World",
        );
        assert!(plan.confident);
        assert_eq!(
            order(&plan),
            [
                ("setup_wall_world_1.2.4.513_(64bit)_(67993).exe", Game),
                (
                    "patch_wall_world_1.2.4.513_(67993)_to_1.2.5_(68100).exe",
                    Patch
                ),
                (
                    "setup_wall_world_deep_threat_1.2.4.513_(64bit)_(67993).exe",
                    Dlc
                ),
            ]
        );
        assert!(plan.setups.iter().all(|s| !s.superseded));
    }

    #[test]
    fn a_patch_the_game_installer_already_includes_is_superseded() {
        let plan = planned(
            &[
                "setup_hades_1.38.exe",
                "patch_hades_1.36_(1)_to_1.37_(2).exe",
                "patch_hades_1.38_(3)_to_1.39_(4).exe",
            ],
            "Hades",
        );
        assert!(plan.confident);
        let superseded: Vec<bool> = plan.setups.iter().map(|s| s.superseded).collect();
        assert_eq!(superseded, [false, true, false]);
        assert_eq!(plan.runnable().count(), 2);
    }

    #[test]
    fn patches_sort_by_the_version_they_bring() {
        let plan = planned(
            &[
                "patch_x_1.9_to_1.10.exe",
                "patch_x_1.8_to_1.9.exe",
                "setup_x.exe",
            ],
            "X",
        );
        let paths: Vec<&str> = plan.setups.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "setup_x.exe",
                "patch_x_1.8_to_1.9.exe",
                "patch_x_1.9_to_1.10.exe"
            ]
        );
    }

    #[test]
    fn scene_updates_in_their_own_folders_run_oldest_first() {
        let update =
            |v: &str| format!("patches/Escape.From.Duckov.Update.v{v}-TENOKE/Update/PATCH.exe");
        let plan = planned(
            &[
                &update("2.3.30"),
                "setup.exe",
                &update("1.3.10"),
                &update("2.1.3"),
                &update("1.0.33"),
            ],
            "Escape from Duckov",
        );
        assert!(plan.confident);
        assert_eq!(plan.setups[0].path, "setup.exe");
        assert_eq!(plan.setups[0].role, SetupRole::Game);
        let patches: Vec<&str> = plan.setups[1..].iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            patches,
            [
                update("1.0.33"),
                update("1.3.10"),
                update("2.1.3"),
                update("2.3.30")
            ]
        );
        assert!(plan.setups[1..]
            .iter()
            .all(|s| s.role == SetupRole::Patch && !s.superseded));
    }

    #[test]
    fn folders_name_updates_and_dlc() {
        use SetupRole::*;
        let plan = planned(
            &["setup.exe", "Update/setup.exe", "DLC/Season Pass/setup.exe"],
            "Something Else Entirely",
        );
        assert!(plan.confident);
        assert_eq!(
            order(&plan),
            [
                ("setup.exe", Game),
                ("Update/setup.exe", Patch),
                ("DLC/Season Pass/setup.exe", Dlc),
            ]
        );
    }

    #[test]
    fn without_a_title_match_the_shortest_shared_name_is_the_game() {
        use SetupRole::*;
        let plan = planned(
            &[
                "setup_foo_bar_2.0.exe",
                "setup_foo_1.0.exe",
                "setup_foo_baz.exe",
            ],
            "Unrelated",
        );
        assert!(plan.confident);
        assert_eq!(
            plan.setups[0],
            PlannedSetup {
                path: "setup_foo_1.0.exe".into(),
                role: Game,
                superseded: false,
            }
        );
        assert!(plan.setups[1..].iter().all(|s| s.role == Dlc));
    }

    #[test]
    fn two_equally_likely_games_are_not_confident() {
        let plan = planned(&["setup_alpha.exe", "setup_beta.exe"], "Gamma");
        assert!(!plan.confident);
        let unrelated = planned(
            &[
                "setup_alpha.exe",
                "setup_alpha_x.exe",
                "install_beta_long.exe",
            ],
            "",
        );
        assert!(!unrelated.confident);
        assert_eq!(unrelated.setups.last().unwrap().role, SetupRole::Other);
    }

    #[test]
    fn a_disc_menu_beside_the_setup_is_left_out() {
        let plan = planned(&["autorun.exe", "setup.exe"], "Game");
        assert!(plan.confident);
        assert_eq!(plan.setups[0].role, SetupRole::Game);
        assert_eq!(plan.setups[1].role, SetupRole::Other);
        assert_eq!(plan.runnable().count(), 1);
    }

    #[test]
    fn a_lone_setup_is_the_game() {
        let plan = planned(&["GameSetup.exe"], "Totally Different");
        assert!(plan.confident);
        assert_eq!(plan.setups[0].role, SetupRole::Game);
    }

    #[test]
    fn only_patches_leave_nothing_to_install_first() {
        let plan = planned(&["patch_x_1.0_to_1.1.exe"], "X");
        assert!(!plan.confident);
        assert_eq!(plan.setups[0].role, SetupRole::Patch);
    }

    #[test]
    fn titles_match_across_punctuation() {
        let plan = planned(
            &[
                "setup_tom_clancys_game_1.0.exe",
                "setup_tom_clancys_game_extra_map.exe",
            ],
            "Tom Clancy's Game",
        );
        assert!(plan.confident);
        assert_eq!(plan.setups[0].path, "setup_tom_clancys_game_1.0.exe");
        assert_eq!(plan.setups[1].role, SetupRole::Dlc);
    }

    #[test]
    fn dlc_numbers_sort_naturally() {
        let plan = planned(&["setup.exe", "dlc_10.exe", "dlc_2.exe"], "");
        let paths: Vec<&str> = plan.setups.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["setup.exe", "dlc_2.exe", "dlc_10.exe"]);
    }

    #[test]
    fn nothing_to_plan_is_not_confident() {
        assert_eq!(planned(&[], "X"), SetupPlan::default());
    }
}
