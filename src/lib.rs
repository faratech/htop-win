pub mod app;
pub mod config;
pub mod data;
pub mod event_wait;
pub mod input;
pub mod json;
pub(crate) mod numfmt;
pub mod system;
pub mod terminal;
pub mod ui;
// Used by the Windows-only installer; compiled everywhere so its tests run on any host.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod update_trust;

#[cfg(windows)]
pub mod installer;
#[cfg(not(windows))]
#[path = "installer_stub.rs"]
pub mod installer;
