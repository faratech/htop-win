//! Build script for htop-win
//! Embeds the application icon on Windows

fn main() {
    println!("cargo:rerun-if-changed=media/htop.rc");

    // Check if we're targeting Windows (works for cross-compilation)
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        validate_resource_version();
        embed_resource::compile("media/htop.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed Windows resources from media/htop.rc");
    }
}

fn validate_resource_version() {
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by Cargo");
    let numeric = version
        .split('.')
        .map(|part| {
            part.parse::<u16>()
                .expect("package version parts must be numeric")
        })
        .chain(std::iter::repeat(0))
        .take(4)
        .map(|part| part.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let rc = std::fs::read_to_string("media/htop.rc")
        .expect("failed to read media/htop.rc for version validation");

    for expected in [
        format!("FILEVERSION     {numeric}"),
        format!("PRODUCTVERSION  {numeric}"),
        format!("VALUE \"FileVersion\",      \"{version}\""),
        format!("VALUE \"ProductVersion\",   \"{version}\""),
    ] {
        assert!(
            rc.contains(&expected),
            "media/htop.rc version metadata is stale; expected `{expected}`"
        );
    }
}
