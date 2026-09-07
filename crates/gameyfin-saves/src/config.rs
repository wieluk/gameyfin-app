//! Generating Ludusavi's `config.yaml`.
//!
//! Written into a private directory (passed as `--config`) so the user's own
//! `~/.config/ludusavi` is never touched.
//!
//! Save paths embed the account name and the install directory, so a backup taken as
//! `C:/Users/alice/...` will not restore for `bob`. Bidirectional redirects map both onto
//! synthetic targets that are identical on every machine.
//!
//! Ludusavi does not translate save locations across operating systems. A Windows game
//! under Proton keeps Windows-shaped paths inside the prefix, so Windows to Proton
//! round-trips work; a Windows game and a native Linux build of the same game do not, and
//! the caller is expected to warn rather than restore across that boundary.

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
struct CloudSection {
    /// Ludusavi's own rclone sync is off: the Gameyfin server is the sync target.
    synchronize: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    roots: Vec<Root>,
    backup: BackupSection,
    restore: RestoreSection,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    redirects: Vec<Redirect>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    custom_games: Vec<CustomGame>,
    cloud: CloudSection,
}

/// Builds the Ludusavi configuration for this machine.
#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    staging: PathBuf,
    roots: Vec<Root>,
    redirects: Vec<Redirect>,
    custom_games: Vec<CustomGame>,
}

impl ConfigBuilder {
    /// `staging` is where backups are written before upload, and read from on restore.
    pub fn new(staging: impl Into<PathBuf>) -> Self {
        Self {
            staging: staging.into(),
            roots: Vec::new(),
            redirects: Vec::new(),
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

    /// Register the app's own per-game Wine prefixes.
    ///
    /// The `<game>` placeholder lets a single root cover every prefix, instead of
    /// enumerating them or rescanning all of them for every game.
    pub fn wine_prefix_collection(self, prefixes_dir: &Path) -> Self {
        // Appended as text, not with `Path::join`: `<game>` is a token Ludusavi expands
        // rather than a real path component, and joining would write a backslash on
        // Windows into a path that is always a Linux one, since Wine prefixes only exist
        // there.
        let root = prefixes_dir.to_string_lossy();
        self.root(
            RootStore::OtherWine,
            format!("{}/<game>", root.trim_end_matches('/')),
        )
    }

    /// Make the user's home directory portable across machines and accounts.
    pub fn portable_home(mut self, home: &Path) -> Self {
        self.redirects.push(Redirect {
            kind: RedirectKind::Bidirectional,
            source: home.to_string_lossy().into_owned(),
            target: HOME_TARGET.to_string(),
        });
        self
    }

    /// Make a game's install directory portable across machines.
    pub fn portable_install_dir(mut self, install_dir: &Path) -> Self {
        self.redirects.push(Redirect {
            kind: RedirectKind::Bidirectional,
            source: install_dir.to_string_lossy().into_owned(),
            target: INSTALL_TARGET.to_string(),
        });
        self
    }

    pub fn custom_game(mut self, game: CustomGame) -> Self {
        self.custom_games.push(game);
        self
    }

    fn build(&self) -> ConfigFile {
        let staging = self.staging.to_string_lossy().into_owned();
        ConfigFile {
            roots: self.roots.clone(),
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
            redirects: self.redirects.clone(),
            custom_games: self.custom_games.clone(),
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
    fn wine_prefix_root_uses_the_game_placeholder() {
        // One root covers every per-game prefix, rather than scanning all of them.
        let yaml = ConfigBuilder::new("/stage")
            .wine_prefix_collection(Path::new("/data/prefixes"))
            .to_yaml()
            .unwrap();
        let v = parse(&yaml);
        let root = &v["roots"][0];
        assert_eq!(root["store"].as_str(), Some("otherWine"));
        assert_eq!(root["path"].as_str(), Some("/data/prefixes/<game>"));
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

    #[test]
    fn custom_game_serialises_with_camel_case_keys() {
        let yaml = ConfigBuilder::new("/s")
            .custom_game(CustomGame {
                name: "Homebrew".into(),
                files: vec!["<home>/.homebrew/*.sav".into()],
                registry: vec![],
                wine_prefix: vec![],
                integration: CustomGameIntegration::Extend,
            })
            .to_yaml()
            .unwrap();
        let v = parse(&yaml);
        let game = &v["customGames"][0];
        assert_eq!(game["name"].as_str(), Some("Homebrew"));
        assert_eq!(game["integration"].as_str(), Some("extend"));
        // Empty lists are skipped rather than written as `[]`.
        assert!(game.get("registry").is_none());
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
