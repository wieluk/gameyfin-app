//! Which platform a backup was taken on. Save locations are not translated between
//! operating systems, so an archive carries this and a mismatch is reported, not guessed.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "UPPERCASE")]
#[ts(export)]
pub enum SavePlatform {
    Windows,
    Linux,
    /// A Windows game running under Proton or Wine. Its saves keep Windows-shaped paths
    /// inside the prefix, which is what makes Windows interchange possible at all.
    Proton,
    MacOS,
    Unknown,
}

impl SavePlatform {
    /// What this machine produces for a game that runs natively.
    pub fn native() -> Self {
        if cfg!(target_os = "windows") {
            SavePlatform::Windows
        } else if cfg!(target_os = "macos") {
            SavePlatform::MacOS
        } else {
            SavePlatform::Linux
        }
    }

    /// What this machine produces for a game, given whether it runs through a prefix.
    pub fn for_game(windows_program: bool) -> Self {
        match (windows_program, SavePlatform::native()) {
            (true, SavePlatform::Windows) => SavePlatform::Windows,
            (true, _) => SavePlatform::Proton,
            (false, native) => native,
        }
    }

    /// Whether a backup from `self` restores onto `other` untranslated. Windows and Proton
    /// interchange because a prefix holds Windows-shaped paths.
    pub fn interchangeable_with(self, other: SavePlatform) -> bool {
        use SavePlatform::*;
        match (self, other) {
            (Unknown, _) | (_, Unknown) => true,
            (a, b) if a == b => true,
            (Windows, Proton) | (Proton, Windows) => true,
            _ => false,
        }
    }
}

impl fmt::Display for SavePlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            SavePlatform::Windows => "WINDOWS",
            SavePlatform::Linux => "LINUX",
            SavePlatform::Proton => "PROTON",
            SavePlatform::MacOS => "MACOS",
            SavePlatform::Unknown => "UNKNOWN",
        };
        f.write_str(name)
    }
}

impl FromStr for SavePlatform {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value.trim().to_ascii_uppercase().as_str() {
            "WINDOWS" => SavePlatform::Windows,
            "LINUX" => SavePlatform::Linux,
            "PROTON" => SavePlatform::Proton,
            "MACOS" => SavePlatform::MacOS,
            _ => SavePlatform::Unknown,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_and_proton_interchange() {
        use SavePlatform::*;
        for (a, b, expected) in [
            (Windows, Proton, true),
            (Proton, Windows, true),
            (Windows, Linux, false),
            (Linux, Proton, false),
            // Archives predating platform tagging must stay restorable.
            (Unknown, Linux, true),
            (Windows, Unknown, true),
        ] {
            assert_eq!(a.interchangeable_with(b), expected, "{a:?} with {b:?}");
        }
    }

    #[test]
    fn a_windows_program_off_windows_is_tagged_proton() {
        let tagged = SavePlatform::for_game(true);
        if cfg!(target_os = "windows") {
            assert_eq!(SavePlatform::Windows, tagged);
        } else {
            assert_eq!(SavePlatform::Proton, tagged);
        }
    }

    #[test]
    fn round_trips_through_its_wire_form() {
        for platform in [
            SavePlatform::Windows,
            SavePlatform::Linux,
            SavePlatform::Proton,
            SavePlatform::MacOS,
            SavePlatform::Unknown,
        ] {
            assert_eq!(platform, platform.to_string().parse().unwrap());
        }
    }

    #[test]
    fn an_unrecognised_platform_degrades_rather_than_failing() {
        assert_eq!(
            SavePlatform::Unknown,
            "SteamDeck".parse::<SavePlatform>().unwrap()
        );
    }
}
