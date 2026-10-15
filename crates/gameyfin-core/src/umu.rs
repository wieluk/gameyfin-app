//! Resolving a game's umu id, which keys umu-launcher's per-title Proton fixes. A miss falls
//! back to the generic id and never fails a launch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_API_URL: &str = "https://umu.openwinecomponents.org/umu_api.php";

/// Identifier `umu-run` falls back to when a game has no known per-title fixes.
pub const DEFAULT_ID: &str = "umu-default";

/// umu's value for a game that did not come from a known storefront.
pub const NO_STORE: &str = "none";

/// The stores protonfixes files fixes under, with [`NO_STORE`] for its own `umu` folder.
pub const STORES: [&str; 11] = [
    NO_STORE,
    "amazon",
    "battlenet",
    "ea",
    "egs",
    "gog",
    "humble",
    "itchio",
    "steam",
    "ubisoft",
    "zoomplatform",
];

/// How long a cached copy lasts. A day-stale workaround list beats blocking every launch
/// on a network request.
pub const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// What umu reads as `GAMEID` and `STORE`. Protonfixes files a fix under its store's folder,
/// so the id alone does not find it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameFix {
    pub id: String,
    pub store: String,
}

impl Default for GameFix {
    fn default() -> Self {
        Self {
            id: DEFAULT_ID.to_string(),
            store: NO_STORE.to_string(),
        }
    }
}

impl GameFix {
    /// A fix typed by hand. The id becomes a Python module name in protonfixes, so only
    /// plain characters pass.
    pub fn checked(id: &str, store: &str) -> Result<Self, String> {
        let id = id.trim();
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_');
        if id.is_empty() || id.len() > 64 || !id.chars().all(plain) {
            return Err(format!(
                "{id:?} is not a fix id. Use one from the list, or a Steam AppID."
            ));
        }
        let store = store.trim().to_ascii_lowercase();
        if !STORES.contains(&store.as_str()) {
            return Err(format!("{store:?} is not a store umu knows."));
        }
        Ok(Self {
            id: id.to_string(),
            store,
        })
    }

    /// The fix filed under a Steam AppID, which protonfixes finds whatever the store says.
    pub fn steam(app_id: u32) -> Self {
        Self {
            id: app_id.to_string(),
            store: NO_STORE.to_string(),
        }
    }

    /// Whether this is the generic id, which names no fix.
    pub fn is_default(&self) -> bool {
        self.id == DEFAULT_ID
    }

    /// The protonfixes folder this pair is looked up in. Mirrors its `get_module_name`: a
    /// numeric id is a Steam one, and no store means the `umu` folder.
    fn folder(&self) -> &str {
        if self.id.chars().all(|c| c.is_ascii_digit()) {
            "steam"
        } else if self.store == NO_STORE || !STORES.contains(&self.store.as_str()) {
            "umu"
        } else {
            &self.store
        }
    }

    /// Whether a Proton build ships a fix for this pair.
    pub fn shipped_in(&self, proton: &Path) -> bool {
        proton
            .join("protonfixes")
            .join(format!("gamefixes-{}", self.folder()))
            .join(format!("{}.py", self.id))
            .is_file()
    }
}

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

impl Entry {
    pub fn fix(&self) -> GameFix {
        GameFix {
            id: self.umu_id.clone(),
            store: if self.store.is_empty() {
                NO_STORE.to_string()
            } else {
                self.store.to_lowercase()
            },
        }
    }

    /// The Steam AppID an id of the form `umu-<digits>` carries, which most of them do.
    fn steam_app_id(&self) -> Option<u32> {
        self.umu_id.strip_prefix("umu-")?.parse().ok()
    }
}

/// The database, indexed for the two lookups that matter. A game has one row per store, so
/// each key keeps every row, in the order the database lists them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Database {
    entries: Vec<Entry>,
    /// Steam AppID to positions in `entries`. Rebuilt on load, never read from a cache.
    #[serde(skip)]
    by_app_id: HashMap<u32, Vec<usize>>,
    /// Normalised title to positions in `entries`.
    #[serde(skip)]
    by_title: HashMap<String, Vec<usize>>,
}

impl Database {
    pub fn new(entries: Vec<Entry>) -> Self {
        let mut by_app_id: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();

        for (index, entry) in entries.iter().enumerate() {
            if entry.umu_id.is_empty() {
                continue;
            }
            // The database has no Steam rows of its own: a Steam game is found through the
            // AppID inside another store's id.
            if let Some(app_id) = entry.steam_app_id() {
                by_app_id.entry(app_id).or_default().push(index);
            }
            let normalized = normalize(&entry.title);
            if !normalized.is_empty() {
                by_title.entry(normalized).or_default().push(index);
            }
        }

        Self {
            entries,
            by_app_id,
            by_title,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn rows(&self, positions: Option<&Vec<usize>>) -> Vec<&Entry> {
        positions
            .into_iter()
            .flatten()
            .filter_map(|&index| self.entries.get(index))
            .collect()
    }

    /// Every store's row for a Steam AppID.
    pub fn by_steam_app_id(&self, app_id: u32) -> Vec<&Entry> {
        self.rows(self.by_app_id.get(&app_id))
    }

    /// Every row with this title, ignoring case, punctuation and numeral style.
    pub fn by_title(&self, title: &str) -> Vec<&Entry> {
        let key = normalize(title);
        if key.is_empty() {
            return Vec::new();
        }
        self.rows(self.by_title.get(&key))
    }

    /// The fix to launch with, the generic one for a game with no entry. `has_fix` picks among
    /// a game's stores, since a fix is often filed under only some; failing that, a shipped fix
    /// under the bare Steam AppID, which needs no row; failing that, the first row.
    pub fn resolve(
        &self,
        title: &str,
        steam_app_id: Option<u32>,
        has_fix: impl Fn(&GameFix) -> bool,
    ) -> GameFix {
        let by_id = steam_app_id
            .map(|id| self.by_steam_app_id(id))
            .unwrap_or_default();
        let rows = if by_id.is_empty() {
            self.by_title(title)
        } else {
            by_id
        };
        let fixes: Vec<GameFix> = rows.iter().map(|row| row.fix()).collect();
        let steam_fix = steam_app_id.map(GameFix::steam);

        // Not louder: a game's options ask this each time they open.
        if let Some(index) = fixes.iter().position(&has_fix) {
            tracing::debug!(
                title,
                umu_id = %fixes[index].id,
                store = %fixes[index].store,
                matched = %rows[index].title,
                "matched a umu entry with a fix"
            );
            return fixes[index].clone();
        }
        if let Some(fix) = steam_fix.filter(&has_fix) {
            tracing::debug!(title, umu_id = %fix.id, "using the fix filed under the Steam AppID");
            return fix;
        }
        match fixes.into_iter().zip(rows).next() {
            Some((fix, row)) => {
                tracing::debug!(
                    title,
                    umu_id = %fix.id,
                    store = %fix.store,
                    matched = %row.title,
                    "matched a umu entry"
                );
                fix
            }
            None => {
                tracing::debug!(title, "no umu entry; using the generic id");
                GameFix::default()
            }
        }
    }

    /// Rows whose title or id contains `query`, closest titles first, for choosing by hand.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Entry> {
        let wanted = normalize(query);
        let typed = query.trim().to_lowercase();
        if wanted.is_empty() && typed.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u8, &Entry)> = self
            .entries
            .iter()
            .filter(|entry| !entry.umu_id.is_empty())
            .filter_map(|entry| {
                let title = normalize(&entry.title);
                let rank = if !wanted.is_empty() && title == wanted {
                    0
                } else if !wanted.is_empty() && title.starts_with(&wanted) {
                    1
                } else if !wanted.is_empty() && title.contains(&wanted) {
                    2
                } else if !typed.is_empty() && entry.umu_id.to_lowercase().contains(&typed) {
                    3
                } else {
                    return None;
                };
                Some((rank, entry))
            })
            .collect();
        found.sort_by(|(a_rank, a), (b_rank, b)| {
            a_rank
                .cmp(b_rank)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.store.cmp(&b.store))
        });
        found
            .into_iter()
            .take(limit)
            .map(|(_, entry)| entry)
            .collect()
    }
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
    let cached: Database = serde_json::from_slice(&bytes)
        .map_err(|e| tracing::warn!("ignoring an unreadable umu cache at {path:?}: {e}"))
        .ok()?;
    // Only the rows are stored, so a change in how they are matched reaches an old cache.
    let database = Database::new(cached.entries);

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

    fn row(title: &str, umu_id: &str, store: &str) -> Entry {
        Entry {
            title: title.into(),
            umu_id: umu_id.into(),
            store: store.into(),
            codename: String::new(),
        }
    }

    fn sample() -> Database {
        Database::new(vec![
            row("Baldur's Gate II: Shadows of Amn", "umu-257350", "gog"),
            row("Celeste", "umu-504230", "egs"),
            row("Deus Ex", "umu-6910", "gog"),
        ])
    }

    /// The id a game resolves to when nothing is known about which fixes a build ships.
    fn id(db: &Database, title: &str, steam_app_id: Option<u32>) -> String {
        db.resolve(title, steam_app_id, |_| false).id
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
    fn a_steam_app_id_is_found_inside_another_stores_id() {
        // The real database has no Steam rows: `umu-504230` on a GOG or Epic row is the AppID.
        let db = sample();
        assert_eq!(db.by_steam_app_id(504230)[0].umu_id, "umu-504230");
        assert!(db.by_steam_app_id(999999).is_empty());
        let named = Database::new(vec![row("Star Citizen", "umu-starcitizen", "none")]);
        assert!(named.by_steam_app_id(0).is_empty());
    }

    #[test]
    fn a_title_matches_across_spellings() {
        let db = sample();
        assert_eq!(
            db.by_title("baldurs gate 2 shadows of amn")[0].umu_id,
            "umu-257350"
        );
        assert!(db.by_title("Something Else").is_empty());
    }

    #[test]
    fn the_steam_id_wins_over_the_title() {
        // The id is exact; the title is a heuristic, and where they disagree the exact
        // one has to win or the fixes applied are for the wrong game.
        let db = Database::new(vec![
            row("Celeste", "umu-by-title", "gog"),
            row("Something Else Entirely", "umu-504230", "egs"),
        ]);
        assert_eq!(id(&db, "Celeste", Some(504230)), "umu-504230");
        assert_eq!(id(&db, "Celeste", None), "umu-by-title");
    }

    #[test]
    fn the_matched_rows_store_travels_with_the_id() {
        // Protonfixes looks in the store's folder, so the id alone finds nothing.
        let db = sample();
        assert_eq!(
            db.resolve("Deus Ex", None, |_| false),
            GameFix {
                id: "umu-6910".into(),
                store: "gog".into()
            }
        );
        assert_eq!(
            db.resolve("Nobody Indexed This", None, |_| false),
            GameFix::default()
        );
    }

    #[test]
    fn among_a_games_stores_the_one_with_a_fix_is_chosen() {
        let db = Database::new(vec![
            row("Horizon Zero Dawn", "umu-1151640", "egs"),
            row("Horizon Zero Dawn", "umu-1151640", "gog"),
        ]);
        let only_gog = |fix: &GameFix| fix.store == "gog";
        assert_eq!(db.resolve("Horizon Zero Dawn", None, only_gog).store, "gog");
        assert_eq!(
            db.resolve("Horizon Zero Dawn", Some(1151640), only_gog)
                .store,
            "gog"
        );
        // No fix anywhere: the first row, as before.
        assert_eq!(
            db.resolve("Horizon Zero Dawn", None, |_| false).store,
            "egs"
        );
    }

    #[test]
    fn a_shipped_fix_under_the_steam_app_id_needs_no_row() {
        // The Witcher 3 has no row, but protonfixes ships `gamefixes-steam/292030.py`.
        let db = sample();
        let ships_292030 = |fix: &GameFix| fix.id == "292030";
        assert_eq!(
            db.resolve("The Witcher 3: Wild Hunt", Some(292030), ships_292030),
            GameFix::steam(292030)
        );
        // Not shipped, or no AppID to try: the generic id, as before.
        assert_eq!(
            id(&db, "The Witcher 3: Wild Hunt", Some(292030)),
            DEFAULT_ID
        );
        assert_eq!(
            db.resolve("The Witcher 3: Wild Hunt", None, ships_292030),
            GameFix::default()
        );
    }

    #[test]
    fn the_steam_app_id_fix_ranks_between_rows_with_and_without_one() {
        let db = Database::new(vec![row("Celeste", "umu-504230", "egs")]);
        // A row whose fix the build ships wins.
        let both = |fix: &GameFix| fix.id == "umu-504230" || fix.id == "504230";
        assert_eq!(db.resolve("Celeste", Some(504230), both).id, "umu-504230");
        // A row without one loses to a shipped Steam fix.
        let steam_only = |fix: &GameFix| fix.id == "504230";
        assert_eq!(
            db.resolve("Celeste", Some(504230), steam_only),
            GameFix::steam(504230)
        );
        // Neither shipped: the row, as before.
        assert_eq!(id(&db, "Celeste", Some(504230)), "umu-504230");
    }

    #[test]
    fn a_fix_is_looked_up_where_protonfixes_would() {
        let build = std::env::temp_dir().join(format!("gameyfin-umu-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&build);
        for file in [
            "gamefixes-gog/umu-6910.py",
            "gamefixes-umu/umu-starcitizen.py",
            "gamefixes-steam/292030.py",
        ] {
            let path = build.join("protonfixes").join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let fix = |id: &str, store: &str| GameFix::checked(id, store).unwrap();
        assert!(fix("umu-6910", "gog").shipped_in(&build));
        assert!(!fix("umu-6910", "egs").shipped_in(&build));
        assert!(!fix("umu-6910", "none").shipped_in(&build));
        assert!(fix("umu-starcitizen", "none").shipped_in(&build));
        // A bare number is a Steam AppID whatever the store says.
        assert!(fix("292030", "none").shipped_in(&build));
        assert!(!fix("umu-292030", "steam").shipped_in(&build));
        std::fs::remove_dir_all(&build).unwrap();
    }

    #[test]
    fn a_typed_fix_is_checked_before_it_reaches_umu() {
        assert_eq!(
            GameFix::checked(" 292030 ", "Steam").unwrap(),
            GameFix {
                id: "292030".into(),
                store: "steam".into()
            }
        );
        for (id, store) in [
            ("", "gog"),
            ("umu-1 && rm", "gog"),
            ("../default", "none"),
            ("os.system", "none"),
            ("umu-1", "my-store"),
        ] {
            assert!(GameFix::checked(id, store).is_err(), "{id:?} {store:?}");
        }
    }

    #[test]
    fn searching_ranks_the_closest_title_first_and_finds_ids() {
        let db = Database::new(vec![
            row("The Witcher 2: Assassins of Kings", "umu-20920", "gog"),
            row("Gwent: The Witcher Card Game", "umu-1284410", "gog"),
            row("Witcher", "umu-20900", "gog"),
            row("Celeste", "umu-504230", "egs"),
        ]);
        let titles = |query: &str| -> Vec<&str> {
            db.search(query, 10)
                .into_iter()
                .map(|e| e.title.as_str())
                .collect()
        };
        assert_eq!(
            titles("witcher"),
            [
                "Witcher",
                "Gwent: The Witcher Card Game",
                "The Witcher 2: Assassins of Kings"
            ]
        );
        assert_eq!(titles("504230"), ["Celeste"]);
        assert!(titles("   ").is_empty());
        assert_eq!(db.search("witcher", 1).len(), 1);
    }

    #[test]
    fn an_unknown_game_gets_the_generic_id() {
        // The fallback has to be exactly the previous behaviour: never a failed launch.
        let db = sample();
        assert_eq!(id(&db, "A Game Nobody Indexed", None), DEFAULT_ID);
        assert_eq!(id(&db, "A Game Nobody Indexed", Some(1)), DEFAULT_ID);
        assert_eq!(
            id(&Database::default(), "Celeste", Some(504230)),
            DEFAULT_ID
        );
    }

    #[test]
    fn rows_without_a_umu_id_are_not_indexed() {
        // Matching one would set GAMEID to an empty string, which umu reads as no id at
        // all but with a warning, and is strictly worse than the documented default.
        let db = Database::new(vec![row("Broken Row", "", "gog")]);
        assert!(db.by_title("Broken Row").is_empty());
        assert_eq!(id(&db, "Broken Row", Some(1)), DEFAULT_ID);
    }

    #[test]
    fn a_duplicate_row_does_not_displace_the_first() {
        let db = Database::new(vec![
            row("Doom", "umu-first", "gog"),
            row("Doom", "umu-second", "gog"),
        ]);
        assert_eq!(id(&db, "Doom", None), "umu-first");
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
        // The indexes are not stored, so they have to work after a reload.
        assert_eq!(id(&cached.database, "Celeste", Some(504230)), "umu-504230");
        assert_eq!(id(&cached.database, "deus ex", None), "umu-6910");

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

        let http = gameyfin_api::http_client().build().unwrap();
        let db = fetch(&http, &format!("{}/umu_api.php", server.url()))
            .await
            .expect("the server answered");

        assert_eq!(db.len(), 1);
        assert_eq!(id(&db, "Celeste", Some(504230)), "umu-504230");
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

        let http = gameyfin_api::http_client().build().unwrap();
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

        let http = gameyfin_api::http_client().build().unwrap();
        let db = fetch(&http, &format!("{}/umu_api.php", server.url()))
            .await
            .expect("unknown fields are ignored");
        assert_eq!(id(&db, "Celeste", None), "umu-504230");
        mock.assert_async().await;
    }
}
