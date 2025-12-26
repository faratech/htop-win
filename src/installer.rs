//! Installation and update functionality for htop-win

use std::fs;
use std::path::PathBuf;

#[cfg(windows)]
use windows::core::{w, PCWSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_FLAG_SECURE,
    URL_COMPONENTS,
    WINHTTP_INTERNET_SCHEME_HTTPS,
    WINHTTP_OPEN_REQUEST_FLAGS,
};
#[cfg(windows)]
use windows::Win32::Foundation::GetLastError;

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
/// Installs to %LOCALAPPDATA%\Microsoft\WindowsApps which is user-writable and already in PATH
pub fn install_to_path(force: bool) -> Result<(), Box<dyn std::error::Error>> {
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
                return Ok(())
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

/// Helper struct to automatically close WinHTTP handles
#[cfg(windows)]
struct HandleGuard(*mut std::ffi::c_void);

#[cfg(windows)]
impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { let _ = WinHttpCloseHandle(self.0); }
        }
    }
}

/// Native HTTP GET using WinHTTP (no PowerShell, no extra deps)
#[cfg(windows)]
fn native_http_get(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::ffi::c_void;

    unsafe {
        // 1. Open Session
        let session = WinHttpOpen(
            w!("htop-win-updater/1.0"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            None,
            None,
            0,
        );
        if session.is_null() {
            return Err(format!("WinHttpOpen failed: {:?}", GetLastError()).into());
        }
        let _session_guard = HandleGuard(session);

        // 2. Crack URL
        let mut host_name = vec![0u16; 256];
        let mut url_path = vec![0u16; 2048];
        
        let url_wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let mut components = URL_COMPONENTS::default();
        components.dwStructSize = std::mem::size_of::<URL_COMPONENTS>() as u32;
        components.dwHostNameLength = host_name.len() as u32;
        components.lpszHostName = PWSTR(host_name.as_mut_ptr());
        components.dwUrlPathLength = url_path.len() as u32;
        components.lpszUrlPath = PWSTR(url_path.as_mut_ptr());

        if WinHttpCrackUrl(&url_wide, 0, &mut components).is_err() {
             return Err(format!("WinHttpCrackUrl failed: {:?}", GetLastError()).into());
        }

        // 3. Connect
        let connect = WinHttpConnect(
            session,
            PCWSTR(components.lpszHostName.0),
            components.nPort as u16,
            0,
        );
        if connect.is_null() {
            return Err(format!("WinHttpConnect failed: {:?}", GetLastError()).into());
        }
        let _connect_guard = HandleGuard(connect);

        // 4. Open Request
        let flags = if components.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let request = WinHttpOpenRequest(
            connect,
            w!("GET"),
            PCWSTR(components.lpszUrlPath.0),
            None,
            None,
            std::ptr::null(), // Accept types
            flags,
        );
        if request.is_null() {
            return Err(format!("WinHttpOpenRequest failed: {:?}", GetLastError()).into());
        }
        let _request_guard = HandleGuard(request);

        // 5. Send Request
        if WinHttpSendRequest(
            request,
            None,
            None,
            0,
            0,
            0,
        ).is_err() {
            return Err(format!("WinHttpSendRequest failed: {:?}", GetLastError()).into());
        }

        // 6. Receive Response
        if WinHttpReceiveResponse(request, std::ptr::null_mut()).is_err() {
            return Err(format!("WinHttpReceiveResponse failed: {:?}", GetLastError()).into());
        }

        // 7. Read Data
        let mut body = Vec::new();
        let mut buffer = vec![0u8; 8192];
        let mut bytes_read = 0;

        loop {
            if WinHttpQueryDataAvailable(request, &mut bytes_read).is_err() {
                break;
            }
            if bytes_read == 0 {
                break;
            }
            
            let to_read = bytes_read.min(buffer.len() as u32);
            let mut read_now = 0;
            
            if WinHttpReadData(
                request,
                buffer.as_mut_ptr() as *mut c_void,
                to_read,
                &mut read_now,
            ).is_err() {
                break;
            }
            
            if read_now == 0 {
                break;
            }
            
            body.extend_from_slice(&buffer[..read_now as usize]);
        }

        Ok(body)
    }
}

// Fallback for non-windows (though we really only target windows)
#[cfg(not(windows))]
fn native_http_get(_url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Err("Not supported on non-Windows".into())
}

/// GitHub repository for releases
const GITHUB_REPO: &str = "faratech/htop-win";

/// Get the latest version info from GitHub
/// Returns (version, download_url) or None if check fails
pub fn get_latest_release() -> Option<(String, String)> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);
    
    // Fetch JSON from GitHub API
    let body = native_http_get(&url).ok()?;
    let json_text = String::from_utf8(body).ok()?;
    
    // Parse JSON manually to avoid complex deps
    // We look for "tag_name": "vX.Y.Z"
    let version = json_text.split("\"tag_name\"")
        .nth(1)?
        .split(':')
        .nth(1)?
        .split("\"")
        .nth(1)?
        .trim_start_matches('v')
        .to_string();

    // Detect architecture
    let target_arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };
    let target_suffix = format!("htop-win-{}.exe", target_arch);

    // Find asset URL
    // Look for "browser_download_url": "..." that ends with target_suffix
    let mut download_url = String::new();
    for part in json_text.split("\"browser_download_url\"") {
        if let Some(url_part) = part.split(':').nth(1) {
            if let Some(url) = url_part.split("\"").nth(1) {
                if url.ends_with(&target_suffix) {
                    download_url = url.to_string();
                    break;
                }
            }
        }
    }

    // Fallback: if specific arch not found, try any .exe
    if download_url.is_empty() {
        for part in json_text.split("\"browser_download_url\"") {
            if let Some(url_part) = part.split(':').nth(1) {
                if let Some(url) = url_part.split("\"").nth(1) {
                    if url.ends_with(".exe") {
                        download_url = url.to_string();
                        break;
                    }
                }
            }
        }
    }

    if version.is_empty() || download_url.is_empty() {
        return None;
    }

    Some((version, download_url))
}

/// Clean up any leftover temp files from previous updates
fn cleanup_temp_files() {
    let temp_dir = std::env::temp_dir();
    let _ = fs::remove_file(temp_dir.join("htop-win-update.exe"));
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
        return Ok(())
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

    let body = native_http_get(&download_url)?;
    if body.is_empty() {
        return Err("Downloaded update file is empty".into());
    }
    fs::write(&temp_file, body)?;

    println!("Download complete. Installing...");

    // Install directly - %LOCALAPPDATA%\Microsoft\WindowsApps is user-writable
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

        // Clean up backup - ignore errors as running process might lock it
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
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("htop-win-update.exe");

    // If update already downloaded and pending, don't re-download
    if temp_file.exists() {
        // Verify it's not empty
        if let Ok(metadata) = fs::metadata(&temp_file) {
            if metadata.len() > 0 {
                return UpdateStatus::None;
            }
        }
        // Invalid file, remove it
        let _ = fs::remove_file(&temp_file);
    }

    let current_version = env!("CARGO_PKG_VERSION");

    let (latest_version, download_url) = match get_latest_release() {
        Some(v) => v,
        None => return UpdateStatus::None,
    };

    if !is_newer_version(&latest_version, current_version) {
        return UpdateStatus::None;
    }

    match native_http_get(&download_url) {
        Ok(body) if !body.is_empty() => {
            if fs::write(&temp_file, body).is_ok() {
                UpdateStatus::Downloaded {
                    version: latest_version,
                    path: temp_file,
                }
            } else {
                UpdateStatus::None
            }
        },
        _ => UpdateStatus::None,
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

    // Get the currently running executable - this is what we need to update
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };

    if !update_file.exists() {
        // Clean up any old backup files from previous updates
        let backup_path = current_exe.with_extension("exe.old");
        // Only remove backup if it's not the running file (unlikely but safe)
        let _ = fs::remove_file(&backup_path);
        return false;
    }

    // Verify update file integrity
    if let Ok(metadata) = fs::metadata(&update_file) {
        if metadata.len() == 0 {
            let _ = fs::remove_file(&update_file);
            return false;
        }
    } else {
        return false;
    }

    let install_path = current_exe;

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

    if let Err(e) = fs::rename(&install_path, &backup_path) {
        // Can't rename - keep update file for retry on next restart
        eprintln!("Update pending (cannot rename running exe: {})", e);
        return true; // Return true to skip re-download
    }

    // Copy new version to install location
    if let Err(e) = fs::copy(&update_file, &install_path) {
        // Failed to copy, restore backup
        eprintln!("Update failed (copy error: {}), restoring backup", e);
        let _ = fs::rename(&backup_path, &install_path);
        // Keep update file for retry
        return true; // Return true to skip re-download
    }

    // Clean up update file ONLY on success
    let _ = fs::remove_file(&update_file);
    
    // Try to remove backup, but ignore error if locked (it's the running executable)
    let _ = fs::remove_file(&backup_path);

    eprintln!("Update applied successfully!");
    true
}