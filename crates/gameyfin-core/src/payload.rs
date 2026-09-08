//! Classifying a download (archive, Windows installer, native build, disc image) from its
//! leading bytes, since extensions are unreliable and the install step differs per kind.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::CoreResult;
use crate::extract::ArchiveKind;

/// What a downloaded file contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    /// An archive to unpack.
    Archive(ArchiveKind),
    /// A Windows executable, almost always a setup program for a game library.
    WindowsExecutable,
    /// A native Linux executable.
    LinuxExecutable,
    /// A shell script, typically a GOG/itch Linux installer.
    ShellScript,
    /// A disc image.
    DiskImage,
    /// Nothing recognisable.
    Unknown,
}

impl Payload {
    pub fn label(self) -> &'static str {
        match self {
            Payload::Archive(kind) => kind.label(),
            Payload::WindowsExecutable => "Windows program",
            Payload::LinuxExecutable => "Linux program",
            Payload::ShellScript => "shell script",
            Payload::DiskImage => "disc image",
            Payload::Unknown => "unrecognised file",
        }
    }

    pub fn is_archive(self) -> bool {
        matches!(self, Payload::Archive(_))
    }
}

/// Classify a downloaded file.
pub fn classify(path: &Path) -> CoreResult<Payload> {
    let mut file = File::open(path)?;
    let mut head = [0u8; 8];
    let read = file.read(&mut head)?;
    let head = &head[..read];

    // ISO 9660 stores its identifier well into the file, so it needs a separate look.
    if is_iso9660(&mut file)? {
        return Ok(Payload::DiskImage);
    }

    let payload = classify_head(head);
    if payload != Payload::Unknown {
        return Ok(payload);
    }

    // An uncompressed tar carries no signature at offset zero either, so it only turns up
    // on a second look. Checked last, for the same reason as the ISO: it costs a seek.
    match crate::extract::detect(path)? {
        ArchiveKind::None => Ok(Payload::Unknown),
        kind => Ok(Payload::Archive(kind)),
    }
}

fn classify_head(head: &[u8]) -> Payload {
    // Archives are identified by the unpacker's own detector rather than a second copy of
    // the same signatures. The install step is chosen here and carried out there, so the
    // two disagreeing means offering "Extract" for something that will not extract, which
    // is how a `.tar.gz` download came to be treated as an unrecognised file.
    let archive = crate::extract::detect_bytes(head);
    if archive != ArchiveKind::None {
        return Payload::Archive(archive);
    }
    // A shebang is checked before ELF because a script can be anything underneath.
    if head.starts_with(b"#!") {
        return Payload::ShellScript;
    }
    if head.starts_with(&[0x7F, b'E', b'L', b'F']) {
        return Payload::LinuxExecutable;
    }
    // DOS/PE header. Self-extracting archives look like this too, which is why they are
    // treated as programs to run rather than archives to unpack.
    if head.starts_with(b"MZ") {
        return Payload::WindowsExecutable;
    }
    Payload::Unknown
}

/// ISO 9660 puts "CD001" at offset 0x8001, in the primary volume descriptor.
fn is_iso9660(file: &mut File) -> CoreResult<bool> {
    if file.seek(SeekFrom::End(0))? < 0x8006 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0x8001))?;
    let mut marker = [0u8; 5];
    let read = file.read(&mut marker)?;
    file.rewind()?;
    Ok(&marker[..read] == b"CD001")
}

/// A way of installing a downloaded game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// Unpack the archive into the install directory.
    Extract,
    /// Run a Windows installer natively (Windows only).
    RunWindowsInstaller,
    /// Run a Windows installer through Proton (Linux).
    RunWindowsInstallerViaProton,
    /// Copy the file into place and mark it executable.
    CopyExecutable,
}

impl InstallMethod {
    pub fn key(self) -> &'static str {
        match self {
            InstallMethod::Extract => "extract",
            InstallMethod::RunWindowsInstaller => "run-installer",
            InstallMethod::RunWindowsInstallerViaProton => "run-installer-proton",
            InstallMethod::CopyExecutable => "copy-executable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "extract" => Some(InstallMethod::Extract),
            "run-installer" => Some(InstallMethod::RunWindowsInstaller),
            "run-installer-proton" => Some(InstallMethod::RunWindowsInstallerViaProton),
            "copy-executable" => Some(InstallMethod::CopyExecutable),
            _ => None,
        }
    }

    /// Whether this method hands control to a third-party installer.
    ///
    /// Those cannot be driven headlessly, the user clicks through the setup wizard, so
    /// the caller has to tell them where to install and cannot infer the result.
    pub fn is_interactive(self) -> bool {
        matches!(
            self,
            InstallMethod::RunWindowsInstaller | InstallMethod::RunWindowsInstallerViaProton
        )
    }
}

/// The install methods that make sense for a payload on this platform, best first.
pub fn methods_for(payload: Payload, windows_host: bool) -> Vec<InstallMethod> {
    match payload {
        Payload::Archive(_) => vec![InstallMethod::Extract],
        Payload::WindowsExecutable => {
            if windows_host {
                vec![InstallMethod::RunWindowsInstaller]
            } else {
                // On Linux a Windows installer still runs, through Proton, the same
                // mechanism used to run the game afterwards.
                vec![InstallMethod::RunWindowsInstallerViaProton]
            }
        }
        Payload::LinuxExecutable | Payload::ShellScript => {
            if windows_host {
                Vec::new()
            } else {
                vec![InstallMethod::CopyExecutable]
            }
        }
        // Mounting a disc image is not supported yet; offering nothing is clearer than
        // offering something that will fail.
        Payload::DiskImage | Payload::Unknown => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_archives() {
        assert_eq!(
            classify_head(b"PK\x03\x04ab"),
            Payload::Archive(ArchiveKind::Zip)
        );
        assert_eq!(
            classify_head(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]),
            Payload::Archive(ArchiveKind::SevenZip)
        );
    }

    #[test]
    fn recognises_a_rar_archive() {
        assert_eq!(
            classify_head(b"Rar!\x1a\x07\x00"),
            Payload::Archive(ArchiveKind::Rar)
        );
    }

    #[test]
    fn recognises_a_windows_program() {
        // The case from the field: a game that arrives as an installer, not an archive.
        assert_eq!(classify_head(b"MZ\x90\x00"), Payload::WindowsExecutable);
    }

    #[test]
    fn recognises_native_linux_payloads() {
        assert_eq!(
            classify_head(&[0x7F, b'E', b'L', b'F', 2]),
            Payload::LinuxExecutable
        );
        assert_eq!(classify_head(b"#!/bin/sh"), Payload::ShellScript);
    }

    #[test]
    fn a_script_wins_over_anything_below_it() {
        // A shebang means the file is run by an interpreter whatever follows.
        assert_eq!(classify_head(b"#!/bin/bash\x7FELF"), Payload::ShellScript);
    }

    #[test]
    fn unknown_content_is_reported_as_such() {
        assert_eq!(classify_head(b"random!"), Payload::Unknown);
        assert_eq!(classify_head(b""), Payload::Unknown);
    }

    #[test]
    fn archives_are_extracted() {
        assert_eq!(
            methods_for(Payload::Archive(ArchiveKind::Zip), false),
            vec![InstallMethod::Extract]
        );
    }

    #[test]
    fn a_windows_installer_runs_natively_on_windows_and_via_proton_elsewhere() {
        assert_eq!(
            methods_for(Payload::WindowsExecutable, true),
            vec![InstallMethod::RunWindowsInstaller]
        );
        assert_eq!(
            methods_for(Payload::WindowsExecutable, false),
            vec![InstallMethod::RunWindowsInstallerViaProton]
        );
    }

    #[test]
    fn a_linux_binary_is_not_offered_on_windows() {
        assert!(methods_for(Payload::LinuxExecutable, true).is_empty());
        assert_eq!(
            methods_for(Payload::LinuxExecutable, false),
            vec![InstallMethod::CopyExecutable]
        );
    }

    #[test]
    fn unsupported_payloads_offer_nothing() {
        assert!(methods_for(Payload::DiskImage, false).is_empty());
        assert!(methods_for(Payload::Unknown, false).is_empty());
    }

    #[test]
    fn installer_methods_are_interactive() {
        assert!(InstallMethod::RunWindowsInstaller.is_interactive());
        assert!(InstallMethod::RunWindowsInstallerViaProton.is_interactive());
        assert!(!InstallMethod::Extract.is_interactive());
    }

    #[test]
    fn method_keys_round_trip() {
        for method in [
            InstallMethod::Extract,
            InstallMethod::RunWindowsInstaller,
            InstallMethod::RunWindowsInstallerViaProton,
            InstallMethod::CopyExecutable,
        ] {
            assert_eq!(InstallMethod::from_key(method.key()), Some(method));
        }
        assert_eq!(InstallMethod::from_key("nonsense"), None);
    }

    #[test]
    fn detects_an_iso_by_its_volume_descriptor() {
        let dir = std::env::temp_dir().join(format!("gameyfin-iso-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.iso");

        let mut data = vec![0u8; 0x8006];
        data[0x8001..0x8006].copy_from_slice(b"CD001");
        std::fs::write(&path, &data).unwrap();

        assert_eq!(classify(&path).unwrap(), Payload::DiskImage);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
