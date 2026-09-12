//! Turning the way a game dies into something a user can act on. A game that exits in four
//! seconds says why in its output, and an exit code alone never does.

/// Advice for a known failure. Most specific first: container and Vulkan failures also
/// mention Direct3D, and those are the things to fix.
pub fn explain(output: &str) -> Option<&'static str> {
    let lower = output.to_lowercase();
    let has = |needles: &[&str]| needles.iter().all(|n| lower.contains(n));

    // Nothing inside the Steam Runtime container ran, so the rest of the output is noise.
    if has(&["setting up uid map"]) || has(&["requires user namespaces"]) {
        return Some(
            "Proton's container could not start because this system blocks user \
             namespaces. Ubuntu 23.10 and newer do this through AppArmor; allowing \
             bubblewrap fixes it. Until then, set this game to run with Wine in its options.",
        );
    }

    if has(&["_v2-entry-point"]) || has(&["runtime platform missing"]) {
        return Some(
            "The Steam Runtime download is incomplete. Delete the folder \
             ~/.local/share/umu/steamrt3 and start the game again to download it fresh.",
        );
    }

    // The container could not build its 32-bit half, so no 32-bit program will start.
    if has(&["cannot determine ld.so for i386"]) {
        return Some(
            "This program is 32-bit, and the Steam Runtime container here has no 32-bit \
             libraries. In the Flatpak, install the runtime's i386 compatibility extension; \
             on a system install, install your distribution's 32-bit glibc. Setting this \
             game to run with Wine in its options also works: its WoW64 build needs none.",
        );
    }

    // Said of the program itself, so the cause is whatever it needed, not the file.
    if has(&["c0000135"]) {
        return Some(
            "Windows could not start the program: something it needs is missing. A 32-bit \
             program needs 32-bit libraries, which the Steam Runtime container only has when \
             the system does, so setting this game to run with Wine in its options often \
             gets past it.",
        );
    }

    if has(&["env: 'python3'"]) || has(&["python3: no such file"]) {
        return Some(
            "Running games through Proton needs python3 3.10 or newer. Install python3 from \
             your distribution, or set this game to run with Wine in its options.",
        );
    }

    if has(&["vulkan-1.dll"]) || has(&["failed to load", "vulkan"]) || has(&["no vulkan driver"]) {
        return Some(
            "This game needs Vulkan and could not find it. Install your distribution's \
             Vulkan driver for your graphics card, then try again.",
        );
    }

    if has(&["directx 12 is not supported"]) || has(&["d3d12", "not supported"]) {
        return Some(
            "DirectX 12 could not start. It needs a graphics driver with Vulkan 1.3, so \
             update yours or run the game with its DirectX 11 option. A game on Wine also \
             needs vkd3d-proton, under Settings, Compatibility.",
        );
    }

    if has(&["d3d11", "device"]) && has(&["fail"]) || has(&["failed to create", "direct3d"]) {
        return Some(
            "The game could not create a Direct3D device. Update your graphics driver. A \
             game on Wine also needs DXVK, under Settings, Compatibility.",
        );
    }

    if has(&["err:vkd3d"]) || has(&["err:dxgi"]) {
        return Some(
            "The Direct3D translation layer reported an error. Deleting this game's \
             prefix under Settings, Compatibility rebuilds it with a fresh copy.",
        );
    }

    if has(&["err:module", "could not load"]) || has(&["is missing", ".dll"]) {
        return Some(
            "The game is missing a Windows library. Its installer may not have finished, \
             so reinstalling it is usually the fix.",
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_that_started_all_this_is_recognised() {
        let advice =
            explain("DirectX 12 is not supported on your system. Try running without the -dx12")
                .unwrap();
        assert!(advice.contains("vkd3d-proton"), "{advice}");
    }

    #[test]
    fn a_missing_vulkan_loader_wins_over_the_direct3d_message_it_also_prints() {
        // A game with no Vulkan prints both, and installing a driver is the real fix.
        let output = "err:module:import_dll Library vulkan-1.dll not found\n\
                      DirectX 12 is not supported on your system.";
        assert!(explain(output).unwrap().contains("Vulkan driver"));
    }

    #[test]
    fn a_direct3d_device_failure_points_at_dxvk() {
        let advice = explain("Failed to create D3D11 device and swapchain").unwrap();
        assert!(advice.contains("DXVK"), "{advice}");
    }

    #[test]
    fn a_container_blocked_by_apparmor_wins_over_everything_it_also_prints() {
        let output = "bwrap: setting up uid map: Permission denied\n\
                      DirectX 12 is not supported on your system.";
        assert!(explain(output).unwrap().contains("user namespaces"));
    }

    #[test]
    fn an_incomplete_steam_runtime_download_names_the_folder_to_delete() {
        let output =
            "ERROR: _v2-entry-point (umu) cannot be found in '/home/u/.local/share/umu/steamrt3'";
        assert!(explain(output).unwrap().contains("steamrt3"));
    }

    #[test]
    fn a_missing_python_points_at_python_rather_than_the_game() {
        let output = "/usr/bin/env: 'python3': No such file or directory";
        assert!(explain(output).unwrap().contains("python3"));
    }

    #[test]
    fn a_program_that_cannot_start_is_not_blamed_on_one_cause_alone() {
        // `c0000135` only says something was missing, not that the program is 32-bit.
        let advice = explain(r#"wine: failed to open "R:\setup.exe": c0000135"#).unwrap();
        assert!(advice.contains("something it needs is missing"), "{advice}");
    }

    #[test]
    fn a_32_bit_program_without_32_bit_libraries_is_recognised() {
        let container = "pressure-vessel-wrap[94]: W: Cannot determine ld.so for \
                         i386-linux-gnu: Failed to execute child process";
        assert!(explain(container).unwrap().contains("32-bit"));
    }

    #[test]
    fn ordinary_output_is_left_alone() {
        assert_eq!(explain(""), None);
        assert_eq!(explain("Loading assets\nStarting up\nGoodbye"), None);
    }

    #[test]
    fn the_match_ignores_case_because_engines_disagree_on_it() {
        assert!(explain("DIRECTX 12 IS NOT SUPPORTED ON YOUR SYSTEM").is_some());
    }
}
