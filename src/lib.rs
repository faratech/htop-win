pub mod app;
pub mod config;
pub mod data;
pub mod json;
pub mod system;
pub mod terminal;
pub mod ui;

#[cfg(windows)]
pub mod installer;
#[cfg(not(windows))]
#[path = "installer_stub.rs"]
pub mod installer;
