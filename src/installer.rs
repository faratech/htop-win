//! Installation and update functionality for htop-win

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[cfg(windows)]
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_ABANDONED, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
#[cfg(windows)]
use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
#[cfg(windows)]
use windows::core::{PCWSTR, PWSTR, w};

use crate::update_trust::{self, SignerChain, Subject, VersionRule, VersionStrings};

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

    read_pe_file_version(&install_path)
}

/// Read the embedded VS_VERSIONINFO FileVersion without executing the binary.
/// The previous approach ran `htop.exe --version`, which both blocked
/// indefinitely on a wedged executable at the install path and executed
/// whatever file happened to occupy it.
#[cfg(windows)]
fn read_pe_file_version(path: &std::path::Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };
    use windows::core::PCWSTR;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let path_pcwstr = PCWSTR(wide.as_ptr());
    unsafe {
        let size = GetFileVersionInfoSizeW(path_pcwstr, None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path_pcwstr, None, size, data.as_mut_ptr() as *mut _).is_err() {
            return None;
        }
        let mut fixed: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(
            data.as_ptr() as *const _,
            windows::core::w!("\\"),
            &mut fixed,
            &mut len,
        )
        .as_bool()
            || fixed.is_null()
            || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }
        // SAFETY: every precondition is checked above — `data` is alive and
        // `size` bytes long, VerQueryValueW reported success for the root
        // value, `fixed` is non-null and `len` is at least the size of
        // VS_FIXEDFILEINFO. `fixed` points into `data`, so the unaligned read
        // stays in bounds; the signature check below gates interpretation of
        // the fields, not memory validity. (CodeQL cannot model the Win32
        // contract binding `fixed` to `data` and may still report this line.)
        let v = fixed.cast::<VS_FIXEDFILEINFO>().read_unaligned();
        if v.dwSignature != 0xfeef04bd {
            return None;
        }
        // Cargo versions here are three-part; the fourth (build) field is
        // padding set by the resource compiler.
        Some(format!(
            "{}.{}.{}",
            v.dwFileVersionMS >> 16,
            v.dwFileVersionMS & 0xffff,
            v.dwFileVersionLS >> 16
        ))
    }
}

/// Non-Windows fallback: shells out, as before (development builds only).
#[cfg(not(windows))]
fn read_pe_file_version(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    let version_output = String::from_utf8_lossy(&output.stdout);
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

    if target_path.exists() && paths_refer_to_same_file(&current_exe, &target_path)? {
        println!(
            "htop {} is already installed and up to date.",
            current_version
        );
        println!("Location: {}", target_path.display());
        return Ok(());
    }

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

    // Install via the same rename-aside helper the self-update path uses:
    // Windows refuses to open a running executable's image for write, so a
    // plain copy onto the installed exe fails while any htop is running.
    install_update_file(&current_exe, &target_path)?;

    println!("Successfully installed htop {}!", current_version);
    println!("Location: {}", target_path.display());
    println!("\nYou can now run 'htop' from any terminal.");
    Ok(())
}

fn paths_refer_to_same_file(source: &Path, target: &Path) -> io::Result<bool> {
    let source_identity = file_identity(source)?;
    if source_identity != (0, 0) && source_identity == file_identity(target)? {
        return Ok(true);
    }

    let source = fs::canonicalize(source)?;
    let target = fs::canonicalize(target)?;
    Ok(source
        .to_string_lossy()
        .eq_ignore_ascii_case(&target.to_string_lossy()))
}

#[repr(C)]
struct FileInformation {
    file_attributes: u32,
    creation_time: [u32; 2],
    last_access_time: [u32; 2],
    last_write_time: [u32; 2],
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetFileInformationByHandle"]
    fn get_file_information_by_handle(
        file: *mut std::ffi::c_void,
        information: *mut FileInformation,
    ) -> i32;
}

fn file_identity(path: &Path) -> io::Result<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;

    let file = File::open(path)?;
    let mut information = std::mem::MaybeUninit::<FileInformation>::uninit();
    let succeeded = unsafe {
        get_file_information_by_handle(file.as_raw_handle(), information.as_mut_ptr()) != 0
    };
    if !succeeded {
        return Err(io::Error::last_os_error());
    }
    let information = unsafe { information.assume_init() };
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    Ok((information.volume_serial_number, index))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemVersion<'a> {
    core: [&'a str; 3],
    prerelease: Option<Vec<&'a str>>,
}

fn valid_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Parse one optional `v` prefix followed by a strict SemVer string.
fn parse_version(version: &str) -> Option<SemVersion<'_>> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let (without_build, build) = version
        .split_once('+')
        .map_or((version, None), |(core, build)| (core, Some(build)));
    if build.is_some_and(|build| {
        build
            .split('.')
            .any(|identifier| !valid_identifier(identifier))
    }) || without_build.contains('+')
    {
        return None;
    }

    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });
    let core = core.split('.').collect::<Vec<_>>();
    if core.len() != 3 || core.iter().any(|part| !valid_numeric_identifier(part)) {
        return None;
    }

    let prerelease = match prerelease {
        Some(prerelease) => {
            let identifiers = prerelease.split('.').collect::<Vec<_>>();
            if identifiers.iter().any(|identifier| {
                !valid_identifier(identifier)
                    || (identifier.bytes().all(|byte| byte.is_ascii_digit())
                        && !valid_numeric_identifier(identifier))
            }) {
                return None;
            }
            Some(identifiers)
        }
        None => None,
    };

    Some(SemVersion {
        core: [core[0], core[1], core[2]],
        prerelease,
    })
}

fn valid_numeric_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier.bytes().all(|byte| byte.is_ascii_digit())
        && (identifier == "0" || !identifier.starts_with('0'))
}

fn compare_numeric_identifier(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn compare_versions(a: &SemVersion<'_>, b: &SemVersion<'_>) -> Ordering {
    for (a, b) in a.core.iter().zip(b.core.iter()) {
        let ordering = compare_numeric_identifier(a, b);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    match (&a.prerelease, &b.prerelease) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => {
            for (a, b) in a.iter().zip(b.iter()) {
                let a_numeric = a.bytes().all(|byte| byte.is_ascii_digit());
                let b_numeric = b.bytes().all(|byte| byte.is_ascii_digit());
                let ordering = match (a_numeric, b_numeric) {
                    (true, true) => compare_numeric_identifier(a, b),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => a.cmp(b),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            a.len().cmp(&b.len())
        }
    }
}

/// Compare two version strings, returns true if `a` is newer than `b`
pub fn is_newer_version(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Some(va), Some(vb)) => compare_versions(&va, &vb) == Ordering::Greater,
        _ => false,
    }
}

/// Minimum plausible size for a release binary (real ones are ~600-800 KB).
const MIN_UPDATE_SIZE: usize = 100 * 1024;

/// Reject anything that is not plausibly a Windows PE executable, so a CDN
/// error page or truncated download is never installed over the working exe.
/// This is only a cheap pre-filter: authenticity comes from
/// [`open_verified_update`], which every update must pass before it is staged
/// and again before it is installed.
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

/// How [`open_verified_update`] checks certificate revocation.
#[derive(Clone, Copy, Debug)]
enum RevocationCheck {
    /// Check the whole chain except the root, fetching CRL/OCSP data as
    /// needed. Used whenever the file was just downloaded or is installed on
    /// explicit request.
    Online,
    /// Skip the revocation check and never touch the network. Only for
    /// applying a staged update at startup: staging already checked
    /// revocation online, startup must not wait on CRL/OCSP downloads, and an
    /// offline launch must not discard a verified update.
    SkipAtStartup,
}

/// Open `path` without write or delete sharing and prove it is the htop-win
/// release it claims to be. Every check fails closed:
/// 1. WinVerifyTrust (WINTRUST_ACTION_GENERIC_VERIFY_V2) must return success.
/// 2. The primary signature's chain must be the Trusted Signing chain for
///    "Fara Technologies LLC" ([`update_trust::check_signer_chain`]).
/// 3. The VERSIONINFO of the same, still-locked file must name htop-win and
///    a FileVersion allowed by `rule` ([`update_trust::check_update_identity`]),
///    so neither another Fara Technologies program nor an older htop-win
///    (rollback) is accepted.
///
/// Callers that install the file keep the returned handle open until the
/// copy finishes, so the bytes installed are the bytes verified.
fn open_verified_update(
    path: &Path,
    revocation: RevocationCheck,
    rule: VersionRule<'_>,
) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;

    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)
        .map_err(|error| format!("failed to open update for verification: {error}"))?;
    let chain = authenticode_signer_chain(path, &file, revocation)?;
    update_trust::check_signer_chain(&chain)?;
    let strings = read_version_strings(path)?;
    update_trust::check_update_identity(
        &strings,
        rule,
        env!("CARGO_PKG_VERSION"),
        is_newer_version,
    )?;
    Ok(file)
}

/// Run WinVerifyTrust on the open file and return the chain it built for
/// the primary signature. Any status other than success is an error.
fn authenticode_signer_chain(
    path: &Path,
    file: &File,
    revocation: RevocationCheck,
) -> Result<SignerChain, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{HWND, INVALID_HANDLE_VALUE};
    use windows::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
        WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_DISABLE_MD2_MD4,
        WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, WTD_REVOKE_NONE, WTD_REVOKE_WHOLECHAIN,
        WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
    };

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide_path.as_ptr()),
        hFile: HANDLE(file.as_raw_handle()),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let (revocation_checks, provider_flags) = match revocation {
        RevocationCheck::Online => (
            WTD_REVOKE_WHOLECHAIN,
            WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT | WTD_DISABLE_MD2_MD4,
        ),
        RevocationCheck::SkipAtStartup => (
            WTD_REVOKE_NONE,
            WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_DISABLE_MD2_MD4,
        ),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: revocation_checks,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: provider_flags,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // INVALID_HANDLE_VALUE as the window means no interactive user: never show UI.
    let no_ui = HWND(INVALID_HANDLE_VALUE.0);

    unsafe {
        let status = WinVerifyTrust(no_ui, &mut action, (&raw mut data).cast());
        let result = if status == 0 {
            // The provider state stays valid until WTD_STATEACTION_CLOSE.
            read_signer_chain(data.hWVTStateData)
        } else {
            Err(format!(
                "Authenticode verification failed ({:#010x}: {})",
                status as u32,
                windows::core::HRESULT(status).message().trim_end()
            ))
        };
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(no_ui, &mut action, (&raw mut data).cast());
        result
    }
}

/// Subjects (leaf to root) and leaf EKUs of signer 0, the primary signature.
unsafe fn read_signer_chain(state: HANDLE) -> Result<SignerChain, String> {
    use windows::Win32::Security::WinTrust::{
        WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData,
    };

    let missing = || "Authenticode verification returned no signer chain".to_string();
    let provider = unsafe { WTHelperProvDataFromStateData(state) };
    if provider.is_null() {
        return Err(missing());
    }
    let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, false, 0) };
    if signer.is_null() {
        return Err(missing());
    }
    let mut chain = SignerChain::default();
    for index in 0..unsafe { (*signer).csCertChain } {
        let entry = unsafe { WTHelperGetProvCertFromChain(signer, index) };
        if entry.is_null() || unsafe { (*entry).pCert.is_null() } {
            return Err(missing());
        }
        let cert = unsafe { (*entry).pCert };
        chain.subjects.push(unsafe { cert_subject(cert) }?);
        if index == 0 {
            chain.leaf_ekus = unsafe { cert_extended_key_usages(cert) }?;
        }
    }
    Ok(chain)
}

/// The certificate's subject as parsed RDN attributes, in encoded order.
/// Values that are not strings decode to U+FFFD, which matches nothing.
unsafe fn cert_subject(
    cert: *const windows::Win32::Security::Cryptography::CERT_CONTEXT,
) -> Result<Subject, String> {
    use windows::Win32::Security::Cryptography::{
        CERT_NAME_INFO, CERT_RDN_ENCODED_BLOB, CERT_RDN_OCTET_STRING, CryptDecodeObjectEx,
        X509_ASN_ENCODING, X509_UNICODE_NAME,
    };

    let info = unsafe { (*cert).pCertInfo };
    if info.is_null() {
        return Err("signing certificate has no CERT_INFO".into());
    }
    let blob = unsafe { (*info).Subject };
    let encoded = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) };
    let decode_error =
        |error: windows::core::Error| format!("cannot decode certificate subject: {error}");
    let mut size = 0u32;
    unsafe {
        CryptDecodeObjectEx(
            X509_ASN_ENCODING,
            X509_UNICODE_NAME,
            encoded,
            0,
            None,
            None,
            &mut size,
        )
    }
    .map_err(decode_error)?;
    // u64 storage keeps the decoded structures aligned.
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    unsafe {
        CryptDecodeObjectEx(
            X509_ASN_ENCODING,
            X509_UNICODE_NAME,
            encoded,
            0,
            None,
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
        )
    }
    .map_err(decode_error)?;

    let name = unsafe { &*buffer.as_ptr().cast::<CERT_NAME_INFO>() };
    let mut subject = Subject::new();
    for rdn in unsafe { raw_slice(name.rgRDN, name.cRDN) } {
        for attribute in unsafe { raw_slice(rdn.rgRDNAttr, rdn.cRDNAttr) } {
            let oid = unsafe { attribute.pszObjId.to_string() }
                .map_err(|_| "certificate subject has a non-UTF-8 OID".to_string())?;
            let value_type = attribute.dwValueType as i32;
            let value = if value_type == CERT_RDN_ENCODED_BLOB.0
                || value_type == CERT_RDN_OCTET_STRING.0
            {
                "\u{FFFD}".to_string()
            } else {
                // X509_UNICODE_NAME decodes string values to UTF-16.
                let bytes = unsafe { raw_slice(attribute.Value.pbData, attribute.Value.cbData) };
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&pair| u16::from_le_bytes(pair))
                    .collect();
                String::from_utf16(&units)
                    .map_err(|_| "certificate subject has invalid UTF-16".to_string())?
                    .trim_end_matches('\0')
                    .to_string()
            };
            subject.push((oid, value));
        }
    }
    Ok(subject)
}

/// EKU OIDs from the certificate's extension (not from store properties).
unsafe fn cert_extended_key_usages(
    cert: *const windows::Win32::Security::Cryptography::CERT_CONTEXT,
) -> Result<Vec<String>, String> {
    use windows::Win32::Security::Cryptography::{
        CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG, CTL_USAGE, CertGetEnhancedKeyUsage,
    };

    let read_error =
        |error: windows::core::Error| format!("cannot read signing certificate EKUs: {error}");
    let flags = CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG.0;
    let mut size = 0u32;
    unsafe { CertGetEnhancedKeyUsage(cert, flags, None, &mut size) }.map_err(read_error)?;
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    unsafe { CertGetEnhancedKeyUsage(cert, flags, Some(buffer.as_mut_ptr().cast()), &mut size) }
        .map_err(read_error)?;
    let usage = unsafe { &*buffer.as_ptr().cast::<CTL_USAGE>() };
    unsafe { raw_slice(usage.rgpszUsageIdentifier, usage.cUsageIdentifier) }
        .iter()
        .map(|oid| unsafe { oid.to_string() }.map_err(|_| "EKU OID is not UTF-8".to_string()))
        .collect()
}

/// `len` elements at `ptr`, or an empty slice for a null pointer.
unsafe fn raw_slice<'a, T>(ptr: *const T, len: u32) -> &'a [T] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

/// The VERSIONINFO strings of the (locked) update file, read from the file
/// itself (FILE_VER_GET_NEUTRAL: never a side-by-side .mui file).
fn read_version_strings(path: &Path) -> Result<VersionStrings, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_VER_GET_NEUTRAL, GetFileVersionInfoExW, GetFileVersionInfoSizeExW, VerQueryValueW,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut ignored = 0u32;
    let size = unsafe {
        GetFileVersionInfoSizeExW(FILE_VER_GET_NEUTRAL, PCWSTR(wide.as_ptr()), &mut ignored)
    };
    if size == 0 {
        return Err("update has no version resource".into());
    }
    let mut block = vec![0u8; size as usize];
    unsafe {
        GetFileVersionInfoExW(
            FILE_VER_GET_NEUTRAL,
            PCWSTR(wide.as_ptr()),
            None,
            size,
            block.as_mut_ptr().cast(),
        )
    }
    .map_err(|error| format!("cannot read version resource: {error}"))?;

    // Bytes of `query` inside `block`, or None if absent or out of bounds.
    let query = |query: &str, unit: usize| -> Option<&[u8]> {
        let query: Vec<u16> = query.encode_utf16().chain(std::iter::once(0)).collect();
        let mut value: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let found = unsafe {
            VerQueryValueW(
                block.as_ptr().cast(),
                PCWSTR(query.as_ptr()),
                &mut value,
                &mut len,
            )
        };
        let start = (value as usize).checked_sub(block.as_ptr() as usize)?;
        let end = start.checked_add(len as usize * unit)?;
        (found.as_bool() && !value.is_null()).then(|| block.get(start..end))?
    };
    let translation = query("\\VarFileInfo\\Translation", 1)
        .filter(|bytes| bytes.len() >= 4)
        .ok_or("version resource has no translation table")?;
    let language = u16::from_le_bytes([translation[0], translation[1]]);
    let code_page = u16::from_le_bytes([translation[2], translation[3]]);
    let string = |name: &str| -> Option<String> {
        let bytes = query(
            &format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\{name}"),
            2,
        )?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .take_while(|&unit| unit != 0)
            .collect();
        String::from_utf16(&units).ok()
    };
    Ok(VersionStrings {
        product_name: string("ProductName"),
        original_filename: string("OriginalFilename"),
        internal_name: string("InternalName"),
        file_version: string("FileVersion"),
    })
}

/// Install `update_file` over `target_path` only after it passes
/// [`open_verified_update`]. The verification handle stays open (denying
/// writers) until the copy completes.
fn install_verified_update(
    update_file: &Path,
    target_path: &Path,
    revocation: RevocationCheck,
    rule: VersionRule<'_>,
) -> Result<(), String> {
    let verified = open_verified_update(update_file, revocation, rule)
        .map_err(|error| format!("update rejected: {error}"))?;
    let installed = install_update_file(update_file, target_path)
        .map_err(|error| format!("installation failed: {error}"));
    drop(verified);
    installed
}

const UPDATE_FILE_NAME: &str = "htop-win-update.exe";
const UPDATE_METADATA_NAME: &str = "htop-win-update.meta";
const UPDATE_ROOT_NAME: &str = "htop-win-updates";
const ABANDONED_STAGE_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
static STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct PendingUpdate {
    version: String,
    path: PathBuf,
    generation_dir: Option<PathBuf>,
}

fn update_root_path() -> PathBuf {
    std::env::temp_dir()
        .join(UPDATE_ROOT_NAME)
        .join(target_arch())
}

fn legacy_update_path() -> PathBuf {
    std::env::temp_dir().join(UPDATE_FILE_NAME)
}

fn legacy_update_meta_path() -> PathBuf {
    std::env::temp_dir().join(UPDATE_METADATA_NAME)
}

fn update_metadata(version: &str) -> String {
    format!("version={version}\narch={}\n", target_arch())
}

fn read_update_metadata(path: &Path) -> Result<String, String> {
    let metadata = fs::read_to_string(path)
        .map_err(|error| format!("failed to read update metadata: {error}"))?;
    let version = metadata
        .lines()
        .find_map(|line| line.strip_prefix("version="))
        .ok_or_else(|| "update metadata is missing a version".to_string())?;
    if parse_version(version).is_none() {
        return Err(format!("update metadata has invalid SemVer: {version}"));
    }
    if !metadata
        .lines()
        .any(|line| line == format!("arch={}", target_arch()))
    {
        return Err(format!(
            "update metadata architecture does not match {}",
            target_arch()
        ));
    }
    Ok(version.to_string())
}

fn unique_generation_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = STAGE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    format!("{}-{nanos}-{counter}", std::process::id())
}

fn write_synced_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

/// Write a complete update pair into a private directory, then publish it with
/// one same-volume directory rename. Readers never observe a partial pair, and
/// nothing is published unless the executable passes [`open_verified_update`]
/// under `rule`, with an online revocation check since it was just downloaded.
fn stage_pending_update(
    version: &str,
    body: &[u8],
    rule: VersionRule<'_>,
) -> Result<PendingUpdate, String> {
    stage_pending_update_in(&update_root_path(), version, body, |path| {
        open_verified_update(path, RevocationCheck::Online, rule).map(drop)
    })
}

fn stage_pending_update_in(
    root: &Path,
    version: &str,
    body: &[u8],
    verify: impl Fn(&Path) -> Result<(), String>,
) -> Result<PendingUpdate, String> {
    validate_target_pe_executable(body)
        .map_err(|error| format!("downloaded update rejected: {error}"))?;
    if parse_version(version).is_none() {
        return Err(format!("release returned invalid SemVer: {version}"));
    }

    fs::create_dir_all(root)
        .map_err(|error| format!("failed to create update staging directory: {error}"))?;
    let id = unique_generation_id();
    let staging_dir = root.join(format!(".stage-{id}"));
    let generation_dir = root.join(format!("pending-{id}"));
    fs::create_dir(&staging_dir)
        .map_err(|error| format!("failed to create private update stage: {error}"))?;

    let result = (|| -> Result<(), String> {
        let staged_file = staging_dir.join(UPDATE_FILE_NAME);
        write_synced_file(&staged_file, body)
            .map_err(|error| format!("failed to write staged update: {error}"))?;
        verify(&staged_file).map_err(|error| format!("downloaded update rejected: {error}"))?;
        write_synced_file(
            &staging_dir.join(UPDATE_METADATA_NAME),
            update_metadata(version).as_bytes(),
        )
        .map_err(|error| format!("failed to write staged update metadata: {error}"))?;
        fs::rename(&staging_dir, &generation_dir)
            .map_err(|error| format!("failed to publish staged update: {error}"))?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    Ok(PendingUpdate {
        version: version.to_string(),
        path: generation_dir.join(UPDATE_FILE_NAME),
        generation_dir: Some(generation_dir),
    })
}

fn validate_pending_pair(update_path: &Path, metadata_path: &Path) -> Result<String, String> {
    let version = read_update_metadata(metadata_path)?;
    let body =
        fs::read(update_path).map_err(|error| format!("failed to read pending update: {error}"))?;
    validate_target_pe_executable(&body)
        .map_err(|error| format!("pending update rejected: {error}"))?;
    Ok(version)
}

fn load_pending_generations(root: &Path) -> Result<Vec<PendingUpdate>, String> {
    let mut updates = Vec::new();
    if root.exists() {
        let entries = fs::read_dir(root)
            .map_err(|error| format!("failed to inspect pending updates: {error}"))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("failed to inspect update entry: {error}"))?;
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            let path = entry.path();
            if file_name.starts_with(".stage-") {
                let abandoned = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age >= ABANDONED_STAGE_AGE);
                if abandoned {
                    let _ = fs::remove_dir_all(path);
                }
                continue;
            }
            if !file_name.starts_with("pending-") || !path.is_dir() {
                continue;
            }

            let update_path = path.join(UPDATE_FILE_NAME);
            let metadata_path = path.join(UPDATE_METADATA_NAME);
            match validate_pending_pair(&update_path, &metadata_path) {
                Ok(version) => updates.push(PendingUpdate {
                    version,
                    path: update_path,
                    generation_dir: Some(path),
                }),
                Err(_) => {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }
    }
    Ok(updates)
}

fn load_pending_updates() -> Result<Vec<PendingUpdate>, String> {
    let mut updates = load_pending_generations(&update_root_path())?;
    let legacy_update = legacy_update_path();
    let legacy_metadata = legacy_update_meta_path();
    if legacy_update.exists() || legacy_metadata.exists() {
        match validate_pending_pair(&legacy_update, &legacy_metadata) {
            Ok(version) => updates.push(PendingUpdate {
                version,
                path: legacy_update,
                generation_dir: None,
            }),
            Err(_) => {
                let _ = fs::remove_file(legacy_update);
                let _ = fs::remove_file(legacy_metadata);
            }
        }
    }
    Ok(updates)
}

fn remove_pending_update(update: &PendingUpdate) {
    if let Some(generation_dir) = &update.generation_dir {
        let _ = fs::remove_dir_all(generation_dir);
    } else {
        let _ = fs::remove_file(&update.path);
        let _ = fs::remove_file(legacy_update_meta_path());
    }
}

fn newest_pending_update(updates: &[PendingUpdate]) -> Option<PendingUpdate> {
    updates
        .iter()
        .filter(|update| is_newer_version(&update.version, env!("CARGO_PKG_VERSION")))
        .max_by(|a, b| {
            let a = parse_version(&a.version).expect("pending versions were validated");
            let b = parse_version(&b.version).expect("pending versions were validated");
            compare_versions(&a, &b)
        })
        .cloned()
}

struct UpdateLock {
    handle: HANDLE,
}

fn update_mutex_name() -> Vec<u16> {
    // Global namespace covers console/RDP sessions. Hash the per-user temp
    // path so unrelated users do not contend for, or need access to, the same
    // kernel object.
    let mut hash = 0xcbf29ce484222325u64;
    let lock_identity = get_install_path().unwrap_or_else(|_| std::env::temp_dir());
    for byte in lock_identity.to_string_lossy().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("Global\\htop-win-update-{hash:016x}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

impl UpdateLock {
    fn acquire() -> io::Result<Self> {
        unsafe {
            // A kernel mutex is released automatically if its owner exits, so
            // crash recovery cannot race with a new owner the way stale lock-
            // file deletion can.
            let name = update_mutex_name();
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))
                .map_err(|error| io::Error::other(format!("CreateMutexW failed: {error}")))?;
            let wait = WaitForSingleObject(handle, 5_000);
            if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                return Ok(Self { handle });
            }

            let error = if wait == WAIT_TIMEOUT {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out waiting for the update lock",
                )
            } else if wait == WAIT_FAILED {
                io::Error::last_os_error()
            } else {
                io::Error::other(format!(
                    "unexpected update mutex wait result: {:#x}",
                    wait.0
                ))
            };
            let _ = CloseHandle(handle);
            Err(error)
        }
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
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
    let release_tag = json_text
        .split("\"tag_name\"")
        .nth(1)
        .and_then(|s| s.split(':').nth(1))
        .and_then(|s| s.split("\"").nth(1))
        .ok_or_else(|| {
            format!(
                "Failed to parse tag_name from GitHub API response (body length: {} bytes)",
                json_text.len()
            )
        })?;
    let version = release_tag
        .strip_prefix('v')
        .ok_or_else(|| format!("Release tag must start with 'v': {release_tag}"))?
        .to_string();
    if parse_version(&version).is_none() {
        return Err(format!("Release tag is not strict SemVer: {release_tag}").into());
    }

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

/// Update htop-win from GitHub releases
pub fn update_from_github(force: bool) -> Result<(), Box<dyn std::error::Error>> {
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

    // --force may reinstall the current release, never an older one.
    let rule = if is_newer_version(&latest_version, current_version) {
        VersionRule::Release(&latest_version)
    } else {
        VersionRule::Reinstall(&latest_version)
    };
    if force && !is_newer_version(&latest_version, current_version) {
        println!("Force reinstalling htop {} from GitHub...", latest_version);
    } else {
        println!(
            "New version available: {} -> {}",
            current_version, latest_version
        );
    }
    println!("Downloading from GitHub...");

    let body = native_http_get(&download_url)
        .map_err(|error| format!("Failed to download update: {error}"))?;

    // Serialize publication and replacement, not the network request. A
    // published generation must never be visible to another instance's
    // cleanup/apply pass before this explicit install consumes it.
    let _lock = UpdateLock::acquire()?;
    let pending = stage_pending_update(&latest_version, &body, rule)?;

    println!("Download complete. Installing...");

    let target_path = get_install_path()?;
    install_verified_update(&pending.path, &target_path, RevocationCheck::Online, rule)?;
    remove_pending_update(&pending);
    print_update_success(&target_path);
    Ok(())
}

fn install_update_file(update_file: &Path, target_path: &Path) -> io::Result<()> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if !target_path.exists() {
        fs::copy(update_file, target_path)?;
        return Ok(());
    }

    let backup_path = target_path.with_extension("exe.old");
    let _ = fs::remove_file(&backup_path);
    fs::rename(target_path, &backup_path)?;

    if let Err(error) = fs::copy(update_file, target_path) {
        let _ = fs::rename(&backup_path, target_path);
        return Err(error);
    }

    // A running old executable may remain locked until this process exits.
    let _ = fs::remove_file(backup_path);
    Ok(())
}

fn print_update_success(target_path: &Path) {
    let version = get_installed_version().unwrap_or_else(|| "unknown".to_string());
    println!("Successfully updated to htop {}!", version);
    println!("Location: {}", target_path.display());
    println!("\nRestart htop to use the new version.");
}

fn remove_installed_update_file(update_file: &Path) {
    let update_root = update_root_path();
    let generation_dir = update_file.parent();
    let is_managed_generation = generation_dir
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("pending-"))
        && generation_dir.and_then(Path::parent) == Some(update_root.as_path());
    if is_managed_generation {
        let _ = fs::remove_dir_all(generation_dir.expect("generation directory was checked"));
    } else {
        let _ = fs::remove_file(update_file);
    }
}

/// Install an update from a downloaded file.
pub fn do_install_update(update_file: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let _lock = UpdateLock::acquire()?;
    let target_path = get_install_path()?;
    install_verified_update(
        update_file,
        &target_path,
        RevocationCheck::Online,
        VersionRule::NewerThanRunning,
    )?;
    remove_installed_update_file(update_file);
    print_update_success(&target_path);
    Ok(())
}

/// Update status for background updates
#[derive(Clone, Debug)]
pub enum UpdateStatus {
    /// A newer version is available and has been downloaded
    Downloaded { version: String, path: PathBuf },
    /// GitHub was reached successfully and no newer release exists
    UpToDate,
    /// The check or download failed; the application may retry later
    Failed(String),
}

/// Check for updates and download if available (for background auto-update)
/// Returns UpdateStatus indicating what happened
pub fn check_and_download_update() -> UpdateStatus {
    let current_version = env!("CARGO_PKG_VERSION");
    let (latest_version, download_url) = match get_latest_release() {
        Ok(v) => v,
        Err(error) => return UpdateStatus::Failed(error.to_string()),
    };

    if !is_newer_version(&latest_version, current_version) {
        if let Ok(_lock) = UpdateLock::acquire()
            && let Ok(updates) = load_pending_updates()
        {
            for update in updates {
                if !is_newer_version(&update.version, current_version) {
                    remove_pending_update(&update);
                }
            }
        }
        return UpdateStatus::UpToDate;
    }

    // Inspect existing generations only after the network request, under the
    // mutex. A pre-request snapshot can be consumed by another instance while
    // GitHub is in flight and must never be returned as if it still existed.
    {
        let _lock = match UpdateLock::acquire() {
            Ok(lock) => lock,
            Err(error) => {
                return UpdateStatus::Failed(format!("Failed to inspect pending updates: {error}"));
            }
        };
        match load_pending_updates() {
            Ok(updates) => {
                if let Some(update) = updates
                    .into_iter()
                    .find(|update| update.version == latest_version)
                {
                    return UpdateStatus::Downloaded {
                        version: update.version,
                        path: update.path,
                    };
                }
            }
            Err(error) => return UpdateStatus::Failed(error),
        }
    }

    let body = match native_http_get(&download_url) {
        Ok(body) => body,
        Err(error) => return UpdateStatus::Failed(format!("Update download failed: {error}")),
    };

    // Recheck under the publication lock: another instance may have staged
    // this release while the network request was in flight.
    let _lock = match UpdateLock::acquire() {
        Ok(lock) => lock,
        Err(error) => {
            return UpdateStatus::Failed(format!("Failed to stage update: {error}"));
        }
    };
    match load_pending_updates() {
        Ok(updates) => {
            if let Some(update) = updates
                .into_iter()
                .find(|update| update.version == latest_version)
            {
                return UpdateStatus::Downloaded {
                    version: update.version,
                    path: update.path,
                };
            }
        }
        Err(error) => return UpdateStatus::Failed(error),
    }
    match stage_pending_update(
        &latest_version,
        &body,
        VersionRule::Release(&latest_version),
    ) {
        Ok(update) => UpdateStatus::Downloaded {
            version: update.version,
            path: update.path,
        },
        Err(error) => UpdateStatus::Failed(error),
    }
}

fn pending_update_for_current_version() -> Result<Option<PendingUpdate>, String> {
    let updates = load_pending_updates()?;
    let pending = newest_pending_update(&updates);
    for update in updates {
        if pending
            .as_ref()
            .is_some_and(|pending| pending.path == update.path)
        {
            continue;
        }
        if !is_newer_version(&update.version, env!("CARGO_PKG_VERSION")) {
            remove_pending_update(&update);
        }
    }
    Ok(pending)
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

/// Result of the startup pass over staged updates.
///
/// `apply_pending_update` collapses this into a bool whose only meaning to its
/// caller is "the running executable was just replaced": the session then skips
/// its background update check because the stale binary would re-download what
/// it just installed, and suppresses further update notices. Everything that is
/// not a completed installation must report false, otherwise a transient
/// problem such as losing the race for the update mutex would silence update
/// checks for the whole session.
enum PendingUpdateOutcome {
    /// An update was installed over the running executable.
    Applied,
    /// No staged update existed.
    NothingPending,
    /// The pass could not be completed; the caller must run its normal update
    /// check instead of assuming the running binary is up to date.
    Incomplete(String),
}

impl PendingUpdateOutcome {
    fn update_just_applied(&self) -> bool {
        matches!(self, Self::Applied)
    }
}

/// Apply the staged update, requiring the caller to hold [`UpdateLock`] so a
/// concurrent instance cannot observe or consume the generation mid-install.
fn apply_pending_update_locked(current_exe: &Path) -> PendingUpdateOutcome {
    match pending_update_for_current_version() {
        Ok(Some(pending)) => {
            let verified = match open_verified_update(
                &pending.path,
                RevocationCheck::SkipAtStartup,
                VersionRule::Release(&pending.version),
            ) {
                Ok(file) => file,
                Err(error) => {
                    // Never keep a file that failed verification: the next
                    // online check downloads and verifies a fresh copy.
                    remove_pending_update(&pending);
                    return PendingUpdateOutcome::Incomplete(format!("update rejected: {error}"));
                }
            };
            let installed = install_update_file(&pending.path, current_exe);
            drop(verified);
            match installed {
                Ok(()) => {
                    remove_pending_update(&pending);
                    PendingUpdateOutcome::Applied
                }
                Err(error) => {
                    PendingUpdateOutcome::Incomplete(format!("installation failed: {error}"))
                }
            }
        }
        Ok(None) => {
            // Clean up any old backup files from previous updates.
            let backup_path = current_exe.with_extension("exe.old");
            let _ = fs::remove_file(&backup_path);
            PendingUpdateOutcome::NothingPending
        }
        Err(error) => {
            PendingUpdateOutcome::Incomplete(format!("cannot inspect update files: {error}"))
        }
    }
}

/// Check for and apply pending update on startup (call before UI starts)
///
/// Returns true only when an update was actually applied this session, telling
/// the caller to skip its own update check because the running binary is older
/// than the one now on disk. Lock timeouts, unreadable state and failed
/// installations all return false so the session still checks for updates.
pub fn apply_pending_update() -> bool {
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let outcome = match UpdateLock::acquire() {
        // Held across inspection, installation and publication.
        Ok(_lock) => apply_pending_update_locked(&current_exe),
        Err(error) => {
            PendingUpdateOutcome::Incomplete(format!("cannot acquire update lock: {error}"))
        }
    };

    match &outcome {
        PendingUpdateOutcome::Applied => eprintln!("Update applied successfully!"),
        PendingUpdateOutcome::Incomplete(error) => {
            eprintln!("Update not applied ({error}); will check for updates this session");
        }
        PendingUpdateOutcome::NothingPending => {}
    }

    outcome.update_just_applied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #82: only a completed installation may tell the caller that an
    /// update was just applied. Any other outcome must report false so the
    /// session still runs its background update check.
    #[test]
    fn only_a_completed_install_counts_as_just_applied() {
        assert!(PendingUpdateOutcome::Applied.update_just_applied());
        assert!(!PendingUpdateOutcome::NothingPending.update_just_applied());
        assert!(
            !PendingUpdateOutcome::Incomplete("timed out waiting for the update lock".into())
                .update_just_applied()
        );
    }

    /// Smallest buffer validate_pe_executable accepts: MZ header, e_lfanew
    /// pointing at a PE\0\0 signature, padded past MIN_UPDATE_SIZE.
    fn synthetic_pe() -> Vec<u8> {
        let mut body = vec![0u8; MIN_UPDATE_SIZE + 1024];
        body[0] = b'M';
        body[1] = b'Z';
        body[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        body[0x80..0x84].copy_from_slice(b"PE\0\0");
        body[0x84..0x86].copy_from_slice(&target_machine().to_le_bytes());
        body
    }

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("htop-win-test-{name}-{}", unique_generation_id()))
    }

    /// Stand-in for the Authenticode check in staging tests that are about
    /// publication, not signatures (synthetic PEs are unsigned).
    fn accept_unsigned(_: &Path) -> Result<(), String> {
        Ok(())
    }

    /// Issue #107: an executable that only passes the MZ/PE/machine checks
    /// must never be accepted as an update.
    #[test]
    fn unsigned_executable_fails_verification() {
        let directory = test_directory("unsigned");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(UPDATE_FILE_NAME);
        fs::write(&path, synthetic_pe()).unwrap();

        for revocation in [RevocationCheck::Online, RevocationCheck::SkipAtStartup] {
            let error = open_verified_update(&path, revocation, VersionRule::NewerThanRunning)
                .expect_err("an unsigned file must fail verification");
            assert!(error.contains("Authenticode"), "{error}");
        }
        fs::remove_dir_all(directory).unwrap();
    }

    /// A validly signed binary from another publisher must be rejected by the
    /// pin. This also exercises the WinVerifyTrust success path and the
    /// chain, RDN and EKU extraction, which unsigned fixtures cannot reach.
    /// Skips when none of the known embedded-signed binaries is installed
    /// (GitHub's Windows runners have both).
    #[test]
    fn validly_signed_binary_from_another_publisher_is_rejected() {
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        let Some(signed) = program_files
            .iter()
            .flat_map(|root| {
                [
                    root.join("PowerShell").join("7").join("pwsh.exe"),
                    root.join("Git").join("cmd").join("git.exe"),
                ]
            })
            .find(|path| path.is_file())
        else {
            eprintln!("skipped: no embedded-signed third-party binary found");
            return;
        };

        let error = open_verified_update(
            &signed,
            RevocationCheck::Online,
            VersionRule::NewerThanRunning,
        )
        .expect_err("a non-Fara signer must be rejected");
        assert!(
            error.starts_with("unexpected publisher") && !error.contains("CN=\"?\""),
            "{}: {error}",
            signed.display()
        );
    }

    /// The accept path, which no synthetic fixture reaches: the published
    /// v0.2.13 binary (signed by release.yml with the Faratech profile, its
    /// 3-day leaf certificate long expired, so the timestamp must carry it)
    /// must pass WinVerifyTrust, the chain/EKU pin and the VERSIONINFO
    /// identity checks, and the install copy must work while the verification
    /// handle denies writers. A one-byte change must fail verification.
    /// Needs the network, so it runs only on GitHub Actions, and never skips
    /// there.
    #[test]
    fn published_release_binary_is_accepted_and_a_modified_copy_is_not() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;

        if std::env::var_os("GITHUB_ACTIONS").is_none() {
            eprintln!("skipped: downloads a release asset; runs on GitHub Actions");
            return;
        }
        let directory = test_directory("release-asset");
        fs::create_dir_all(&directory).unwrap();
        let asset = directory.join("htop-win-amd64.exe");
        let status = std::process::Command::new("curl.exe")
            .args(["--fail", "--silent", "--show-error", "--location"])
            .args(["--retry", "3", "--output"])
            .arg(&asset)
            .arg(
                "https://github.com/faratech/htop-win/releases/download/v0.2.13/htop-win-amd64.exe",
            )
            .status()
            .expect("run curl.exe");
        assert!(status.success(), "download failed: {status}");

        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&asset)
            .unwrap();
        for revocation in [RevocationCheck::Online, RevocationCheck::SkipAtStartup] {
            let chain = authenticode_signer_chain(&asset, &held, revocation)
                .unwrap_or_else(|error| panic!("{revocation:?}: {error}"));
            assert_eq!(
                update_trust::check_signer_chain(&chain),
                Ok(()),
                "{chain:?}"
            );
        }
        let strings = read_version_strings(&asset).expect("version resource");
        assert_eq!(
            update_trust::check_update_identity(
                &strings,
                VersionRule::Release("0.2.13"),
                "0.2.12",
                is_newer_version
            ),
            Ok(()),
            "{strings:?}"
        );

        // Writers are refused while the handle is held; the install copy is not.
        assert!(
            fs::OpenOptions::new().write(true).open(&asset).is_err(),
            "a writer opened the file during verification"
        );
        let target = directory.join("installed").join("htop.exe");
        install_update_file(&asset, &target).expect("copy while the handle is held");
        drop(held);
        assert_eq!(fs::read(&target).unwrap(), fs::read(&asset).unwrap());

        let mut modified = fs::read(&asset).unwrap();
        modified[0x1000] ^= 0x01;
        let modified_path = directory.join("modified.exe");
        fs::write(&modified_path, &modified).unwrap();
        let error = open_verified_update(
            &modified_path,
            RevocationCheck::SkipAtStartup,
            VersionRule::NewerThanRunning,
        )
        .expect_err("a modified release binary must be rejected");
        assert!(error.contains("Authenticode"), "{error}");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stage_publishes_nothing_when_verification_fails() {
        let root = test_directory("rejected-stage");
        let error = stage_pending_update_in(&root, "1.2.3", &synthetic_pe(), |_| {
            Err("not signed by the release publisher".into())
        })
        .expect_err("a rejected update must not be staged");

        assert!(error.contains("downloaded update rejected"), "{error}");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
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
        assert!(is_newer_version("1.0.0", "1.0.0-rc.1"));
        assert!(is_newer_version("1.0.0-beta.11", "1.0.0-beta.2"));
        assert!(!is_newer_version("0.1.10", "0.2.0"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0+build.2", "1.0.0+build.1"));
        assert!(!is_newer_version("garbage", "1.0.0"));

        let precedence = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ];
        for versions in precedence.windows(2) {
            assert!(is_newer_version(versions[1], versions[0]));
        }
    }

    #[test]
    fn test_parse_version_rejects_malformed_semver() {
        for version in [
            "1.2.3junk",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2",
            "1.2.3-01",
            "1.2.3-",
            "1.2.3+",
            "vv1.2.3",
        ] {
            assert!(parse_version(version).is_none(), "accepted {version}");
        }
    }

    #[test]
    fn test_parse_version_accepts_prerelease_and_build_metadata() {
        assert!(parse_version("1.2.3-beta.1+build.7").is_some());
        assert!(parse_version("v1.2.3-rc.1").is_some());
    }

    #[test]
    fn test_paths_refer_to_same_file_detects_hard_link() {
        let directory = test_directory("same-file");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.exe");
        let alias = directory.join("alias.exe");
        fs::write(&source, b"same file").unwrap();
        fs::hard_link(&source, &alias).unwrap();

        assert!(paths_refer_to_same_file(&source, &alias).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn test_stage_pending_update_publishes_complete_pair() {
        let root = test_directory("atomic-stage");
        let update = stage_pending_update_in(
            &root,
            "1.2.3-beta.1+build.7",
            &synthetic_pe(),
            accept_unsigned,
        )
        .expect("stage should succeed");

        assert_eq!(
            validate_pending_pair(
                &update.path,
                &update
                    .generation_dir
                    .as_ref()
                    .unwrap()
                    .join(UPDATE_METADATA_NAME),
            )
            .unwrap(),
            "1.2.3-beta.1+build.7"
        );
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".stage-")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_concurrent_stages_do_not_overwrite_each_other() {
        let root = test_directory("concurrent-stage");
        let body = synthetic_pe();
        let handles = (0..4)
            .map(|index| {
                let root = root.clone();
                let body = body.clone();
                std::thread::spawn(move || {
                    stage_pending_update_in(
                        &root,
                        &format!("1.2.{}", index + 3),
                        &body,
                        accept_unsigned,
                    )
                    .expect("concurrent stage should succeed")
                })
            })
            .collect::<Vec<_>>();
        let updates = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(updates.len(), 4);
        assert!(updates.iter().all(|update| update.path.exists()));
        let mut paths = updates
            .iter()
            .map(|update| update.path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_invalid_generation_cleanup_preserves_valid_pending_update() {
        let root = test_directory("preserve-valid");
        let valid =
            stage_pending_update_in(&root, "1.2.3", &synthetic_pe(), accept_unsigned).unwrap();
        let invalid =
            stage_pending_update_in(&root, "1.2.4", &synthetic_pe(), accept_unsigned).unwrap();
        fs::write(&invalid.path, b"truncated").unwrap();

        let updates = load_pending_generations(&root).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].path, valid.path);
        assert!(valid.path.exists());
        assert!(!invalid.generation_dir.unwrap().exists());
        fs::remove_dir_all(root).unwrap();
    }
}
