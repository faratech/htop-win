//! Build script for htop-win
//! Embeds the application icon on Windows

fn main() {
    // Check if we're targeting Windows (works for cross-compilation)
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let _ = embed_resource::compile("media/htop.rc", embed_resource::NONE);
    }
}
