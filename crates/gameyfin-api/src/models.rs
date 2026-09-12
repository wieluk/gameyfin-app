//! DTOs mirroring the server's Hilla types (`GameDto`/`LibraryDto`/`ImageDto`). Optional
//! fields are omitted on the wire, not null (`@JsonInclude(NON_NULL)`).

use serde::{Deserialize, Deserializer, Serialize};

/// An `ImageDto`. `blurhash` lets the UI paint a placeholder before the bytes arrive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
    /// Where the server serves this image: `/images/<kind>/<id>`, relative to its root.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Library {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub game_ids: Vec<i64>,
}

/// A `DownloadProviderDto`. Providers are contributed by plugins (`directdownload`,
/// `torrentdownload`); higher `priority` wins when several can serve a game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

/// A `GameMetadataUserDto`. `original_ids` holds the Steam AppID for exact save matching; a
/// stock 2.4.0 server omits it, so matching falls back to the title.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameMetadata {
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub original_ids: Option<std::collections::HashMap<String, String>>,
}

/// A `GameDto` (the `GameUserDto` variant).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
    #[serde(default, deserialize_with = "string_list")]
    pub perspectives: Vec<String>,
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
    /// The Steam AppID, when the Steam plugin recorded one. Stock servers hide it from users.
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

/// A list whose items are plain strings or enum DTOs: Gameyfin sends genres and platforms
/// as objects carrying a constant and a `displayName`, and the UI wants one shape.
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
        // An unrecognised type falls back to cover rather than 404ing.
        for (id, kind, expected) in [
            (7, "COVER", "/images/cover/7"),
            (8, "HEADER", "/images/header/8"),
            (9, "SCREENSHOT", "/images/screenshot/9"),
            (1, "SOMETHING", "/images/cover/1"),
        ] {
            let image = Image {
                id,
                r#type: kind.into(),
                blurhash: None,
            };
            assert_eq!(image.path(), expected, "{kind}");
        }
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
        // Absent on a stock 2.4.0 server, which exposes only fileSize to non-admins.
        for (metadata, expected) in [
            (
                serde_json::json!({"fileSize": 100, "originalIds": {"steam": "504230", "igdb": "7788"}}),
                Some(504230),
            ),
            (serde_json::json!({"fileSize": 100}), None),
        ] {
            let json = serde_json::json!({"id": 1, "title": "Celeste", "metadata": metadata});
            let game: Game = serde_json::from_value(json).unwrap();
            assert_eq!(game.steam_app_id(), expected);
        }
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
