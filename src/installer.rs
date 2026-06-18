//! Installation and update functionality for htop-win

use std::fs;
use std::path::PathBuf;

#[cfg(windows)]
use windows::Win32::Foundation::GetLastError;
#[cfg(windows)]
use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest,
};
#[cfg(windows)]
use windows::core::{PCWSTR, PWSTR, w};

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
                println!(
                    "htop {} is already installed and up to date.",
                    current_version
                );
                println!("Location: {}", target_path.display());
                println!("\nUse --force to reinstall anyway.");
                return Ok(());
            } else {
                println!(
                    "Updating htop from {} to {}...",
                    installed_version, current_version
                );
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

/// Minimum plausible size for a release binary (real ones are ~600-800 KB).
const MIN_UPDATE_SIZE: usize = 100 * 1024;

/// Reject anything that is not plausibly a Windows PE executable, so a CDN
/// error page or truncated download is never installed over the working exe.
/// (Transport integrity comes from WinHTTP's TLS; if stronger guarantees are
/// ever wanted, Authenticode via WinVerifyTrust — zero new deps — is the next
/// step, not a hand-rolled checksum.)
fn validate_pe_executable(body: &[u8]) -> Result<(), String> {
    if body.len() < MIN_UPDATE_SIZE {
        return Err(format!(
            "file too small ({} bytes) to be htop-win",
            body.len()
        ));
    }
    if &body[0..2] != b"MZ" {
        return Err("missing MZ header (not a Windows executable)".into());
    }
    // The PE signature lives at the offset stored in e_lfanew (u32 LE at 0x3C)
    let e_lfanew = u32::from_le_bytes([body[0x3c], body[0x3d], body[0x3e], body[0x3f]]) as usize;
    match body.get(e_lfanew..e_lfanew + 6) {
        Some(sig) if &sig[..4] == b"PE\0\0" => Ok(()),
        _ => Err("missing PE signature".into()),
    }
}

fn pe_machine(body: &[u8]) -> Result<u16, String> {
    validate_pe_executable(body)?;
    let e_lfanew = u32::from_le_bytes([body[0x3c], body[0x3d], body[0x3e], body[0x3f]]) as usize;
    let machine = body
        .get(e_lfanew + 4..e_lfanew + 6)
        .ok_or_else(|| "missing PE machine field".to_string())?;
    Ok(u16::from_le_bytes([machine[0], machine[1]]))
}

fn target_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    }
}

fn target_machine() -> u16 {
    if cfg!(target_arch = "aarch64") {
        0xAA64
    } else {
        0x8664
    }
}

fn validate_target_pe_executable(body: &[u8]) -> Result<(), String> {
    let machine = pe_machine(body)?;
    if machine == target_machine() {
        Ok(())
    } else {
        Err(format!(
            "wrong architecture: PE machine {machine:#06x}, expected {}",
            target_arch()
        ))
    }
}

fn update_meta_path() -> PathBuf {
    std::env::temp_dir().join("htop-win-update.meta")
}

fn write_update_metadata(version: &str) {
    let _ = fs::write(
        update_meta_path(),
        format!("version={version}\narch={}\n", target_arch()),
    );
}

fn pending_metadata_matches(expected_version: &str) -> bool {
    pending_metadata_version()
        .as_deref()
        .is_some_and(|version| version == expected_version)
}

fn pending_metadata_version() -> Option<String> {
    let Ok(meta) = fs::read_to_string(update_meta_path()) else {
        return None;
    };
    let version = meta
        .lines()
        .find_map(|line| line.strip_prefix("version="))
        .map(str::to_string)?;
    let arch_matches = meta
        .lines()
        .any(|line| line == format!("arch={}", target_arch()));
    arch_matches.then_some(version)
}

/// Helper struct to automatically close WinHTTP handles
#[cfg(windows)]
struct HandleGuard(*mut std::ffi::c_void);

#[cfg(windows)]
impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
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
        let mut components = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwHostNameLength: host_name.len() as u32,
            lpszHostName: PWSTR(host_name.as_mut_ptr()),
            dwUrlPathLength: url_path.len() as u32,
            lpszUrlPath: PWSTR(url_path.as_mut_ptr()),
            ..Default::default()
        };

        if WinHttpCrackUrl(&url_wide, 0, &mut components).is_err() {
            return Err(format!("WinHttpCrackUrl failed: {:?}", GetLastError()).into());
        }

        // 3. Connect
        let connect = WinHttpConnect(
            session,
            PCWSTR(components.lpszHostName.0),
            components.nPort,
            0,
        );
        if connect.is_null() {
            return Err(format!("WinHttpConnect failed: {:?}", GetLastError()).into());
        }
        let _connect_guard = HandleGuard(connect);

        // 4. Open Request
        let flags = if components.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
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
        if WinHttpSendRequest(request, None, None, 0, 0, 0).is_err() {
            return Err(format!("WinHttpSendRequest failed: {:?}", GetLastError()).into());
        }

        // 6. Receive Response
        if WinHttpReceiveResponse(request, std::ptr::null_mut()).is_err() {
            return Err(format!("WinHttpReceiveResponse failed: {:?}", GetLastError()).into());
        }

        // 6b. Check the HTTP status code. WinHTTP follows redirects by default, so this
        // is the FINAL status (e.g. after a GitHub asset URL redirects to its CDN).
        // Without this, a 404/403/5xx HTML error body would be read as "success" and,
        // for the asset download, written to disk and installed as the executable.
        let mut status_code: u32 = 0;
        let mut status_len = std::mem::size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(), // WINHTTP_HEADER_NAME_BY_INDEX
            Some(&mut status_code as *mut u32 as *mut c_void),
            &mut status_len,
            std::ptr::null_mut(), // WINHTTP_NO_HEADER_INDEX
        )
        .is_err()
        {
            return Err(format!("WinHttpQueryHeaders failed: {:?}", GetLastError()).into());
        }
        if !(200..300).contains(&status_code) {
            return Err(format!("HTTP request failed with status {}", status_code).into());
        }

        // 7. Read Data
        let mut body = Vec::new();
        let mut buffer = vec![0u8; 8192];
        let mut bytes_read = 0;

        loop {
            // Propagate read errors instead of returning a truncated body as Ok:
            // a mid-stream failure must NOT be reported as a complete download, or a
            // corrupt partial .exe could be installed over the working one.
            if WinHttpQueryDataAvailable(request, &mut bytes_read).is_err() {
                return Err(
                    format!("WinHttpQueryDataAvailable failed: {:?}", GetLastError()).into(),
                );
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
            )
            .is_err()
            {
                return Err(format!("WinHttpReadData failed: {:?}", GetLastError()).into());
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
pub fn get_latest_release() -> Result<(String, String), Box<dyn std::error::Error>> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );

    // Fetch JSON from GitHub API
    let body = native_http_get(&url)?;
    let json_text = String::from_utf8(body).map_err(|_| "GitHub API returned invalid UTF-8")?;

    // Parse JSON manually to avoid complex deps
    // We look for "tag_name": "vX.Y.Z"
    let version = json_text
        .split("\"tag_name\"")
        .nth(1)
        .and_then(|s| s.split(':').nth(1))
        .and_then(|s| s.split("\"").nth(1))
        .ok_or_else(|| {
            format!(
                "Failed to parse tag_name from GitHub API response (body length: {} bytes)",
                json_text.len()
            )
        })?
        .trim_start_matches('v')
        .to_string();

    // Detect architecture
    let target_arch = target_arch();
    let target_suffix = format!("htop-win-{}.exe", target_arch);

    // Find asset URL
    // Look for "browser_download_url": "..." that ends with target_suffix
    // Note: Can't split on ':' because URLs contain "https:"
    let mut download_url = String::new();
    for part in json_text.split("\"browser_download_url\"") {
        // Part starts with: ": "https://..." or similar
        // Extract the first quoted string after the colon-space separator
        if let Some(after_colon) = part.split_once(':') {
            // after_colon.1 is everything after the first ':', e.g. ' "https://...foo.exe",...'
            let rest = after_colon.1.trim();
            if rest.starts_with('"')
                && let Some(url) = rest[1..].split('"').next()
                && url.ends_with(&target_suffix)
            {
                download_url = url.to_string();
                break;
            }
        }
    }

    if version.is_empty() || download_url.is_empty() {
        return Err(format!(
            "Could not find download URL for this architecture (version={}, url_empty={})",
            version,
            download_url.is_empty()
        )
        .into());
    }

    Ok((version, download_url))
}

/// Clean up any leftover temp files from previous updates
fn cleanup_temp_files() {
    let temp_dir = std::env::temp_dir();
    let _ = fs::remove_file(temp_dir.join("htop-win-update.exe"));
    let _ = fs::remove_file(update_meta_path());
}

/// Update htop-win from GitHub releases
pub fn update_from_github(force: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Clean up any old temp files from previous failed updates
    cleanup_temp_files();

    println!("Checking for updates...");

    let (latest_version, download_url) = match get_latest_release() {
        Ok(v) => v,
        Err(e) => return Err(format!("Failed to check for updates: {}", e).into()),
    };

    let current_version = env!("CARGO_PKG_VERSION");

    if !force && !is_newer_version(&latest_version, current_version) {
        println!("htop {} is already the latest version.", current_version);
        println!("\nUse --force to reinstall anyway.");
        return Ok(());
    }

    if force && !is_newer_version(&latest_version, current_version) {
        println!("Force reinstalling htop {} from GitHub...", latest_version);
    } else {
        println!(
            "New version available: {} -> {}",
            current_version, latest_version
        );
    }
    println!("Downloading from GitHub...");

    // Download to temp file
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("htop-win-update.exe");

    let body = native_http_get(&download_url)?;
    validate_target_pe_executable(&body)
        .map_err(|e| format!("Downloaded update rejected: {}", e))?;
    fs::write(&temp_file, body)?;
    write_update_metadata(&latest_version);

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

    // If update already downloaded and pending, report it without re-downloading
    if temp_file.exists()
        && let Ok(pending) = fs::read(&temp_file)
    {
        if validate_target_pe_executable(&pending).is_ok() {
            // A non-empty pending update exists. Act only on a definitive answer
            // from GitHub: report it if newer, delete it only if CONFIRMED stale.
            // On a transient API failure, keep it — don't discard a valid,
            // already-downloaded update just because the check momentarily failed.
            match get_latest_release() {
                Ok((latest_version, _)) => {
                    let current_version = env!("CARGO_PKG_VERSION");
                    if is_newer_version(&latest_version, current_version)
                        && pending_metadata_matches(&latest_version)
                    {
                        return UpdateStatus::Downloaded {
                            version: latest_version,
                            path: temp_file,
                        };
                    }
                    // Confirmed not newer than current -- the pending file is stale.
                    let _ = fs::remove_file(&temp_file);
                    let _ = fs::remove_file(update_meta_path());
                }
                Err(_) => {
                    // Couldn't reach GitHub; preserve the pending update for retry.
                    return UpdateStatus::None;
                }
            }
        } else {
            let _ = fs::remove_file(&temp_file);
        }
    }

    let current_version = env!("CARGO_PKG_VERSION");

    let (latest_version, download_url) = match get_latest_release() {
        Ok(v) => v,
        Err(_) => return UpdateStatus::None,
    };

    if !is_newer_version(&latest_version, current_version) {
        return UpdateStatus::None;
    }

    match native_http_get(&download_url) {
        Ok(body) if validate_target_pe_executable(&body).is_ok() => {
            if fs::write(&temp_file, body).is_ok() {
                write_update_metadata(&latest_version);
                UpdateStatus::Downloaded {
                    version: latest_version,
                    path: temp_file,
                }
            } else {
                UpdateStatus::None
            }
        }
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

    // Verify update file integrity before installing it over the working exe.
    // The pending file may have been written by an older htop-win without
    // download-time validation, so this gate must not rely on the downloader.
    let pending_version = pending_metadata_version();
    let current_version = env!("CARGO_PKG_VERSION");
    match fs::read(&update_file) {
        Ok(body)
            if validate_target_pe_executable(&body).is_ok()
                && pending_version
                    .as_deref()
                    .is_some_and(|version| is_newer_version(version, current_version)) => {}
        Ok(_) => {
            // Not a plausible executable — discard rather than install
            let _ = fs::remove_file(&update_file);
            let _ = fs::remove_file(update_meta_path());
            return false;
        }
        Err(_) => return false,
    }

    let install_path = current_exe;

    // If install path doesn't exist, just copy directly
    if !install_path.exists() {
        if fs::copy(&update_file, &install_path).is_ok() {
            let _ = fs::remove_file(&update_file);
            let _ = fs::remove_file(update_meta_path());
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
        if let Err(e2) = fs::rename(&backup_path, &install_path) {
            eprintln!(
                "CRITICAL: Failed to restore backup: {}. Working executable is at: {:?}",
                e2, backup_path
            );
        }
        // Keep update file for retry
        return true; // Return true to skip re-download
    }

    // Clean up update file ONLY on success
    let _ = fs::remove_file(&update_file);
    let _ = fs::remove_file(update_meta_path());

    // Try to remove backup, but ignore error if locked (it's the running executable)
    let _ = fs::remove_file(&backup_path);

    eprintln!("Update applied successfully!");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smallest buffer validate_pe_executable accepts: MZ header, e_lfanew
    /// pointing at a PE\0\0 signature, padded past MIN_UPDATE_SIZE.
    fn synthetic_pe() -> Vec<u8> {
        let mut body = vec![0u8; MIN_UPDATE_SIZE + 1024];
        body[0] = b'M';
        body[1] = b'Z';
        body[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        body[0x80..0x84].copy_from_slice(b"PE\0\0");
        body
    }

    #[test]
    fn test_validate_pe_accepts_synthetic_pe() {
        assert!(validate_pe_executable(&synthetic_pe()).is_ok());
    }

    #[test]
    fn test_validate_pe_rejects_empty() {
        assert!(validate_pe_executable(&[]).is_err());
    }

    #[test]
    fn test_validate_pe_rejects_missing_mz() {
        let body = vec![b'A'; MIN_UPDATE_SIZE + 1024];
        assert!(validate_pe_executable(&body).is_err());
    }

    #[test]
    fn test_validate_pe_rejects_too_small() {
        let mut body = synthetic_pe();
        body.truncate(MIN_UPDATE_SIZE / 2);
        assert!(validate_pe_executable(&body).is_err());
    }

    #[test]
    fn test_validate_pe_rejects_missing_pe_signature() {
        let mut body = synthetic_pe();
        body[0x80..0x84].copy_from_slice(b"XX\0\0");
        assert!(validate_pe_executable(&body).is_err());
    }

    #[test]
    fn test_validate_pe_rejects_out_of_bounds_e_lfanew() {
        let mut body = synthetic_pe();
        let oob = (body.len() as u32).to_le_bytes();
        body[0x3c..0x40].copy_from_slice(&oob);
        assert!(validate_pe_executable(&body).is_err());
    }

    #[test]
    fn test_validate_pe_rejects_html_error_page() {
        let mut body = b"<html><body>404 Not Found</body></html>".to_vec();
        body.resize(MIN_UPDATE_SIZE + 1024, b' ');
        assert!(validate_pe_executable(&body).is_err());
    }

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("0.2.0", "0.1.10"));
        assert!(is_newer_version("0.10.0", "0.9.9"));
        assert!(!is_newer_version("0.1.10", "0.2.0"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("garbage", "1.0.0"));
    }
}
