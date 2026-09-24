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

#[cfg(windows)]
pub mod installer;
#[cfg(not(windows))]
#[path = "installer_stub.rs"]
pub mod installer;
