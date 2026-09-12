//! Resolving a game's umu id, which keys umu-launcher's per-title Proton fixes. A miss falls
//! back to the generic id and never fails a launch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_API_URL: &str = "https://umu.openwinecomponents.org/umu_api.php";

/// Identifier `umu-run` falls back to when a game has no known per-title fixes.
pub const DEFAULT_ID: &str = "umu-default";

/// How long a cached copy lasts. A day-stale workaround list beats blocking every launch
/// on a network request.
pub const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub umu_id: String,
    #[serde(default)]
    pub store: String,
    /// The store's own identifier: a Steam AppID, a GOG slug, and so on.
    #[serde(default)]
    pub codename: String,
}

/// The database, indexed for the two lookups that matter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Database {
    entries: Vec<Entry>,
    /// `store:codename` to a position in `entries`.
    by_codename: HashMap<String, usize>,
    /// Normalised title to a position in `entries`.
    by_title: HashMap<String, usize>,
}

impl Database {
    pub fn new(entries: Vec<Entry>) -> Self {
        let mut by_codename = HashMap::new();
        let mut by_title = HashMap::new();

        for (index, entry) in entries.iter().enumerate() {
            if entry.umu_id.is_empty() {
                continue;
            }
            if !entry.store.is_empty() && !entry.codename.is_empty() {
                by_codename
                    .entry(codename_key(&entry.store, &entry.codename))
                    // First wins: the database lists some titles more than once, and a
                    // later duplicate should not displace the row already matched.
                    .or_insert(index);
            }
            let normalized = normalize(&entry.title);
            if !normalized.is_empty() {
                by_title.entry(normalized).or_insert(index);
            }
        }

        Self {
            entries,
            by_codename,
            by_title,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn by_steam_app_id(&self, app_id: u32) -> Option<&Entry> {
        self.by_codename
            .get(&codename_key("steam", &app_id.to_string()))
            .and_then(|&index| self.entries.get(index))
    }

    /// Look a game up by title, ignoring case, punctuation and numeral style.
    pub fn by_title(&self, title: &str) -> Option<&Entry> {
        let key = normalize(title);
        if key.is_empty() {
            return None;
        }
        self.by_title
            .get(&key)
            .and_then(|&index| self.entries.get(index))
    }

    /// The id to pass as `GAMEID`. Never fails: a game with no entry gets the generic one,
    /// which is the behaviour it would have had anyway.
    pub fn resolve(&self, title: &str, steam_app_id: Option<u32>) -> String {
        let found = steam_app_id
            .and_then(|id| self.by_steam_app_id(id))
            .or_else(|| self.by_title(title));

        match found {
            Some(entry) => {
                tracing::info!(
                    title,
                    umu_id = %entry.umu_id,
                    matched = %entry.title,
                    "resolved a umu id, per-title Proton fixes will apply"
                );
                entry.umu_id.clone()
            }
            None => {
                tracing::debug!(title, "no umu entry; using the generic id");
                DEFAULT_ID.to_string()
            }
        }
    }
}

fn codename_key(store: &str, codename: &str) -> String {
    format!("{}:{}", store.to_lowercase(), codename.to_lowercase())
}

/// Reduces a title to a form two spellings agree on: roman numerals become arabic, anything
/// but letters and digits goes, and the rest is lowercased.
pub fn normalize(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| match roman_to_arabic(word) {
            Some(number) => number.to_string(),
            None => word.to_lowercase(),
        })
        .collect();

    words
        .concat()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// A whole word that is a roman numeral from I to X. Whole words only: a substring pass
/// would make "Civilization" into "Civ1lization".
fn roman_to_arabic(word: &str) -> Option<u8> {
    match word.to_ascii_uppercase().as_str() {
        "I" => Some(1),
        "II" => Some(2),
        "III" => Some(3),
        "IV" => Some(4),
        "V" => Some(5),
        "VI" => Some(6),
        "VII" => Some(7),
        "VIII" => Some(8),
        "IX" => Some(9),
        "X" => Some(10),
        _ => None,
    }
}

pub fn cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join("umu-database.json")
}

pub struct Cached {
    pub database: Database,
    pub stale: bool,
}

pub fn load_cache(config_dir: &Path) -> Option<Cached> {
    let path = cache_path(config_dir);
    let bytes = std::fs::read(&path).ok()?;
    let database: Database = serde_json::from_slice(&bytes)
        .map_err(|e| tracing::warn!("ignoring an unreadable umu cache at {path:?}: {e}"))
        .ok()?;

    let stale = cache_age(config_dir).is_none_or(|age| age > CACHE_TTL);

    Some(Cached { database, stale })
}

/// How long ago the cached copy was written. `None` when there is none, or its time is unreadable.
pub fn cache_age(config_dir: &Path) -> Option<std::time::Duration> {
    std::fs::metadata(cache_path(config_dir))
        .and_then(|m| m.modified())
        .ok()?
        .elapsed()
        .ok()
}

pub fn save_cache(config_dir: &Path, database: &Database) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let json = serde_json::to_vec(database)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(cache_path(config_dir), json)
}

/// Fetches the whole database. The URL is a parameter so a test can point it elsewhere.
pub async fn fetch(http: &reqwest::Client, api_url: &str) -> crate::CoreResult<Database> {
    let response = http
        .get(api_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| crate::CoreError::Other(format!("could not reach the umu database: {e}")))?;

    if !response.status().is_success() {
        return Err(crate::CoreError::Other(format!(
            "the umu database answered HTTP {}",
            response.status()
        )));
    }

    let entries: Vec<Entry> = response
        .json()
        .await
        .map_err(|e| crate::CoreError::Other(format!("could not read the umu database: {e}")))?;

    tracing::info!(entries = entries.len(), "fetched the umu database");
    Ok(Database::new(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Database {
        Database::new(vec![
            Entry {
                title: "Baldur's Gate II: Shadows of Amn".into(),
                umu_id: "umu-257350".into(),
                store: "gog".into(),
                codename: "baldurs_gate_2".into(),
            },
            Entry {
                title: "Celeste".into(),
                umu_id: "umu-504230".into(),
                store: "steam".into(),
                codename: "504230".into(),
            },
            Entry {
                title: "Deus Ex".into(),
                umu_id: "umu-6910".into(),
                store: "steam".into(),
                codename: "6910".into(),
            },
        ])
    }

    #[test]
    fn normalising_folds_punctuation_case_spacing_and_numerals() {
        for (title, expected) in [
            ("Baldur's Gate", "baldursgate"),
            ("baldurs   gate", "baldursgate"),
            ("BALDURS-GATE!", "baldursgate"),
            ("Baldur's Gate II", "baldursgate2"),
            ("baldurs gate 2", "baldursgate2"),
            ("Final Fantasy VII", "finalfantasy7"),
            ("Portal X", "portal10"),
            // Only whole words are numerals: substring replacement would mangle these.
            ("Civilization", "civilization"),
            ("Vice City", "vicecity"),
            ("Ixion", "ixion"),
            ("", ""),
            ("!!!", ""),
        ] {
            assert_eq!(normalize(title), expected, "{title:?}");
        }
    }

    #[test]
    fn a_steam_app_id_is_an_exact_match() {
        let db = sample();
        assert_eq!(db.by_steam_app_id(504230).unwrap().umu_id, "umu-504230");
        assert!(db.by_steam_app_id(999999).is_none());
    }

    #[test]
    fn a_title_matches_across_spellings() {
        let db = sample();
        assert_eq!(
            db.by_title("baldurs gate 2 shadows of amn").unwrap().umu_id,
            "umu-257350"
        );
        assert!(db.by_title("Something Else").is_none());
    }

    #[test]
    fn the_steam_id_wins_over_the_title() {
        // The id is exact; the title is a heuristic, and where they disagree the exact
        // one has to win or the fixes applied are for the wrong game.
        let db = Database::new(vec![
            Entry {
                title: "Celeste".into(),
                umu_id: "umu-by-title".into(),
                store: "gog".into(),
                codename: "celeste".into(),
            },
            Entry {
                title: "Something Else Entirely".into(),
                umu_id: "umu-by-id".into(),
                store: "steam".into(),
                codename: "504230".into(),
            },
        ]);
        assert_eq!(db.resolve("Celeste", Some(504230)), "umu-by-id");
        assert_eq!(db.resolve("Celeste", None), "umu-by-title");
    }

    #[test]
    fn an_unknown_game_gets_the_generic_id() {
        // The fallback has to be exactly the previous behaviour: never a failed launch.
        let db = sample();
        assert_eq!(db.resolve("A Game Nobody Indexed", None), DEFAULT_ID);
        assert_eq!(db.resolve("A Game Nobody Indexed", Some(1)), DEFAULT_ID);
        assert_eq!(
            Database::default().resolve("Celeste", Some(504230)),
            DEFAULT_ID
        );
    }

    #[test]
    fn rows_without_a_umu_id_are_not_indexed() {
        // Matching one would set GAMEID to an empty string, which umu reads as no id at
        // all but with a warning, and is strictly worse than the documented default.
        let db = Database::new(vec![Entry {
            title: "Broken Row".into(),
            umu_id: String::new(),
            store: "steam".into(),
            codename: "1".into(),
        }]);
        assert!(db.by_title("Broken Row").is_none());
        assert_eq!(db.resolve("Broken Row", Some(1)), DEFAULT_ID);
    }

    #[test]
    fn a_duplicate_row_does_not_displace_the_first() {
        let db = Database::new(vec![
            Entry {
                title: "Doom".into(),
                umu_id: "umu-first".into(),
                store: "steam".into(),
                codename: "1".into(),
            },
            Entry {
                title: "Doom".into(),
                umu_id: "umu-second".into(),
                store: "steam".into(),
                codename: "1".into(),
            },
        ]);
        assert_eq!(db.resolve("Doom", Some(1)), "umu-first");
    }

    #[test]
    fn the_cache_round_trips_and_reports_freshness() {
        let dir = std::env::temp_dir().join(format!("gameyfin-umu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        assert!(load_cache(&dir).is_none());

        save_cache(&dir, &sample()).unwrap();
        let cached = load_cache(&dir).expect("just written");
        assert_eq!(cached.database.len(), 3);
        assert!(!cached.stale, "a file written now is fresh");
        assert_eq!(
            cached.database.resolve("Celeste", Some(504230)),
            "umu-504230"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_cache_is_ignored_rather_than_fatal() {
        let dir = std::env::temp_dir().join(format!("gameyfin-umu-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(cache_path(&dir), b"{ not json").unwrap();

        assert!(load_cache(&dir).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn fetching_indexes_what_the_server_returns() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/umu_api.php")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[{"title":"Celeste","umu_id":"umu-504230","store":"steam","codename":"504230"}]"#,
            )
            .create_async()
            .await;

        let http = reqwest::Client::new();
        let db = fetch(&http, &format!("{}/umu_api.php", server.url()))
            .await
            .expect("the server answered");

        assert_eq!(db.len(), 1);
        assert_eq!(db.resolve("Celeste", Some(504230)), "umu-504230");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn a_failing_server_is_an_error_not_a_panic() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/umu_api.php")
            .with_status(503)
            .create_async()
            .await;

        let http = reqwest::Client::new();
        let result = fetch(&http, &format!("{}/umu_api.php", server.url())).await;
        assert!(result.is_err());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn rows_with_unexpected_fields_still_load() {
        // The database is someone else's, and it gains columns; an added field must not
        // stop every fix from applying.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/umu_api.php")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[{"title":"Celeste","umu_id":"umu-504230","store":"steam",
                     "codename":"504230","notes":"something new","acquired":1}]"#,
            )
            .create_async()
            .await;

        let http = reqwest::Client::new();
        let db = fetch(&http, &format!("{}/umu_api.php", server.url()))
            .await
            .expect("unknown fields are ignored");
        assert_eq!(db.resolve("Celeste", None), "umu-504230");
        mock.assert_async().await;
    }
}
