//! Build script for htop-win
//! Embeds the application icon and version info on Windows

fn main() {
    println!("cargo:rerun-if-changed=media/htop.rc");
    println!("cargo:rerun-if-changed=media/htop.ico");
    // Emitting rerun-if-changed disables the default rerun-on-any-change
    // behavior, so a Cargo.toml version bump alone would not regenerate the
    // resource — track the version explicitly.
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    // Check if we're targeting Windows (works for cross-compilation)
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let rc_path = write_versioned_rc();
        embed_resource::compile(&rc_path, embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed Windows resources");
    }
}

/// Generate a copy of media/htop.rc in OUT_DIR with the version fields filled
/// in from Cargo.toml, so the resource metadata can never go stale (it used to
/// be hand-maintained, and a missed bump broke the build). The icon path is
/// made absolute because the generated copy no longer lives next to the icon.
fn write_versioned_rc() -> std::path::PathBuf {
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by Cargo");
    let numeric = windows_resource_version(&version).unwrap_or_else(|error| panic!("{error}"));
    // Forward slashes work in quoted rc paths for both rc.exe and windres.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is set by Cargo")
        .replace('\\', "/");
    let template = std::fs::read_to_string("media/htop.rc").expect("failed to read media/htop.rc");

    let rc: String = template
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("FILEVERSION") {
                format!("FILEVERSION     {numeric}")
            } else if trimmed.starts_with("PRODUCTVERSION") {
                format!("PRODUCTVERSION  {numeric}")
            } else if trimmed.starts_with("VALUE \"FileVersion\"") {
                format!("            VALUE \"FileVersion\",      \"{version}\"")
            } else if trimmed.starts_with("VALUE \"ProductVersion\"") {
                format!("            VALUE \"ProductVersion\",   \"{version}\"")
            } else if trimmed.starts_with("1 ICON") {
                format!("1 ICON \"{manifest_dir}/media/htop.ico\"")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join("htop.rc");
    std::fs::write(&out, rc).expect("failed to write versioned htop.rc");
    out
}

/// Convert Cargo SemVer to the four numeric fields supported by VERSIONINFO.
/// Cargo has already validated the full version, including prerelease and build
/// identifiers; Windows only accepts the three numeric core components.
fn windows_resource_version(version: &str) -> Result<String, String> {
    let without_build = version.split_once('+').map_or(version, |(core, _)| core);
    let core = without_build
        .split_once('-')
        .map_or(without_build, |(core, _)| core);
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Err(format!(
            "package version '{version}' does not have a three-part SemVer core"
        ));
    }

    let mut numeric = Vec::with_capacity(4);
    for part in parts {
        let component = part.parse::<u16>().map_err(|_| {
            format!(
                "SemVer core component '{part}' in '{version}' exceeds the Windows resource limit of 65535"
            )
        })?;
        numeric.push(component.to_string());
    }
    numeric.push("0".to_string());
    Ok(numeric.join(","))
}

#[cfg(test)]
mod tests {
    use super::windows_resource_version;

    #[test]
    fn resource_version_uses_semver_core() {
        assert_eq!(windows_resource_version("1.2.3").unwrap(), "1,2,3,0");
        assert_eq!(
            windows_resource_version("1.2.3-beta.1+build.7").unwrap(),
            "1,2,3,0"
        );
    }

    #[test]
    fn resource_version_rejects_oversized_components() {
        let error = windows_resource_version("1.65536.3").unwrap_err();
        assert!(error.contains("65535"));
    }
}
