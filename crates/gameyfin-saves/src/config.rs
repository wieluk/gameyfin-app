//! Ludusavi's `config.yaml`, written into a private config directory. Redirects map the
//! account name and install directory onto targets identical on every machine, so a backup
//! restores under a different username. Windows to native Linux is not translated at all,
//! which the caller warns about rather than restoring across.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::SaveResult;

/// Synthetic redirect targets. Arbitrary but fixed, they only have to be identical
/// across machines, and distinctive enough never to collide with a real path.
const HOME_TARGET: &str = "/gameyfin/home";
const INSTALL_TARGET: &str = "/gameyfin/install";

/// A location Ludusavi should scan, and how to interpret its layout.
#[derive(Debug, Clone, Serialize)]
pub struct Root {
    pub store: RootStore,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RootStore {
    /// A Steam library. Ludusavi finds Proton saves under
    /// `steamapps/compatdata/<AppID>/pfx` automatically, and captures Proton's registry
    /// files for games that need them.
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wine_prefix: Vec<String>,
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
struct ScanSection {
    /// Ludusavi's best-effort Windows/Wine path translation. Restore-only, files only.
    redirect_wine: bool,
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
    /// Ludusavi asks GitHub whether it is out of date on every single run. Gameyfin
    /// manages its version itself, so that is a network call per backup with nothing to
    /// show for it.
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
    scan: ScanSection,
    cloud: CloudSection,
}

/// How a backup from elsewhere maps onto this machine. Mutually exclusive by necessity:
/// Ludusavi stops at the first redirect that changes a path, so the portable home one
/// would disable its own Wine translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestoreStrategy {
    /// Same operating system, possibly a different account and install directory.
    /// Synthetic redirects make the paths identical everywhere.
    #[default]
    Portable,
    /// Windows to or from a Wine/Proton prefix. Hands the job to Ludusavi's own
    /// translation, which needs a preferred prefix and does not carry registry values.
    CrossOs,
}

/// Builds the Ludusavi configuration for this machine.
#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    staging: PathBuf,
    roots: Vec<Root>,
    portable_redirects: Vec<Redirect>,
    manual_redirects: Vec<Redirect>,
    custom_games: Vec<CustomGame>,
    strategy: RestoreStrategy,
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
            strategy: RestoreStrategy::default(),
        }
    }

    /// Selecting [`RestoreStrategy::CrossOs`] drops the portable redirects, which would
    /// otherwise suppress Ludusavi's Wine translation.
    pub fn strategy(mut self, strategy: RestoreStrategy) -> Self {
        self.strategy = strategy;
        self
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

    /// Make the user's home directory portable across machines and accounts.
    pub fn portable_home(mut self, home: &Path) -> Self {
        self.portable_redirects.push(Redirect {
            kind: RedirectKind::Bidirectional,
            source: home.to_string_lossy().into_owned(),
            target: HOME_TARGET.to_string(),
        });
        self
    }

    /// Make a game's install directory portable across machines.
    pub fn portable_install_dir(mut self, install_dir: &Path) -> Self {
        self.portable_redirects.push(Redirect {
            kind: RedirectKind::Bidirectional,
            source: install_dir.to_string_lossy().into_owned(),
            target: INSTALL_TARGET.to_string(),
        });
        self
    }

    /// Hand-entered path mappings, which survive both strategies: an explicit answer is the
    /// escape hatch for what Ludusavi cannot work out.
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
                    wine_prefix: Vec::new(),
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

    /// Names the prefix to translate against. [`RestoreStrategy::CrossOs`] does nothing
    /// without one, and Ludusavi only takes it from a custom game entry.
    pub fn preferred_wine_prefix(self, game_name: impl Into<String>, prefix: &Path) -> Self {
        let prefix = prefix.to_string_lossy().into_owned();
        self.amend_custom_game(game_name.into(), |game| game.wine_prefix.push(prefix))
    }

    /// The redirects that actually reach the config file, in priority order.
    fn effective_redirects(&self) -> Vec<Redirect> {
        let mut redirects = self.manual_redirects.clone();
        if self.strategy == RestoreStrategy::Portable {
            redirects.extend(self.portable_redirects.clone());
        }
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
            scan: ScanSection {
                redirect_wine: self.strategy == RestoreStrategy::CrossOs,
            },
            cloud: CloudSection { synchronize: false },
        }
    }

    pub fn to_yaml(&self) -> SaveResult<String> {
        Ok(serde_yaml_ng::to_string(&self.build())?)
    }

    /// Write `config.yaml` into `config_dir`, creating it if needed.
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
    fn a_prefix_and_hand_set_folders_share_one_entry() {
        // Two custom games with the same name is not something Ludusavi promises to merge.
        let yaml = parse(
            &ConfigBuilder::new("/staging")
                .custom_save_paths("Example Game", ["/saves/one".to_string()])
                .preferred_wine_prefix("Example Game", std::path::Path::new("/prefixes/1"))
                .to_yaml()
                .unwrap(),
        );

        let games = yaml["customGames"].as_sequence().expect("custom games");
        assert_eq!(1, games.len());
        assert_eq!(1, games[0]["files"].as_sequence().unwrap().len());
        assert_eq!(1, games[0]["winePrefix"].as_sequence().unwrap().len());
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
    fn the_wine_prefix_root_is_a_real_path() {
        // Not "<prefixes>/<game>": that token expands to the game's name, while prefixes
        // are created under the game's id, so the root matched nothing and a Windows game
        // on Linux backed up zero files.
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
mod strategy_tests {
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
    fn the_portable_strategy_emits_the_synthetic_redirects_and_leaves_wine_translation_off() {
        let config = yaml(&base());

        let redirects = config["redirects"].as_array().unwrap();
        assert_eq!(2, redirects.len());
        assert_eq!("/gameyfin/home", redirects[0]["target"].as_str().unwrap());
        assert!(!config["scan"]["redirectWine"].as_bool().unwrap());
    }

    #[test]
    fn the_cross_os_strategy_drops_the_portable_redirects() {
        // A user redirect that fires first stops Ludusavi before its own Wine translation,
        // so leaving these in would silently disable the feature we just turned on.
        let config = yaml(&base().strategy(RestoreStrategy::CrossOs));

        assert!(config.get("redirects").is_none());
        assert!(config["scan"]["redirectWine"].as_bool().unwrap());
    }

    #[test]
    fn manual_redirects_survive_both_strategies() {
        let manual = Redirect {
            kind: RedirectKind::Bidirectional,
            source: "C:/Saves".into(),
            target: "/home/alice/saves".into(),
        };

        for strategy in [RestoreStrategy::Portable, RestoreStrategy::CrossOs] {
            let config = yaml(&base().strategy(strategy).manual_redirects([manual.clone()]));
            let redirects = config["redirects"].as_array().unwrap();
            assert_eq!("C:/Saves", redirects[0]["source"].as_str().unwrap());
        }
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

    #[test]
    fn a_preferred_prefix_is_written_as_a_custom_game() {
        let config = yaml(
            &base()
                .strategy(RestoreStrategy::CrossOs)
                .preferred_wine_prefix("Celeste", Path::new("/prefixes/12")),
        );

        let games = config["customGames"].as_array().unwrap();
        assert_eq!("Celeste", games[0]["name"].as_str().unwrap());
        assert_eq!("/prefixes/12", games[0]["winePrefix"][0].as_str().unwrap());
        assert_eq!("extend", games[0]["integration"].as_str().unwrap());
    }
}
