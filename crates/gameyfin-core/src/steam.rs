//! Non-Steam shortcuts in Steam's binary `shortcuts.vdf`. Other launchers share the file, so it
//! is parsed into a generic tree and written back with only this app's entry changed.

use std::path::{Path, PathBuf};

/// One node of a VDF document. `Map` keeps insertion order: Steam indexes shortcuts by
/// position, so reordering silently reassigns entries.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Map(Vec<(String, Value)>),
    Str(String),
    Int(i32),
    /// A 64-bit value, kept only so documents containing one survive a round trip.
    Wide(u64),
}

impl Value {
    /// The nested map under `key`, if this is a map and that entry is one.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    fn entries_mut(&mut self) -> Option<&mut Vec<(String, Value)>> {
        match self {
            Value::Map(entries) => Some(entries),
            _ => None,
        }
    }
}

mod tag {
    pub const MAP: u8 = 0x00;
    pub const STR: u8 = 0x01;
    pub const INT: u8 = 0x02;
    pub const WIDE_U64: u8 = 0x07;
    pub const END: u8 = 0x08;
}

#[derive(Debug, thiserror::Error)]
pub enum VdfError {
    #[error("unexpected end of file while reading {0}")]
    Truncated(&'static str),
    #[error("unknown value tag {0:#04x} at byte {1}")]
    UnknownTag(u8, usize),
    #[error("a key or string was not valid UTF-8")]
    NotUtf8,
}

/// Parses a VDF document. The top level is a map with no opening tag of its own, so this
/// reads until the input runs out rather than to a terminator.
pub fn parse(bytes: &[u8]) -> Result<Value, VdfError> {
    let mut cursor = 0usize;
    let mut entries = Vec::new();
    while cursor < bytes.len() {
        // Steam terminates the outermost map too; anything after it is padding.
        if bytes[cursor] == tag::END {
            break;
        }
        let (key, value) = read_entry(bytes, &mut cursor)?;
        entries.push((key, value));
    }
    Ok(Value::Map(entries))
}

fn read_entry(bytes: &[u8], cursor: &mut usize) -> Result<(String, Value), VdfError> {
    let tag = *bytes.get(*cursor).ok_or(VdfError::Truncated("a tag"))?;
    *cursor += 1;
    let key = read_cstring(bytes, cursor)?;

    let value = match tag {
        tag::MAP => read_map(bytes, cursor)?,
        tag::STR => Value::Str(read_cstring(bytes, cursor)?),
        tag::INT => Value::Int(read_i32(bytes, cursor)?),
        tag::WIDE_U64 => Value::Wide(read_u64(bytes, cursor)?),
        other => return Err(VdfError::UnknownTag(other, *cursor)),
    };
    Ok((key, value))
}

fn read_map(bytes: &[u8], cursor: &mut usize) -> Result<Value, VdfError> {
    let mut entries = Vec::new();
    loop {
        match bytes.get(*cursor) {
            None => return Err(VdfError::Truncated("a map")),
            Some(&tag::END) => {
                *cursor += 1;
                return Ok(Value::Map(entries));
            }
            Some(_) => {
                let (key, value) = read_entry(bytes, cursor)?;
                entries.push((key, value));
            }
        }
    }
}

fn read_cstring(bytes: &[u8], cursor: &mut usize) -> Result<String, VdfError> {
    let start = *cursor;
    let end = bytes[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|offset| start + offset)
        .ok_or(VdfError::Truncated("a string"))?;
    *cursor = end + 1;
    // Steam writes UTF-8, but a hand-edited file need not be. Lossy rather than fatal:
    // one mangled title is better than refusing to read the user's whole library.
    Ok(String::from_utf8_lossy(&bytes[start..end]).into_owned())
}

fn read_i32(bytes: &[u8], cursor: &mut usize) -> Result<i32, VdfError> {
    let slice = bytes
        .get(*cursor..*cursor + 4)
        .ok_or(VdfError::Truncated("an integer"))?;
    *cursor += 4;
    Ok(i32::from_le_bytes(
        slice.try_into().expect("checked length"),
    ))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, VdfError> {
    let slice = bytes
        .get(*cursor..*cursor + 8)
        .ok_or(VdfError::Truncated("a wide integer"))?;
    *cursor += 8;
    Ok(u64::from_le_bytes(
        slice.try_into().expect("checked length"),
    ))
}

pub fn serialize(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    if let Value::Map(entries) = value {
        for (key, child) in entries {
            write_entry(&mut out, key, child);
        }
    }
    // The outermost map is terminated too, matching what Steam writes.
    out.push(tag::END);
    out
}

fn write_entry(out: &mut Vec<u8>, key: &str, value: &Value) {
    match value {
        Value::Map(entries) => {
            out.push(tag::MAP);
            write_cstring(out, key);
            for (child_key, child) in entries {
                write_entry(out, child_key, child);
            }
            out.push(tag::END);
        }
        Value::Str(text) => {
            out.push(tag::STR);
            write_cstring(out, key);
            write_cstring(out, text);
        }
        Value::Int(number) => {
            out.push(tag::INT);
            write_cstring(out, key);
            out.extend_from_slice(&number.to_le_bytes());
        }
        Value::Wide(number) => {
            out.push(tag::WIDE_U64);
            write_cstring(out, key);
            out.extend_from_slice(&number.to_le_bytes());
        }
    }
}

fn write_cstring(out: &mut Vec<u8>, text: &str) {
    // A NUL inside the text would terminate the field early and shift every byte after
    // it, so it is dropped rather than written.
    out.extend(text.bytes().filter(|&b| b != 0));
    out.push(0);
}

/// A non-Steam game as this app writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shortcut {
    pub app_name: String,
    /// The program Steam runs. Stored quoted, which is what Steam itself writes.
    pub exe: String,
    pub start_dir: String,
    pub icon: String,
    pub launch_options: String,
    /// Marks the entry as ours, so it can be found again to update or remove.
    pub tags: Vec<String>,
}

/// The tag written on every shortcut this app creates, so ours can be told from the user's
/// hand-made entries.
pub const OWNER_TAG: &str = "Gameyfin";

impl Shortcut {
    /// The id Steam derives for a non-Steam game: CRC-32 of exe and name with the top bit
    /// set. Computed here because it names the artwork files and identifies our own entry.
    pub fn app_id(&self) -> u32 {
        let mut hasher = Crc32::new();
        hasher.update(self.exe.as_bytes());
        hasher.update(self.app_name.as_bytes());
        hasher.finish() | 0x8000_0000
    }

    fn to_value(&self, existing: Option<&Value>) -> Value {
        // Start from the existing entry so fields Steam maintains, like LastPlayTime and artwork,
        // survive each write.
        let mut entries: Vec<(String, Value)> = match existing {
            Some(Value::Map(current)) => current.clone(),
            _ => Vec::new(),
        };

        let tags = Value::Map(
            self.tags
                .iter()
                .enumerate()
                .map(|(i, tag)| (i.to_string(), Value::Str(tag.clone())))
                .collect(),
        );

        // `appid` is stored as a signed 32-bit value; the top bit we set above makes it
        // negative, which is exactly what Steam writes.
        set(&mut entries, "appid", Value::Int(self.app_id() as i32));
        set(&mut entries, "AppName", Value::Str(self.app_name.clone()));
        set(&mut entries, "Exe", Value::Str(self.exe.clone()));
        set(&mut entries, "StartDir", Value::Str(self.start_dir.clone()));
        set(&mut entries, "icon", Value::Str(self.icon.clone()));
        set(
            &mut entries,
            "LaunchOptions",
            Value::Str(self.launch_options.clone()),
        );
        set(&mut entries, "tags", tags);

        // Defaults only for a new entry: overwriting these would undo deliberate choices
        // the user made in Steam's own interface.
        if existing.is_none() {
            set(&mut entries, "ShortcutPath", Value::Str(String::new()));
            set(&mut entries, "IsHidden", Value::Int(0));
            set(&mut entries, "AllowDesktopConfig", Value::Int(1));
            set(&mut entries, "AllowOverlay", Value::Int(1));
            set(&mut entries, "OpenVR", Value::Int(0));
            set(&mut entries, "Devkit", Value::Int(0));
            set(&mut entries, "DevkitGameID", Value::Str(String::new()));
            set(&mut entries, "DevkitOverrideAppID", Value::Int(0));
            set(&mut entries, "LastPlayTime", Value::Int(0));
            set(&mut entries, "FlatpakAppID", Value::Str(String::new()));
        }

        Value::Map(entries)
    }
}

/// Insert or replace a key, matched case-insensitively, since Steam varies the
/// capitalisation between versions (`appid` vs `Appid`).
fn set(entries: &mut Vec<(String, Value)>, key: &str, value: Value) {
    match entries
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
    {
        Some((_, slot)) => *slot = value,
        None => entries.push((key.to_string(), value)),
    }
}

/// Adds or updates one shortcut, matched by app id, so running it twice for a game updates
/// the entry rather than adding a duplicate.
pub fn upsert(document: &mut Value, shortcut: &Shortcut) {
    let list = shortcuts_map(document);
    let target = shortcut.app_id() as i32;

    let existing = list
        .iter()
        .position(|(_, entry)| matches!(entry.get("appid"), Some(Value::Int(id)) if *id == target));

    match existing {
        Some(index) => {
            let updated = shortcut.to_value(Some(&list[index].1));
            list[index].1 = updated;
        }
        None => {
            // Steam keys entries by their index as a string; the next free one is the
            // current length, since the list is always contiguous.
            let key = list.len().to_string();
            list.push((key, shortcut.to_value(None)));
        }
    }
}

/// Remove a shortcut by app id. True when one was there.
pub fn remove(document: &mut Value, app_id: u32) -> bool {
    let list = shortcuts_map(document);
    let target = app_id as i32;
    let Some(index) = list
        .iter()
        .position(|(_, entry)| matches!(entry.get("appid"), Some(Value::Int(id)) if *id == target))
    else {
        return false;
    };
    list.remove(index);
    // Steam reads the list by index, so the keys have to stay contiguous; leaving a hole
    // makes every entry after it invisible.
    for (position, (key, _)) in list.iter_mut().enumerate() {
        *key = position.to_string();
    }
    true
}

pub fn names(document: &Value) -> Vec<String> {
    let Some(Value::Map(list)) = document.get("shortcuts") else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|(_, entry)| entry.get("AppName").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn shortcuts_map(document: &mut Value) -> &mut Vec<(String, Value)> {
    let entries = document
        .entries_mut()
        .expect("a parsed document is always a map");

    if !entries
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("shortcuts"))
    {
        entries.push(("shortcuts".to_string(), Value::Map(Vec::new())));
    }

    entries
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case("shortcuts"))
        .and_then(|(_, value)| value.entries_mut())
        .expect("just ensured it is a map")
}

/// Where Steam might be, relative to home. The Flatpak path is listed because that install
/// keeps `userdata` inside its sandbox and has no native path at all.
const STEAM_ROOTS: [&str; 4] = [
    ".local/share/Steam",
    ".steam/steam",
    ".steam/root",
    ".var/app/com.valvesoftware.Steam/data/Steam",
];

/// Every `shortcuts.vdf` under `home`, one per Steam account, since a shared machine has
/// one each and the game should reach all of them.
pub fn shortcut_files(home: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in STEAM_ROOTS {
        let userdata = home.join(root).join("userdata");
        let Ok(accounts) = std::fs::read_dir(&userdata) else {
            continue;
        };
        for account in accounts.flatten() {
            // `userdata/0` is Steam's placeholder for "no account", not a real profile.
            if account.file_name() == "0" {
                continue;
            }
            let config = account.path().join("config");
            if config.is_dir() {
                found.push(config.join("shortcuts.vdf"));
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Whether Steam appears to be installed for this user at all.
/// Steam's own folder, the one holding `steamapps` and `userdata`. A save scanner wants it
/// because Proton keeps each game's prefix under `steamapps/compatdata`.
pub fn root(home: &Path) -> Option<PathBuf> {
    STEAM_ROOTS
        .iter()
        .map(|root| home.join(root))
        .find(|root| root.join("userdata").is_dir())
}

pub fn is_installed(home: &Path) -> bool {
    STEAM_ROOTS
        .iter()
        .any(|root| home.join(root).join("userdata").is_dir())
}

/// Read a `shortcuts.vdf`, treating a missing or unreadable file as an empty document
/// (it can be locked while Steam runs; failing here should not fail an install).
pub fn read_document(path: &Path) -> Value {
    match std::fs::read(path) {
        Ok(bytes) => parse(&bytes).unwrap_or_else(|e| {
            tracing::warn!(?path, "could not parse Steam's shortcuts file: {e}");
            Value::Map(Vec::new())
        }),
        Err(_) => Value::Map(Vec::new()),
    }
}

/// Writes a document back, keeping one backup: this file is the only record of every
/// non-Steam game from every launcher, so a bug here would be silent and unrecoverable.
pub fn write_document(path: &Path, document: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let backup = path.with_extension("vdf.gameyfin-backup");
        // Best effort: a backup that cannot be written must not stop the write, but it is
        // worth knowing about.
        if let Err(e) = std::fs::copy(path, &backup) {
            tracing::warn!(?backup, "could not back up Steam's shortcuts file: {e}");
        }
    }

    // Written beside the target and renamed, so an interrupted write leaves the original
    // intact rather than a half-file Steam will reject.
    let temporary = path.with_extension("vdf.gameyfin-tmp");
    std::fs::write(&temporary, serialize(document))?;
    std::fs::rename(&temporary, path)
}

/// CRC-32 (IEEE, reflected), which is how Steam derives a shortcut id. Twenty lines rather
/// than a dependency, since nothing else in the app needs a CRC.
struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            let index = ((self.0 ^ u32::from(byte)) & 0xFF) as usize;
            self.0 = (self.0 >> 8) ^ TABLE[index];
        }
    }

    fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

static TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut value = i as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[i] = value;
        i += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_shortcut(name: &str) -> Shortcut {
        Shortcut {
            app_name: name.to_string(),
            exe: format!("\"/games/{name}/run.sh\""),
            start_dir: format!("\"/games/{name}\""),
            icon: String::new(),
            launch_options: String::new(),
            tags: vec![OWNER_TAG.to_string()],
        }
    }

    #[test]
    fn crc32_matches_the_known_check_value() {
        // The standard CRC-32 check value for "123456789".
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }

    #[test]
    fn an_app_id_is_negative_and_stable_per_game() {
        // Steam stores it signed, so the top bit is what makes it read as negative there.
        let id = a_shortcut("Celeste").app_id();
        assert_ne!(id & 0x8000_0000, 0);
        assert!((id as i32) < 0);

        assert_eq!(a_shortcut("Hades").app_id(), a_shortcut("Hades").app_id());
        assert_ne!(a_shortcut("Hades").app_id(), a_shortcut("Tunic").app_id());
    }

    #[test]
    fn a_document_survives_a_round_trip() {
        let mut document = Value::Map(Vec::new());
        upsert(&mut document, &a_shortcut("Celeste"));
        upsert(&mut document, &a_shortcut("Tunic"));

        let bytes = serialize(&document);
        let reparsed = parse(&bytes).expect("what we wrote is readable");
        assert_eq!(reparsed, document);
        assert_eq!(names(&reparsed), vec!["Celeste", "Tunic"]);
    }

    #[test]
    fn keys_written_by_other_tools_are_preserved() {
        // Heroic and Lutris write here too, and their entries must survive.
        let mut document = Value::Map(vec![(
            "shortcuts".to_string(),
            Value::Map(vec![(
                "0".to_string(),
                Value::Map(vec![
                    ("appid".to_string(), Value::Int(-12345)),
                    ("AppName".to_string(), Value::Str("Someone else's".into())),
                    ("SomeFutureKey".to_string(), Value::Wide(42)),
                ]),
            )]),
        )]);

        upsert(&mut document, &a_shortcut("Celeste"));
        let reparsed = parse(&serialize(&document)).unwrap();

        assert_eq!(names(&reparsed), vec!["Someone else's", "Celeste"]);
        let theirs = reparsed.get("shortcuts").unwrap().get("0").unwrap();
        assert_eq!(theirs.get("SomeFutureKey"), Some(&Value::Wide(42)));
    }

    #[test]
    fn adding_the_same_game_twice_updates_rather_than_duplicates() {
        let mut document = Value::Map(Vec::new());
        upsert(&mut document, &a_shortcut("Celeste"));

        let mut changed = a_shortcut("Celeste");
        changed.launch_options = "--fullscreen".to_string();
        upsert(&mut document, &changed);

        assert_eq!(names(&document).len(), 1);
        let entry = document.get("shortcuts").unwrap().get("0").unwrap();
        assert_eq!(
            entry.get("LaunchOptions"),
            Some(&Value::Str("--fullscreen".into()))
        );
    }

    #[test]
    fn steams_own_fields_are_not_reset_by_an_update() {
        // Overwriting these would undo the user's per-game choices in Steam every time
        // the app touched the file.
        let mut document = Value::Map(Vec::new());
        upsert(&mut document, &a_shortcut("Celeste"));

        {
            let list = shortcuts_map(&mut document);
            let entry = list[0].1.entries_mut().unwrap();
            set(entry, "LastPlayTime", Value::Int(1_700_000_000));
            set(entry, "AllowOverlay", Value::Int(0));
        }

        upsert(&mut document, &a_shortcut("Celeste"));
        let entry = document.get("shortcuts").unwrap().get("0").unwrap();
        assert_eq!(entry.get("LastPlayTime"), Some(&Value::Int(1_700_000_000)));
        assert_eq!(entry.get("AllowOverlay"), Some(&Value::Int(0)));
    }

    #[test]
    fn removing_an_entry_renumbers_the_rest() {
        // Steam reads the list by index; a hole hides everything after it.
        let mut document = Value::Map(Vec::new());
        for name in ["A", "B", "C"] {
            upsert(&mut document, &a_shortcut(name));
        }

        assert!(remove(&mut document, a_shortcut("B").app_id()));
        assert_eq!(names(&document), vec!["A", "C"]);

        let Some(Value::Map(list)) = document.get("shortcuts") else {
            panic!("expected a list");
        };
        assert_eq!(
            list.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["0", "1"]
        );

        // An id that was never there is reported rather than silently succeeding.
        assert!(!remove(&mut document, 123));
    }

    #[test]
    fn a_truncated_file_is_an_error_rather_than_a_panic() {
        // Steam can be mid-write, and a crash leaves a partial file.
        let mut bytes = serialize(&{
            let mut d = Value::Map(Vec::new());
            upsert(&mut d, &a_shortcut("Celeste"));
            d
        });
        bytes.truncate(bytes.len() / 2);
        assert!(parse(&bytes).is_err());

        // An unknown tag is rejected rather than guessed at.
        assert!(matches!(
            parse(&[0x7F, b'k', 0, 0]),
            Err(VdfError::UnknownTag(0x7F, _))
        ));
    }

    #[test]
    fn case_differences_in_steams_keys_do_not_create_duplicates() {
        // Steam has shipped both `appid` and `AppID`; a case-sensitive match would give
        // one entry two ids.
        let shortcut = a_shortcut("Celeste");
        let mut document = Value::Map(vec![(
            "Shortcuts".to_string(),
            Value::Map(vec![(
                "0".to_string(),
                Value::Map(vec![
                    ("AppID".to_string(), Value::Int(shortcut.app_id() as i32)),
                    ("appname".to_string(), Value::Str("Celeste".into())),
                ]),
            )]),
        )]);

        upsert(&mut document, &shortcut);

        let Some(Value::Map(list)) = document.get("shortcuts") else {
            panic!("expected the list");
        };
        assert_eq!(list.len(), 1, "matched the existing entry");
        let Value::Map(entry) = &list[0].1 else {
            panic!("expected a map");
        };
        let ids = entry
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("appid"))
            .count();
        assert_eq!(ids, 1, "one id, not two spellings of it");
    }

    #[test]
    fn a_nul_in_a_title_cannot_shift_the_rest_of_the_file() {
        let mut shortcut = a_shortcut("Celeste");
        shortcut.app_name = "Cel\0este".to_string();
        let mut document = Value::Map(Vec::new());
        upsert(&mut document, &shortcut);

        let reparsed = parse(&serialize(&document)).expect("still readable");
        assert_eq!(names(&reparsed), vec!["Celeste"]);
    }

    #[test]
    fn writing_keeps_a_backup_and_is_atomic() {
        let dir = std::env::temp_dir().join(format!("gameyfin-vdf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shortcuts.vdf");

        let mut first = Value::Map(Vec::new());
        upsert(&mut first, &a_shortcut("Celeste"));
        write_document(&path, &first).unwrap();
        assert!(!path.with_extension("vdf.gameyfin-backup").exists());

        let mut second = first.clone();
        upsert(&mut second, &a_shortcut("Tunic"));
        write_document(&path, &second).unwrap();

        let backup = path.with_extension("vdf.gameyfin-backup");
        assert!(backup.exists(), "the previous file is kept");
        assert_eq!(
            names(&parse(&std::fs::read(&backup).unwrap()).unwrap()).len(),
            1
        );
        assert_eq!(names(&read_document(&path)).len(), 2);
        assert!(!path.with_extension("vdf.gameyfin-tmp").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_reads_as_an_empty_document() {
        let document = read_document(Path::new("/definitely/not/here/shortcuts.vdf"));
        assert_eq!(document, Value::Map(Vec::new()));
        assert!(names(&document).is_empty());
    }

    #[test]
    fn every_signed_in_account_is_found() {
        let home = std::env::temp_dir().join(format!("gameyfin-steam-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for account in ["0", "1234", "5678"] {
            std::fs::create_dir_all(
                home.join(".local/share/Steam/userdata")
                    .join(account)
                    .join("config"),
            )
            .unwrap();
        }

        let files = shortcut_files(&home);
        assert_eq!(files.len(), 2, "the `0` placeholder is not an account");
        assert!(files.iter().all(|p| p.ends_with("shortcuts.vdf")));
        assert!(is_installed(&home));

        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn no_steam_means_no_files_and_no_error() {
        let home = std::env::temp_dir().join(format!("gameyfin-nosteam-{}", std::process::id()));
        assert!(shortcut_files(&home).is_empty());
        assert!(!is_installed(&home));
    }
}
