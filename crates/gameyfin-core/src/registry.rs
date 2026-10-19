//! The few registry values Windows-only features read and write.

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::HKEY;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub(crate) fn string(root: HKEY, key: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};

    let (key, value) = (wide(key), wide(value));
    let mut size = 0u32;
    // SAFETY: both names are NUL-terminated, and a null buffer asks only for the size.
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: `size` is the byte length of `buffer`, which the call writes no further than.
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}

/// Writes a string value, creating the key if needed.
pub(crate) fn set_string(root: HKEY, key: &str, value: &str, data: &str) -> bool {
    use windows_sys::Win32::System::Registry::{RegSetKeyValueW, REG_SZ};

    let (key, value, data) = (wide(key), wide(value), wide(data));
    // SAFETY: the names are NUL-terminated and the byte count covers `data` and its NUL.
    let status = unsafe {
        RegSetKeyValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    status == ERROR_SUCCESS
}

pub(crate) fn delete_value(root: HKEY, key: &str, value: &str) -> bool {
    use windows_sys::Win32::System::Registry::RegDeleteKeyValueW;

    let (key, value) = (wide(key), wide(value));
    // SAFETY: both names are NUL-terminated.
    let status = unsafe { RegDeleteKeyValueW(root, key.as_ptr(), value.as_ptr()) };
    status == ERROR_SUCCESS
}
