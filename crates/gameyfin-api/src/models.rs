//! DTOs mirroring the Gameyfin server's Hilla types.
//!
//! Field names follow the server's `GameDto` / `LibraryDto` / `ImageDto` (see
//! `app/src/main/kotlin/org/gameyfin/app/games/dto/`). Everything optional is genuinely
//! optional on the wire: the server annotates its DTOs `@JsonInclude(NON_NULL)`, so absent
//! fields are omitted rather than sent as null.

use serde::{Deserialize, Deserializer, Serialize};

/// An `ImageDto`. `blurhash` lets the UI paint a placeholder before the bytes arrive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub id: i64,
    #[serde(default = "default_image_type")]
    pub r#type: String,
    #[serde(default)]
    pub blurhash: Option<String>,
}

fn default_image_type() -> String {
    "COVER".to_string()
}

impl Image {
    /// REST path serving this image, relative to the server root.
    ///
    /// Gameyfin serves artwork from `/images/<kind>/<id>` and permits it anonymously
    /// (`SecurityConfig` allows `/images/**`).
    pub fn path(&self) -> String {
        let kind = match self.r#type.to_ascii_uppercase().as_str() {
            "HEADER" => "header",
            "SCREENSHOT" => "screenshot",
            _ => "cover",
        };
        format!("/images/{kind}/{}", self.id)
    }
}

/// A `LibraryDto`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub game_ids: Vec<i64>,
}

/// A `DownloadProviderDto`. Providers are contributed by plugins (`directdownload`,
/// `torrentdownload`); higher `priority` wins when several can serve a game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProvider {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub short_description: Option<String>,
}

/// A `GameMetadataUserDto`.
///
/// `original_ids` maps a metadata plugin id to that provider's own identifier, the Steam
/// plugin stores the Steam AppID there (`plugins/steam/.../SteamPlugin.kt`), which is what
/// makes deterministic Ludusavi matching possible.
///
/// The stock 2.4.0 server exposes **only** `fileSize` to non-admin users; `originalIds`
/// arrives with PR C1 of the server plan. It is optional here so the client works against
/// both, degrading to title-based save matching when absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMetadata {
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub original_ids: Option<std::collections::HashMap<String, String>>,
}

/// A `GameDto` (the `GameUserDto` variant).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub library_id: i64,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    /// ISO-8601 date as serialized by Hilla from a Kotlin `LocalDate`.
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub user_rating: Option<i32>,
    #[serde(default)]
    pub critic_rating: Option<i32>,
    #[serde(default, deserialize_with = "string_list")]
    pub platforms: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub genres: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub themes: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub publishers: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub developers: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub features: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub collection_ids: Vec<i64>,
    #[serde(default)]
    pub cover: Option<Image>,
    #[serde(default)]
    pub header: Option<Image>,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub video_urls: Vec<String>,
    #[serde(default)]
    pub metadata: GameMetadata,
}

impl Game {
    /// The Steam AppID, when a Steam-plugin match recorded one.
    ///
    /// Returns `None` on a stock server, which does not expose `originalIds` to users.
    pub fn steam_app_id(&self) -> Option<u32> {
        self.metadata
            .original_ids
            .as_ref()?
            .iter()
            .find(|(plugin, _)| plugin.eq_ignore_ascii_case("steam"))
            .and_then(|(_, id)| id.parse().ok())
    }

    /// REST path for downloading this game from a given provider.
    pub fn download_path(&self, provider_key: &str) -> String {
        format!("/download/{}?provider={}", self.id, urlencode(provider_key))
    }
}

/// A `UserInfoDto` / `ExtendedUserInfoDto` from `UserEndpoint.getUserInfo()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    #[serde(default)]
    pub id: Option<i64>,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub avatar: Option<Image>,
}

impl UserInfo {
    pub fn is_admin(&self) -> bool {
        self.roles
            .iter()
            .any(|r| r.eq_ignore_ascii_case("ROLE_ADMIN") || r.eq_ignore_ascii_case("ADMIN"))
    }
}

/// Percent-encode a query-parameter value.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Deserialize a list whose items are either plain strings or enum DTOs.
///
/// Gameyfin serializes enum-valued fields (platforms, genres, ...) as objects carrying both
/// the raw constant and a `displayName`. We keep the display form, falling back to the
/// constant, so the UI never has to know which shape arrived.
fn string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrEnum {
        Plain(String),
        Enum {
            #[serde(rename = "displayName")]
            display_name: Option<String>,
            name: Option<String>,
        },
        // Anything unrecognised is dropped rather than failing the whole game.
        Other(serde::de::IgnoredAny),
    }

    let raw = Option::<Vec<StringOrEnum>>::deserialize(deserializer)?;
    Ok(raw
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| match item {
            StringOrEnum::Plain(s) => Some(s),
            StringOrEnum::Enum { display_name, name } => display_name.or(name),
            StringOrEnum::Other(_) => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_paths_follow_type() {
        let cover = Image {
            id: 7,
            r#type: "COVER".into(),
            blurhash: None,
        };
        let header = Image {
            id: 8,
            r#type: "HEADER".into(),
            blurhash: None,
        };
        let shot = Image {
            id: 9,
            r#type: "SCREENSHOT".into(),
            blurhash: None,
        };
        assert_eq!(cover.path(), "/images/cover/7");
        assert_eq!(header.path(), "/images/header/8");
        assert_eq!(shot.path(), "/images/screenshot/9");
    }

    #[test]
    fn unknown_image_type_falls_back_to_cover() {
        let odd = Image {
            id: 1,
            r#type: "SOMETHING".into(),
            blurhash: None,
        };
        assert_eq!(odd.path(), "/images/cover/1");
    }

    #[test]
    fn parses_enum_dtos_and_plain_strings() {
        let json = serde_json::json!({
            "id": 1,
            "title": "Celeste",
            "libraryId": 2,
            "platforms": [{"name": "PC_MICROSOFT_WINDOWS", "displayName": "Windows"}],
            "genres": ["Platformer"],
            "keywords": [{"name": "PIXEL_ART"}]
        });
        let game: Game = serde_json::from_value(json).unwrap();
        assert_eq!(game.platforms, vec!["Windows"]);
        assert_eq!(game.genres, vec!["Platformer"]);
        // No displayName, so the raw constant is kept.
        assert_eq!(game.keywords, vec!["PIXEL_ART"]);
    }

    #[test]
    fn minimal_game_deserializes() {
        // The server omits null fields, so a sparse game must still parse.
        let json = serde_json::json!({"id": 5, "title": "Unknown"});
        let game: Game = serde_json::from_value(json).unwrap();
        assert_eq!(game.id, 5);
        assert!(game.cover.is_none());
        assert!(game.platforms.is_empty());
        assert_eq!(game.metadata.file_size, 0);
    }

    #[test]
    fn steam_app_id_read_from_original_ids() {
        let json = serde_json::json!({
            "id": 1,
            "title": "Celeste",
            "metadata": {"fileSize": 100, "originalIds": {"steam": "504230", "igdb": "7788"}}
        });
        let game: Game = serde_json::from_value(json).unwrap();
        assert_eq!(game.steam_app_id(), Some(504230));
    }

    #[test]
    fn steam_app_id_absent_on_stock_server() {
        // A stock 2.4.0 server exposes only fileSize to non-admins.
        let json = serde_json::json!({"id": 1, "title": "Celeste", "metadata": {"fileSize": 100}});
        let game: Game = serde_json::from_value(json).unwrap();
        assert_eq!(game.steam_app_id(), None);
    }

    #[test]
    fn download_path_encodes_provider() {
        let json = serde_json::json!({"id": 42, "title": "X"});
        let game: Game = serde_json::from_value(json).unwrap();
        assert_eq!(
            game.download_path("direct download"),
            "/download/42?provider=direct%20download"
        );
    }
}
