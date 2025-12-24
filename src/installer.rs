//! Installation and update functionality for htop-win

use std::fs;
use std::path::PathBuf;

#[cfg(windows)]
use windows::core::{PCWSTR, w};
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
#[cfg(windows)]
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
#[cfg(windows)]
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
#[cfg(windows)]
use windows::Win32::UI::Shell::ShellExecuteW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Check if running as administrator
#[cfg(windows)]
pub fn is_admin() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut size = 0u32;
            let result = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            );
            let _ = CloseHandle(token);
            result.is_ok() && elevation.TokenIsElevated != 0
        } else {
            false
        }
    }
}

#[cfg(not(windows))]
pub fn is_admin() -> bool {
    false
}

/// Re-launch the current process with UAC elevation
#[cfg(windows)]
pub fn elevate_with_args(args: &str) -> Result<(), Box<dyn std::error::Error>> {
    let exe_path = std::env::current_exe()?;
    let exe_path_wide: Vec<u16> = exe_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let args_wide: Vec<u16> = format!("{}\0", args).encode_utf16().collect();

    let result = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            w!("runas"),
            PCWSTR(exe_path_wide.as_ptr()),
            PCWSTR(args_wide.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns > 32 on success
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err("Failed to elevate privileges".into())
    }
}

#[cfg(not(windows))]
pub fn elevate_with_args(_args: &str) -> Result<(), Box<dyn std::error::Error>> {
    Err("UAC elevation is only supported on Windows".into())
}

/// Get the installation path for htop
pub fn get_install_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let local_app_data = std::env::var("LOCALAPPDATA")?;
    Ok(PathBuf::from(&local_app_data)
        .join("Microsoft")
        .join("WindowsApps")
        .join("htop.exe"))
}

/// Get version of installed htop (if any)
pub fn get_installed_version() -> Option<String> {
    let install_path = get_install_path().ok()?;
    if !install_path.exists() {
        return None;
    }

    let output = std::process::Command::new(&install_path)
        .arg("--version")
        .output()
        .ok()?;

    let version_output = String::from_utf8_lossy(&output.stdout);
    // Parse "htop-win X.Y.Z" to get version
    version_output
        .split_whitespace()
        .last()
        .map(|s| s.to_string())
}

/// Install htop-win to a PATH directory so it can be run from anywhere
pub fn install_to_path() -> Result<(), Box<dyn std::error::Error>> {
    if !is_admin() {
        // Re-launch with UAC elevation
        println!("Requesting administrator privileges...");
        elevate_with_args("--install")?;
        println!("Elevated process launched. Check that window for results.");
        return Ok(());
    }

    // We're running as admin - do the installation
    let current_exe = std::env::current_exe()?;
    let current_version = env!("CARGO_PKG_VERSION");
    let target_path = get_install_path()?;

    // Check if already installed and compare versions
    if target_path.exists() {
        if let Some(installed_version) = get_installed_version() {
            if installed_version == current_version {
                println!("htop {} is already installed and up to date.", current_version);
                println!("Location: {}", target_path.display());
                wait_for_key();
                return Ok(());
            } else {
                println!("Updating htop from {} to {}...", installed_version, current_version);
            }
        } else {
            println!("Reinstalling htop {}...", current_version);
        }
    } else {
        println!("Installing htop {} to PATH...", current_version);
    }

    // Copy the binary
    fs::copy(&current_exe, &target_path)?;

    println!("Successfully installed htop {}!", current_version);
    println!("Location: {}", target_path.display());
    println!("\nYou can now run 'htop' from any terminal.");
    wait_for_key();
    Ok(())
}

/// Wait for user to press any key (used in elevated console windows)
pub fn wait_for_key() {
    use crossterm::event::{self, Event, KeyEventKind};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    println!("\nPress any key to close...");

    // Use crossterm's event reading - works properly in elevated windows
    if enable_raw_mode().is_ok() {
        loop {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    break;
                }
            }
        }
        let _ = disable_raw_mode();
    }
}

/// Parse version string to comparable tuple
fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = version.trim_start_matches('v').split('.').collect();
    if parts.len() >= 3 {
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    } else {
        None
    }
}

/// Compare two version strings, returns true if `a` is newer than `b`
pub fn is_newer_version(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Some(va), Some(vb)) => va > vb,
        _ => false,
    }
}

/// Check for updates from GitHub releases
/// Returns Some((version, download_url)) if a newer version is available
pub fn check_for_update() -> Option<(String, String)> {
    // Use minimal HTTP request to GitHub API
    // For now, return None - full implementation would need HTTP client
    // TODO: Implement GitHub release check
    None
}
