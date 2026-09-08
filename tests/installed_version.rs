//! One test in a separate process so LOCALAPPDATA overrides are isolated.
use htop_win::installer::get_installed_version;
use std::path::PathBuf;

#[test]
fn installed_version_reads_real_pe_metadata_without_executing_it() {
    let mut source = PathBuf::from(env!("CARGO_BIN_EXE_htop-win"));
    if !source.exists() {
        // WSL cross-tests embed a Linux path; resolve its Windows sibling.
        source = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("htop-win.exe");
    }
    let root = std::env::temp_dir().join(format!("htop-version-test-{}", std::process::id()));
    let fixture = root.join("Microsoft/WindowsApps/htop.exe");
    std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
    std::fs::copy(source, &fixture).unwrap();
    let original = std::env::var_os("LOCALAPPDATA");
    // This integration executable contains only this test, and spawns no workers.
    unsafe {
        std::env::set_var("LOCALAPPDATA", &root);
    }
    let expected = env!("CARGO_PKG_VERSION").split(['-', '+']).next().unwrap();
    assert_eq!(get_installed_version().as_deref(), Some(expected));
    // Cargo test executables are valid PEs without the application's version resource.
    std::fs::copy(std::env::current_exe().unwrap(), &fixture).unwrap();
    assert!(get_installed_version().is_none());
    std::fs::write(&fixture, b"not a PE").unwrap();
    assert!(get_installed_version().is_none());
    std::fs::remove_file(&fixture).unwrap();
    assert!(get_installed_version().is_none());
    unsafe {
        if let Some(original) = original {
            std::env::set_var("LOCALAPPDATA", original);
        } else {
            std::env::remove_var("LOCALAPPDATA");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
