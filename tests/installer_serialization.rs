//! Isolate LOCALAPPDATA overrides in a test executable with only this test.
#![cfg(windows)]

use htop_win::installer::{get_install_path, install_to_path};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

struct InstallFixture {
    root: PathBuf,
    original_local_app_data: Option<OsString>,
}

impl Drop for InstallFixture {
    fn drop(&mut self) {
        // All worker threads have been joined before the fixture is dropped.
        unsafe {
            match &self.original_local_app_data {
                Some(value) => std::env::set_var("LOCALAPPDATA", value),
                None => std::env::remove_var("LOCALAPPDATA"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn concurrent_install_commands_publish_only_complete_fixture_files() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = InstallFixture {
        root: std::env::temp_dir().join(format!(
            "htop-install-serialization-{}-{stamp}",
            std::process::id()
        )),
        original_local_app_data: std::env::var_os("LOCALAPPDATA"),
    };
    // This executable contains one test; no workers exist until after the
    // override. Each real public installer call targets only this fixture.
    unsafe { std::env::set_var("LOCALAPPDATA", &fixture.root) };
    let target = get_install_path().unwrap();
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"previous fixture executable").unwrap();

    // Keep the queue within the production mutex's five-second wait budget:
    // copying this debug test executable over WSL can take over a second.
    let workers = 2;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                install_to_path(true).map_err(|error| error.to_string())
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join())
        .collect::<Vec<_>>();
    for result in results {
        result.unwrap().unwrap();
    }
    assert_eq!(
        std::fs::read(&target).unwrap(),
        std::fs::read(std::env::current_exe().unwrap()).unwrap()
    );
    assert_eq!(
        std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
        1
    );
}
