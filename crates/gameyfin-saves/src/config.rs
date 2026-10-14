//! Ludusavi's `config.yaml`. Redirects map the account name and install directory onto targets
//! identical on every machine, so a backup restores under a different username.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::SaveResult;

/// Synthetic redirect targets. Arbitrary but fixed, they only have to be identical
/// across machines, and distinctive enough never to collide with a real path.
pub const HOME_TARGET: &str = "/gameyfin/home";
/// What every synthetic target starts with. A restored path still under it was never
/// mapped back onto a real folder.
pub const SYNTHETIC_ROOT: &str = "/gameyfin/";
/// The Linux home of a machine running a Windows game, which is not the home that game
/// writes its saves into. Its own target, so the two never collapse onto each other.
pub const HOST_HOME_TARGET: &str = "/gameyfin/host-home";
pub const INSTALL_TARGET: &str = "/gameyfin/install";
/// Documents inside the travelling home, given its own redirect wherever it lives elsewhere.
pub const DOCUMENTS_TARGET: &str = "/gameyfin/home/Documents";

/// A location Ludusavi should scan, and how to interpret its layout.
#[derive(Debug, Clone, Serialize)]
pub struct Root {
    pub store: RootStore,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RootStore {
    /// A Steam library: Ludusavi finds Proton saves and registry files under `compatdata` itself.
    Steam,
    Heroic,
    Lutris,
    Gog,
    /// A bare Wine prefix, the folder that directly contains `drive_c`. File saves only;
    /// registry values are not captured from generic prefixes.
    OtherWine,
    OtherHome,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RedirectKind {
    Backup,
    Restore,
    Bidirectional,
}

#[derive(Debug, Clone, Serialize)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub source: String,
    pub target: String,
}

/// A game the manifest does not cover, or covers wrongly.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomGame {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<String>,
    /// `extend` merges with a manifest entry of the same name; `override` replaces it.
    pub integration: CustomGameIntegration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CustomGameIntegration {
    Override,
    Extend,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupSection {
    path: String,
    format: FormatSection,
    retention: RetentionSection,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FormatSection {
    chosen: String,
    zip: ZipSection,
}

#[derive(Debug, Clone, Serialize)]
struct ZipSection {
    compression: String,
}

#[derive(Debug, Clone, Serialize)]
struct RetentionSection {
    full: u8,
    differential: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RestoreSection {
    path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CloudSection {
    /// Ludusavi's own rclone sync is off: the Gameyfin server is the sync target.
    synchronize: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseSection {
    /// Off: Gameyfin manages the version, so a GitHub check on every run is wasted.
    check: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    roots: Vec<Root>,
    release: ReleaseSection,
    backup: BackupSection,
    restore: RestoreSection,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    redirects: Vec<Redirect>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    custom_games: Vec<CustomGame>,
    cloud: CloudSection,
}

#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    staging: PathBuf,
    roots: Vec<Root>,
    portable_redirects: Vec<Redirect>,
    manual_redirects: Vec<Redirect>,
    custom_games: Vec<CustomGame>,
}

impl ConfigBuilder {
    /// `staging` is where backups are written before upload, and read from on restore.
    pub fn new(staging: impl Into<PathBuf>) -> Self {
        Self {
            staging: staging.into(),
            roots: Vec::new(),
            portable_redirects: Vec::new(),
            manual_redirects: Vec::new(),
            custom_games: Vec::new(),
        }
    }

    pub fn root(mut self, store: RootStore, path: impl Into<String>) -> Self {
        self.roots.push(Root {
            store,
            path: path.into(),
        });
        self
    }

    /// Registers this game's prefix as a place to scan, by concrete path: Ludusavi's
    /// `<game>` token expands to the title, and prefixes here are named by id.
    pub fn wine_prefix(self, prefix_dir: &Path) -> Self {
        self.root(RootStore::OtherWine, prefix_dir.to_string_lossy())
    }

    /// Stores `source` as `target`, both ways. Ludusavi stops at the first matching redirect, so
    /// the more specific folder goes first.
    pub fn portable(mut self, source: &Path, target: impl Into<String>) -> Self {
        self.portable_redirects.push(Redirect {
            kind: RedirectKind::Bidirectional,
            source: source.to_string_lossy().into_owned(),
            target: target.into(),
        });
        self
    }

    /// Restores what was stored as `source` into `target`, without touching backups.
    pub fn restore_redirect(mut self, source: &str, target: &Path) -> Self {
        self.portable_redirects.push(Redirect {
            kind: RedirectKind::Restore,
            source: source.to_string(),
            target: target.to_string_lossy().into_owned(),
        });
        self
    }

    /// Make the user's home portable. For a Windows game this is the home inside the prefix.
    pub fn portable_home(self, home: &Path) -> Self {
        self.portable(home, HOME_TARGET)
    }

    /// The machine's own home, for a Windows game whose portable home is the prefix's. Added after
    /// [`Self::portable_home`], since Ludusavi stops at the first matching redirect.
    pub fn portable_host_home(self, home: &Path) -> Self {
        self.portable(home, HOST_HOME_TARGET)
    }

    pub fn portable_install_dir(self, install_dir: &Path) -> Self {
        self.portable(install_dir, INSTALL_TARGET)
    }

    /// Hand-entered path mappings, ahead of the portable ones: an explicit answer is the escape
    /// hatch for what Ludusavi cannot work out.
    pub fn manual_redirects(mut self, redirects: impl IntoIterator<Item = Redirect>) -> Self {
        self.manual_redirects.extend(redirects);
        self
    }

    /// Adds to a game's custom entry, creating it if there is none. One entry per name,
    /// since Ludusavi does not promise to merge two that share one.
    fn amend_custom_game(mut self, name: String, edit: impl FnOnce(&mut CustomGame)) -> Self {
        let index = match self.custom_games.iter().position(|g| g.name == name) {
            Some(index) => index,
            None => {
                self.custom_games.push(CustomGame {
                    name,
                    files: Vec::new(),
                    registry: Vec::new(),
                    // Never `override`: a game the database already covers must keep
                    // everything it knew and simply gain what is added here.
                    integration: CustomGameIntegration::Extend,
                });
                self.custom_games.len() - 1
            }
        };
        edit(&mut self.custom_games[index]);
        self
    }

    /// Hand-named save folders, the escape hatch for a game the database does not list:
    /// naming one makes the game findable and says what to copy.
    pub fn custom_save_paths(
        self,
        game_name: impl Into<String>,
        paths: impl IntoIterator<Item = String>,
    ) -> Self {
        let paths: Vec<String> = paths.into_iter().filter(|p| !p.trim().is_empty()).collect();
        if paths.is_empty() {
            return self;
        }
        self.amend_custom_game(game_name.into(), |game| game.files.extend(paths))
    }

    /// The redirects that reach the config file, in priority order. The portable ones are never
    /// dropped: without them a backup records this machine's own paths, which nowhere else has.
    fn effective_redirects(&self) -> Vec<Redirect> {
        let mut redirects = self.manual_redirects.clone();
        redirects.extend(self.portable_redirects.clone());
        redirects
    }

    fn build(&self) -> ConfigFile {
        let staging = self.staging.to_string_lossy().into_owned();
        ConfigFile {
            roots: self.roots.clone(),
            release: ReleaseSection { check: false },
            backup: BackupSection {
                path: staging.clone(),
                format: FormatSection {
                    chosen: "zip".into(),
                    zip: ZipSection {
                        compression: "zstd".into(),
                    },
                },
                // The server keeps version history, so only the newest backup is held
                // locally, anything more would inflate every upload.
                retention: RetentionSection {
                    full: 1,
                    differential: 0,
                },
            },
            restore: RestoreSection { path: staging },
            redirects: self.effective_redirects(),
            custom_games: self.custom_games.clone(),
            cloud: CloudSection { synchronize: false },
        }
    }

    pub fn to_yaml(&self) -> SaveResult<String> {
        Ok(serde_yaml_ng::to_string(&self.build())?)
    }

    pub async fn write(&self, config_dir: &Path) -> SaveResult<PathBuf> {
        tokio::fs::create_dir_all(config_dir).await?;
        let path = config_dir.join("config.yaml");
        tokio::fs::write(&path, self.to_yaml()?).await?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> serde_yaml_ng::Value {
        serde_yaml_ng::from_str(yaml).expect("valid yaml")
    }

    #[test]
    fn hand_set_folders_become_a_custom_game() {
        let yaml = parse(
            &ConfigBuilder::new("/staging")
                .custom_save_paths("Example Game", ["/saves/one".to_string(), "  ".to_string()])
                .to_yaml()
                .unwrap(),
        );

        let games = yaml["customGames"].as_sequence().expect("custom games");
        assert_eq!(1, games.len());
        assert_eq!("Example Game", games[0]["name"].as_str().unwrap());
        // Blank rows are a half-finished edit, not an instruction to scan the whole disk.
        assert_eq!(1, games[0]["files"].as_sequence().unwrap().len());
        // Never `override`: a game the database covers must keep what it already knew.
        assert_eq!("extend", games[0]["integration"].as_str().unwrap());
    }

    #[test]
    fn no_folders_means_no_custom_game() {
        let yaml = parse(
            &ConfigBuilder::new("/staging")
                .custom_save_paths("Example Game", Vec::<String>::new())
                .to_yaml()
                .unwrap(),
        );
        assert!(yaml.get("customGames").is_none());
    }

    #[test]
    fn the_self_update_check_is_turned_off() {
        // Left on, Ludusavi asks GitHub about itself on every run, with no timeout.
        let yaml = parse(&ConfigBuilder::new("/staging").to_yaml().unwrap());
        assert_eq!(Some(false), yaml["release"]["check"].as_bool());
    }

    #[test]
    fn writes_backup_and_restore_to_the_same_staging_dir() {
        let yaml = ConfigBuilder::new("/stage").to_yaml().unwrap();
        let v = parse(&yaml);
        assert_eq!(v["backup"]["path"].as_str(), Some("/stage"));
        assert_eq!(v["restore"]["path"].as_str(), Some("/stage"));
    }

    #[test]
    fn retains_only_the_newest_backup_locally() {
        let yaml = ConfigBuilder::new("/stage").to_yaml().unwrap();
        let v = parse(&yaml);
        assert_eq!(v["backup"]["retention"]["full"].as_u64(), Some(1));
        assert_eq!(v["backup"]["retention"]["differential"].as_u64(), Some(0));
    }

    #[test]
    fn disables_ludusavi_cloud_sync() {
        // The Gameyfin server is the sync target; rclone would be a second, conflicting one.
        let yaml = ConfigBuilder::new("/stage").to_yaml().unwrap();
        assert_eq!(parse(&yaml)["cloud"]["synchronize"].as_bool(), Some(false));
    }

    #[test]
    fn redirects_make_home_and_install_portable() {
        let yaml = ConfigBuilder::new("/stage")
            .portable_home(Path::new("/home/alice"))
            .portable_install_dir(Path::new("/games/Celeste"))
            .to_yaml()
            .unwrap();
        let v = parse(&yaml);
        let redirects = v["redirects"].as_sequence().unwrap();
        assert_eq!(redirects.len(), 2);

        assert_eq!(redirects[0]["kind"].as_str(), Some("bidirectional"));
        assert_eq!(redirects[0]["source"].as_str(), Some("/home/alice"));
        assert_eq!(redirects[0]["target"].as_str(), Some("/gameyfin/home"));
        assert_eq!(redirects[1]["source"].as_str(), Some("/games/Celeste"));
        assert_eq!(redirects[1]["target"].as_str(), Some("/gameyfin/install"));
    }

    #[test]
    fn the_prefix_home_outranks_the_machines_own() {
        // The prefix's home travels; the Linux home gets its own target, or a save could restore
        // into whichever of the two Ludusavi matched first.
        let yaml = ConfigBuilder::new("/stage")
            .portable_home(Path::new(
                "/games/Gameyfin/Prefixes/7/pfx/drive_c/users/steamuser",
            ))
            .portable_host_home(Path::new("/home/alice"))
            .to_yaml()
            .unwrap();
        let v = parse(&yaml);
        let redirects = v["redirects"].as_sequence().unwrap();

        assert_eq!(
            redirects[0]["source"].as_str(),
            Some("/games/Gameyfin/Prefixes/7/pfx/drive_c/users/steamuser")
        );
        assert_eq!(redirects[0]["target"].as_str(), Some("/gameyfin/home"));
        assert_eq!(redirects[1]["source"].as_str(), Some("/home/alice"));
        assert_eq!(redirects[1]["target"].as_str(), Some("/gameyfin/host-home"));
    }

    #[test]
    fn the_wine_prefix_root_is_a_real_path() {
        // Not "<prefixes>/<game>": that token expands to the name, while prefixes are named by id.
        let yaml = ConfigBuilder::new("/stage")
            .wine_prefix(Path::new("/data/prefixes/1234"))
            .to_yaml()
            .unwrap();
        let v = parse(&yaml);
        let root = &v["roots"][0];
        assert_eq!(root["store"].as_str(), Some("otherWine"));
        assert_eq!(root["path"].as_str(), Some("/data/prefixes/1234"));
        assert!(
            !yaml.contains("<game>"),
            "no unexpanded placeholder should reach the config"
        );
    }

    #[test]
    fn steam_root_is_serialised_with_the_name_ludusavi_expects() {
        let yaml = ConfigBuilder::new("/s")
            .root(RootStore::Steam, "/home/u/.steam/steam")
            .to_yaml()
            .unwrap();
        assert_eq!(parse(&yaml)["roots"][0]["store"].as_str(), Some("steam"));
    }

    #[test]
    fn empty_optional_sections_are_omitted() {
        let yaml = ConfigBuilder::new("/s").to_yaml().unwrap();
        assert!(!yaml.contains("redirects"));
        assert!(!yaml.contains("customGames"));
    }

    #[tokio::test]
    async fn write_creates_the_config_directory() {
        let dir = std::env::temp_dir().join(format!("gameyfin-cfg-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;

        let path = ConfigBuilder::new("/stage").write(&dir).await.unwrap();
        assert!(path.exists());
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("/stage"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}

#[cfg(test)]
mod redirect_tests {
    use super::*;

    fn yaml(builder: &ConfigBuilder) -> serde_json::Value {
        serde_yaml_ng::from_str(&builder.to_yaml().unwrap()).unwrap()
    }

    fn base() -> ConfigBuilder {
        ConfigBuilder::new("/staging")
            .portable_home(Path::new("/home/alice"))
            .portable_install_dir(Path::new("/games/Celeste"))
    }

    #[test]
    fn the_synthetic_redirects_are_always_written() {
        let config = yaml(&base());
        let redirects = config["redirects"].as_array().unwrap();
        assert_eq!(2, redirects.len());
        assert_eq!("/gameyfin/home", redirects[0]["target"].as_str().unwrap());
        // Ludusavi 0.31 has no such setting, and an unknown key only hides that.
        assert!(config.get("scan").is_none());
    }

    #[test]
    fn a_restore_redirect_leaves_backups_alone() {
        let config =
            yaml(&base().restore_redirect(HOST_HOME_TARGET, Path::new("/saves/unplaced/7")));
        let last = config["redirects"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!("restore", last["kind"].as_str().unwrap());
        assert_eq!("/gameyfin/host-home", last["source"].as_str().unwrap());
        assert_eq!("/saves/unplaced/7", last["target"].as_str().unwrap());
    }

    #[test]
    fn documents_travels_inside_the_home() {
        assert!(Path::new(DOCUMENTS_TARGET).starts_with(HOME_TARGET));
    }

    #[test]
    fn a_manual_redirect_outranks_the_portable_ones() {
        // Ludusavi applies redirects top to bottom, so ordering is the priority.
        let config = yaml(&base().manual_redirects([Redirect {
            kind: RedirectKind::Bidirectional,
            source: "C:/Saves".into(),
            target: "/home/alice/saves".into(),
        }]));

        let redirects = config["redirects"].as_array().unwrap();
        assert_eq!("C:/Saves", redirects[0]["source"].as_str().unwrap());
        assert_eq!("/home/alice", redirects[1]["source"].as_str().unwrap());
    }
}
