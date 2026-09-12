//! JSON files the app owns: written atomically so a crash or a racing writer never leaves
//! half a file, which the loaders would otherwise read as "start from scratch".

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Reads a JSON file. `Ok(None)` only when it does not exist.
pub async fn read_json<T: DeserializeOwned>(path: &Path) -> std::io::Result<Option<T>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads a JSON file, logging and defaulting when it is missing or unreadable.
pub async fn read_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    match read_json(path).await {
        Ok(value) => value.unwrap_or_default(),
        Err(e) => {
            tracing::warn!("ignoring unreadable {path:?}: {e}");
            T::default()
        }
    }
}

/// Writes JSON through a sibling temp file and a rename. `private` limits it to the owner.
pub async fn write_json<T: Serialize>(
    path: &Path,
    value: &T,
    private: bool,
) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || write_atomic(&path, &json, private))
        .await
        .map_err(std::io::Error::other)?
}

fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Created owner-only, so the secrets are never readable under the umask, not even briefly.
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let result = (|| {
        let mut file = options.open(&temp)?;
        #[cfg(windows)]
        if private {
            restrict_to_owner(&temp);
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Narrows a file's DACL to its owner and SYSTEM. Best effort: a failure is logged.
#[cfg(windows)]
fn restrict_to_owner(path: &Path) {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    let sddl: Vec<u16> = "D:PAI(A;;FA;;;OW)(A;;FA;;;SY)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: the SDDL string is NUL-terminated, and `descriptor` is only read after the
    // call reports success. It is released with LocalFree below, as the API requires.
    let built = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if built == 0 || descriptor.is_null() {
        tracing::warn!("could not build owner-only access rules");
        return;
    }

    let mut present: i32 = 0;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut defaulted: i32 = 0;
    // SAFETY: `descriptor` is a valid security descriptor built from SDDL above, so its
    // DACL pointer can be read out.
    let read =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) };

    let result = if read != 0 && present != 0 {
        // SAFETY: `wide` is NUL-terminated and `dacl` points into `descriptor`, which is
        // still alive until the LocalFree below.
        unsafe {
            SetNamedSecurityInfoW(
                wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            )
        }
    } else {
        ERROR_SUCCESS
    };

    // SAFETY: `descriptor` came from a Local* allocation and is not used afterwards.
    unsafe { LocalFree(descriptor as _) };

    if result != ERROR_SUCCESS {
        tracing::warn!(code = result, "could not restrict {path:?} to its owner");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-persist-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn round_trips_and_leaves_no_temp_files() {
        let dir = scratch("roundtrip");
        let path = dir.join("value.json");
        write_json(&path, &vec![1, 2, 3], true).await.unwrap();
        write_json(&path, &vec![4], true).await.unwrap();

        assert_eq!(read_json::<Vec<i32>>(&path).await.unwrap(), Some(vec![4]));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn missing_is_none_and_corrupt_is_an_error() {
        let dir = scratch("corrupt");
        let path = dir.join("value.json");
        assert_eq!(read_json::<Vec<i32>>(&path).await.unwrap(), None);

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(read_json::<Vec<i32>>(&path).await.is_err());
        assert!(read_json_or_default::<Vec<i32>>(&path).await.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn concurrent_writers_never_leave_a_torn_file() {
        let dir = scratch("race");
        let path = dir.join("value.json");
        let writers = (0..16).map(|i| {
            let path = path.clone();
            tokio::spawn(async move { write_json(&path, &vec![i; 5000], false).await })
        });
        for writer in writers {
            writer.await.unwrap().unwrap();
        }
        let value: Vec<i32> = read_json(&path).await.unwrap().unwrap();
        assert!(value.iter().all(|v| *v == value[0]));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
