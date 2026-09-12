//! Classifying a download (archive, Windows installer, native build, disc image) from its
//! leading bytes, since extensions are unreliable and the install step differs per kind.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::CoreResult;
use crate::extract::ArchiveKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    Archive(ArchiveKind),
    /// A Windows executable, almost always a setup program for a game library.
    WindowsExecutable,
    LinuxExecutable,
    /// A shell script, typically a GOG/itch Linux installer.
    ShellScript,
    DiskImage,
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
    // The unpacker's own detector, not a second copy of the signatures: the two disagreeing
    // would offer "Extract" for something that will not extract.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
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

    /// Whether this hands control to a setup wizard, which the user clicks through: the
    /// caller has to offer them the install path and cannot infer what they chose.
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
            // Most downloaded `.exe` files are installers, so that comes first, but plenty
            // are the game itself and need somewhere to go.
            if windows_host {
                vec![
                    InstallMethod::RunWindowsInstaller,
                    InstallMethod::CopyExecutable,
                ]
            } else {
                // On Linux a Windows installer still runs, through Proton, the same
                // mechanism used to run the game afterwards.
                vec![
                    InstallMethod::RunWindowsInstallerViaProton,
                    InstallMethod::CopyExecutable,
                ]
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
    fn classify_head_recognises_each_payload() {
        use ArchiveKind::*;
        for (head, expected) in [
            (b"PK\x03\x04ab".as_slice(), Payload::Archive(Zip)),
            (
                &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C],
                Payload::Archive(SevenZip),
            ),
            (b"Rar!\x1a\x07\x00", Payload::Archive(Rar)),
            // A game arriving as an installer, not an archive.
            (b"MZ\x90\x00", Payload::WindowsExecutable),
            (&[0x7F, b'E', b'L', b'F', 2], Payload::LinuxExecutable),
            (b"#!/bin/sh", Payload::ShellScript),
            // A shebang means an interpreter runs the file whatever follows it.
            (b"#!/bin/bash\x7FELF", Payload::ShellScript),
            (b"random!", Payload::Unknown),
            (b"", Payload::Unknown),
        ] {
            assert_eq!(classify_head(head), expected, "{head:?}");
        }
    }

    #[test]
    fn each_payload_offers_the_right_install_methods() {
        use InstallMethod::*;
        // `windows` is the host, not the payload: a Windows installer runs natively there
        // and through Proton elsewhere, and a Linux binary cannot be offered at all.
        for (payload, windows, expected) in [
            (Payload::Archive(ArchiveKind::Zip), false, vec![Extract]),
            (
                Payload::WindowsExecutable,
                true,
                vec![RunWindowsInstaller, CopyExecutable],
            ),
            (
                Payload::WindowsExecutable,
                false,
                vec![RunWindowsInstallerViaProton, CopyExecutable],
            ),
            (Payload::LinuxExecutable, true, vec![]),
            (Payload::LinuxExecutable, false, vec![CopyExecutable]),
            (Payload::DiskImage, false, vec![]),
            (Payload::Unknown, false, vec![]),
        ] {
            assert_eq!(
                methods_for(payload, windows),
                expected,
                "{payload:?} on windows={windows}"
            );
        }
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
