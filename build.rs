//! Build script for htop-win
//! Embeds the application icon on Windows

fn main() {
    // Only compile resources on Windows
    #[cfg(windows)]
    {
        let _ = embed_resource::compile("media/htop.rc", embed_resource::NONE);
    }
}
