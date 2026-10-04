#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod calibration;
pub mod config;
pub mod data;
pub mod domain;
pub mod game;
#[cfg(windows)]
pub mod installer;
pub mod model;
#[cfg(windows)]
pub mod native;
#[cfg(windows)]
pub(crate) mod ocr_cache;
#[cfg(windows)]
pub mod recognition;
#[cfg(windows)]
pub(crate) mod scan_policy;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod update;
pub(crate) mod update_package;
#[cfg(windows)]
pub(crate) mod visual_gate;
