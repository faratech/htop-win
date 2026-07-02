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
    let numeric = version
        .split('.')
        .map(|part| {
            part.parse::<u16>()
                .expect("package version parts must be numeric")
                .to_string()
        })
        .chain(std::iter::repeat("0".to_string()))
        .take(4)
        .collect::<Vec<_>>()
        .join(",");
    // Forward slashes work in quoted rc paths for both rc.exe and windres.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is set by Cargo")
        .replace('\\', "/");
    let template =
        std::fs::read_to_string("media/htop.rc").expect("failed to read media/htop.rc");

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
