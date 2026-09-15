//! Recognising installer toolkits by markers inside the executable, so the destination can be
//! passed on the command line and the game lands where the app can find it.

use std::io::Read;
use std::path::Path;

use crate::error::CoreResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    /// Inno Setup, GOG's installers, and most repacks.
    InnoSetup,
    Nsis,
    /// InstallShield. Its destination switch varies by version, so it is recognised but
    /// not driven.
    InstallShield,
    Unknown,
}

impl InstallerKind {
    pub fn label(self) -> &'static str {
        match self {
            InstallerKind::InnoSetup => "Inno Setup",
            InstallerKind::Nsis => "NSIS",
            InstallerKind::InstallShield => "InstallShield",
            InstallerKind::Unknown => "unknown installer",
        }
    }

    /// Whether the toolkit has silent switches.
    pub fn runs_unattended(self) -> bool {
        matches!(self, InstallerKind::InnoSetup | InstallerKind::Nsis)
    }

    pub fn default_unattended_args(self) -> &'static str {
        match self {
            // `/NORESTART` keeps Windows from rebooting afterwards.
            InstallerKind::InnoSetup => "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART",
            InstallerKind::Nsis => "/S",
            InstallerKind::InstallShield | InstallerKind::Unknown => "",
        }
    }

    /// Arguments that point this installer at `destination`. Empty when the toolkit is unknown.
    pub fn destination_args(self, destination: &str) -> Vec<String> {
        match self {
            // `/DIR` sets the target; `/SP-` skips the "This will install…" prompt that
            // otherwise appears before the destination page is even reached.
            InstallerKind::InnoSetup => {
                vec!["/SP-".to_string(), format!("/DIR={destination}")]
            }
            // NSIS requires `/D=` to be the final argument, unquoted, and without a
            // trailing separator. Quoting it makes the path part of the directory name.
            InstallerKind::Nsis => vec![format!("/D={}", destination.trim_end_matches('\\'))],
            InstallerKind::InstallShield | InstallerKind::Unknown => Vec::new(),
        }
    }
}

pub fn identify(path: &Path) -> CoreResult<InstallerKind> {
    // The markers sit in the stub near the start, but repacks can push them further in,
    // so a few megabytes are scanned rather than a fixed header.
    const SCAN_LIMIT: usize = 4 * 1024 * 1024;

    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; SCAN_LIMIT];
    let read = read_up_to(&mut file, &mut buffer)?;
    Ok(identify_bytes(&buffer[..read]))
}

fn read_up_to(file: &mut std::fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn identify_bytes(bytes: &[u8]) -> InstallerKind {
    // Checked before NSIS: an Inno installer can mention NSIS in bundled data, but not
    // the other way round.
    if contains(bytes, b"Inno Setup") || contains(bytes, b"JR.Inno.Setup") {
        return InstallerKind::InnoSetup;
    }
    if contains(bytes, b"Nullsoft") || contains(bytes, b"NullsoftInst") {
        return InstallerKind::Nsis;
    }
    if contains(bytes, b"InstallShield") {
        return InstallerKind::InstallShield;
    }
    InstallerKind::Unknown
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_inno_setup() {
        assert_eq!(
            identify_bytes(b"MZ\x90\x00 .... Inno Setup 6.2.0 ...."),
            InstallerKind::InnoSetup
        );
    }

    #[test]
    fn recognises_nsis() {
        assert_eq!(
            identify_bytes(b"MZ .... Nullsoft Install System v3.08"),
            InstallerKind::Nsis
        );
    }

    #[test]
    fn an_unmarked_program_is_unknown() {
        assert_eq!(
            identify_bytes(b"MZ\x90\x00 just a program"),
            InstallerKind::Unknown
        );
        assert_eq!(identify_bytes(b""), InstallerKind::Unknown);
    }

    #[test]
    fn inno_gets_a_dir_switch_and_skips_the_opening_prompt() {
        let args = InstallerKind::InnoSetup.destination_args("G:\\(78) Wall World");
        assert_eq!(args, vec!["/SP-", "/DIR=G:\\(78) Wall World"]);
    }

    #[test]
    fn nsis_gets_an_unquoted_trailing_switch() {
        // NSIS treats everything after /D= as the path, so it must come last and unquoted.
        let args = InstallerKind::Nsis.destination_args("G:\\(78) Wall World\\");
        assert_eq!(args, vec!["/D=G:\\(78) Wall World"]);
    }

    #[test]
    fn unknown_toolkits_are_not_driven() {
        assert!(InstallerKind::Unknown.destination_args("G:\\x").is_empty());
        // Recognised, but its switch varies too much by version to guess.
        assert!(InstallerKind::InstallShield
            .destination_args("G:\\x")
            .is_empty());
    }

    #[test]
    fn only_known_toolkits_run_unattended() {
        for kind in [InstallerKind::InnoSetup, InstallerKind::Nsis] {
            assert!(kind.runs_unattended() && !kind.default_unattended_args().is_empty());
        }
        for kind in [InstallerKind::InstallShield, InstallerKind::Unknown] {
            assert!(!kind.runs_unattended() && kind.default_unattended_args().is_empty());
        }
    }

    #[test]
    fn identifies_a_real_file() {
        let dir = std::env::temp_dir().join(format!("gameyfin-inst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("setup.exe");
        std::fs::write(&path, b"MZ padding padding Inno Setup Setup Data (6.2.0)").unwrap();

        assert_eq!(identify(&path).unwrap(), InstallerKind::InnoSetup);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
