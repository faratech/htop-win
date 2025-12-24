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
pub fn install_to_path(force: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !is_admin() {
        // Re-launch with UAC elevation
        println!("Requesting administrator privileges...");
        let args = if force { "--install --force" } else { "--install" };
        elevate_with_args(args)?;
        println!("Elevated process launched. Check that window for results.");
        return Ok(());
    }

    // We're running as admin - do the installation
    let current_exe = std::env::current_exe()?;
    let current_version = env!("CARGO_PKG_VERSION");
    let target_path = get_install_path()?;

    // Check if already installed and compare versions (unless force)
    if target_path.exists() && !force {
        if let Some(installed_version) = get_installed_version() {
            if installed_version == current_version {
                println!("htop {} is already installed and up to date.", current_version);
                println!("Location: {}", target_path.display());
                println!("\nUse --force to reinstall anyway.");
                return Ok(());
            } else {
                println!("Updating htop from {} to {}...", installed_version, current_version);
            }
        } else {
            println!("Reinstalling htop {}...", current_version);
        }
    } else if force && target_path.exists() {
        println!("Force reinstalling htop {}...", current_version);
    } else {
        println!("Installing htop {} to PATH...", current_version);
    }

    // Copy the binary
    fs::copy(&current_exe, &target_path)?;

    println!("Successfully installed htop {}!", current_version);
    println!("Location: {}", target_path.display());
    println!("\nYou can now run 'htop' from any terminal.");
    Ok(())
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

/// GitHub repository for releases
const GITHUB_REPO: &str = "faratech/htop-win";

/// Get the latest version info from GitHub
/// Returns (version, download_url) or None if check fails
pub fn get_latest_release() -> Option<(String, String)> {
    // Detect architecture for correct binary
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };

    let ps_script = format!(
        r#"
        try {{
            $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/{}/releases/latest' -Headers @{{'User-Agent'='htop-win'}}
            $asset = $release.assets | Where-Object {{ $_.name -like '*{}.exe' }} | Select-Object -First 1
            if (-not $asset) {{ $asset = $release.assets | Where-Object {{ $_.name -like '*.exe' }} | Select-Object -First 1 }}
            Write-Output "$($release.tag_name)|$($asset.browser_download_url)"
        }} catch {{
            Write-Output "ERROR"
        }}
        "#,
        GITHUB_REPO, arch
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .output()
        .ok()?;

    let result = String::from_utf8_lossy(&output.stdout);
    let result = result.trim();

    if result == "ERROR" || result.is_empty() {
        return None;
    }

    let parts: Vec<&str> = result.splitn(2, '|').collect();
    if parts.len() != 2 {
        return None;
    }

    let version = parts[0].trim_start_matches('v').to_string();
    let download_url = parts[1].to_string();

    Some((version, download_url))
}

/// Download a file from URL to destination path using PowerShell
fn download_file(url: &str, dest: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let dest_str = dest.to_string_lossy();
    let ps_script = format!(
        r#"Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing"#,
        url, dest_str
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Download failed: {}", stderr).into())
    }
}

/// Clean up any leftover temp files from previous updates
fn cleanup_temp_files() {
    let temp_dir = std::env::temp_dir();
    let _ = fs::remove_file(temp_dir.join("htop-win-update.exe"));
    let _ = fs::remove_file(temp_dir.join("htop-win-update-path.txt"));
}

/// Update htop-win from GitHub releases
pub fn update_from_github(force: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Clean up any old temp files from previous failed updates
    cleanup_temp_files();

    println!("Checking for updates...");

    let (latest_version, download_url) = get_latest_release()
        .ok_or("Failed to check for updates. Check your internet connection.")?;

    let current_version = env!("CARGO_PKG_VERSION");

    if !force && !is_newer_version(&latest_version, current_version) {
        println!("htop {} is already the latest version.", current_version);
        println!("\nUse --force to reinstall anyway.");
        return Ok(());
    }

    if force && !is_newer_version(&latest_version, current_version) {
        println!("Force reinstalling htop {} from GitHub...", latest_version);
    } else {
        println!("New version available: {} -> {}", current_version, latest_version);
    }
    println!("Downloading from GitHub...");

    // Download to temp file
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("htop-win-update.exe");

    download_file(&download_url, &temp_file)?;

    println!("Download complete. Installing...");

    // Need admin to install to WindowsApps
    if !is_admin() {
        // Copy temp file path to a location the elevated process can access
        let update_marker = temp_dir.join("htop-win-update-path.txt");
        fs::write(&update_marker, temp_file.to_string_lossy().as_bytes())?;

        println!("Requesting administrator privileges...");
        elevate_with_args("--install-update")?;
        println!("Elevated process launched. Check that window for results.");
        return Ok(());
    }

    // We're admin - do the actual install
    do_install_update(&temp_file)
}

/// Install an update from a downloaded file (called from elevated process)
pub fn do_install_update(update_file: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let target_path = get_install_path()?;

    // Ensure parent directory exists
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // If target exists, use rename trick (Windows allows renaming running exe)
    if target_path.exists() {
        let backup_path = target_path.with_extension("exe.old");
        let _ = fs::remove_file(&backup_path); // Remove old backup if exists

        // Rename current exe to .old
        fs::rename(&target_path, &backup_path)?;

        // Copy new version
        if let Err(e) = fs::copy(update_file, &target_path) {
            // Failed - restore backup
            let _ = fs::rename(&backup_path, &target_path);
            return Err(e.into());
        }

        // Clean up backup
        let _ = fs::remove_file(&backup_path);
    } else {
        // No existing file, just copy
        fs::copy(update_file, &target_path)?;
    }

    // Clean up temp file
    let _ = fs::remove_file(update_file);

    // Get version of newly installed binary
    let version = get_installed_version().unwrap_or_else(|| "unknown".to_string());

    println!("Successfully updated to htop {}!", version);
    println!("Location: {}", target_path.display());
    println!("\nRestart htop to use the new version.");
    Ok(())
}

/// Complete an update installation (called when elevated with --install-update)
pub fn complete_update_install() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = std::env::temp_dir();
    let update_marker = temp_dir.join("htop-win-update-path.txt");

    let update_path = fs::read_to_string(&update_marker)?;
    let update_file = PathBuf::from(update_path.trim());

    // Clean up marker file
    let _ = fs::remove_file(&update_marker);

    if !update_file.exists() {
        return Err("Update file not found".into());
    }

    do_install_update(&update_file)
}

/// Update status for background updates
#[derive(Clone)]
pub enum UpdateStatus {
    /// A newer version is available and has been downloaded
    Downloaded { version: String, path: PathBuf },
    /// No update available or error occurred
    None,
}

/// Check for updates and download if available (for background auto-update)
/// Returns UpdateStatus indicating what happened
pub fn check_and_download_update() -> UpdateStatus {
    let current_version = env!("CARGO_PKG_VERSION");

    let (latest_version, download_url) = match get_latest_release() {
        Some(v) => v,
        None => return UpdateStatus::None,
    };

    if !is_newer_version(&latest_version, current_version) {
        return UpdateStatus::None;
    }

    // Download to temp file
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("htop-win-update.exe");

    // Clean up any old temp files first
    cleanup_temp_files();

    match download_file(&download_url, &temp_file) {
        Ok(()) => UpdateStatus::Downloaded {
            version: latest_version,
            path: temp_file,
        },
        Err(_) => UpdateStatus::None,
    }
}

/// Spawn a background thread to check and download updates
/// Returns a receiver that will receive the update status
pub fn spawn_update_check() -> std::sync::mpsc::Receiver<UpdateStatus> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        // Small delay to not slow down startup
        std::thread::sleep(std::time::Duration::from_secs(3));
        let result = check_and_download_update();
        let _ = tx.send(result);
    });

    rx
}

/// Check for and apply pending update on startup (call before UI starts)
/// Returns true if an update was applied (caller should continue normally)
pub fn apply_pending_update() -> bool {
    let temp_dir = std::env::temp_dir();
    let update_file = temp_dir.join("htop-win-update.exe");

    if !update_file.exists() {
        // Clean up any old backup files from previous updates
        let install_path = match get_install_path() {
            Ok(p) => p,
            Err(_) => return false,
        };
        let backup_path = install_path.with_extension("exe.old");
        let _ = fs::remove_file(&backup_path);
        return false;
    }

    let install_path = match get_install_path() {
        Ok(p) => p,
        Err(_) => {
            let _ = fs::remove_file(&update_file);
            return false;
        }
    };

    // If install path doesn't exist, just copy directly
    if !install_path.exists() {
        if fs::copy(&update_file, &install_path).is_ok() {
            let _ = fs::remove_file(&update_file);
            eprintln!("Update installed successfully!");
            return true;
        }
        return false;
    }

    // Rename current exe to .old (Windows allows renaming running exe)
    let backup_path = install_path.with_extension("exe.old");
    let _ = fs::remove_file(&backup_path); // Remove old backup if exists

    if fs::rename(&install_path, &backup_path).is_err() {
        // Can't rename, might not have permissions
        let _ = fs::remove_file(&update_file);
        return false;
    }

    // Copy new version to install location
    if fs::copy(&update_file, &install_path).is_err() {
        // Failed to copy, restore backup
        let _ = fs::rename(&backup_path, &install_path);
        let _ = fs::remove_file(&update_file);
        return false;
    }

    // Clean up
    let _ = fs::remove_file(&update_file);
    let _ = fs::remove_file(&backup_path);

    eprintln!("Update applied successfully!");
    true
}
