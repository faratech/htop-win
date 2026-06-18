use std::path::PathBuf;

#[derive(Clone)]
pub enum UpdateStatus {
    Downloaded { version: String, path: PathBuf },
    None,
}

pub fn install_to_path(_force: bool) -> Result<(), Box<dyn std::error::Error>> {
    Err("Installation is only available on Windows".into())
}

pub fn update_from_github(_force: bool) -> Result<(), Box<dyn std::error::Error>> {
    Err("Self-update is only available on Windows".into())
}

pub fn apply_pending_update() -> bool {
    false
}

pub fn spawn_update_check() -> std::sync::mpsc::Receiver<UpdateStatus> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = tx.send(UpdateStatus::None);
    rx
}
