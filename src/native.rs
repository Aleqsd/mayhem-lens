//! Windows integration. No function is invoked merely by loading this module.
//! Capture and OCR run on the caller's worker; the window loop owns every HWND.

use std::sync::{
    Arc,
    atomic::AtomicBool,
    mpsc::{Receiver, Sender},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub text: String,
    /// Physical screen pixels, including the game's current monitor origin.
    pub rect: Rect,
}

#[derive(Clone, Debug)]
pub struct CaptureReading {
    pub observations: Vec<Observation>,
    pub game_bounds: Rect,
    pub calibration: crate::calibration::CalibrationSnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    /// Desired badge bounds in physical screen pixels.
    pub rect: Rect,
    pub title: String,
    pub detail: String,
    /// Font scale; the caller already scales the physical geometry.
    pub scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct OverlayFrame {
    pub badges: Vec<Badge>,
    /// Geometry of the capture that produced these badges; None clears display.
    pub game_bounds: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserAction {
    ForceScan,
    /// The user records their own choice; no game input is generated.
    SelectSlot(u8),
    /// Explicit user declaration; None preserves an unknown stage.
    SetStage(Option<u8>),
    SetAutoStage(bool),
}

#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::{
    InstanceGuard, acquire_single_instance, calibrate_offers, diagnostics, ensure_ready,
    game_window_visible, invalidate_observations, observe_game, package_data_directory,
    reading_is_current, reset_calibration,
};

#[cfg(windows)]
pub fn run_overlay(
    receiver: Receiver<OverlayFrame>,
    actions: Sender<UserAction>,
    stop: Arc<AtomicBool>,
    config_path: &std::path::Path,
    updates: crate::update::UpdateController,
) -> anyhow::Result<()> {
    platform::run_overlay(receiver, actions, stop, config_path, updates)
}

#[cfg(not(windows))]
pub fn package_data_directory() -> Option<std::path::PathBuf> {
    None
}

#[cfg(not(windows))]
pub fn diagnostics() -> anyhow::Result<String> {
    Ok("Capture, OCR et overlay natifs disponibles uniquement sous Windows.".into())
}

#[cfg(not(windows))]
pub fn ensure_ready(_: &str) -> anyhow::Result<()> {
    anyhow::bail!("L'application native nécessite Windows.")
}

#[cfg(not(windows))]
pub fn game_window_visible() -> bool {
    false
}

#[cfg(not(windows))]
pub fn invalidate_observations() {}

#[cfg(not(windows))]
pub fn calibrate_offers(_: &[Rect]) {}

#[cfg(not(windows))]
pub fn reset_calibration() {}

#[cfg(not(windows))]
pub fn reading_is_current(_: Rect) -> bool {
    false
}

#[cfg(not(windows))]
pub struct InstanceGuard;

#[cfg(not(windows))]
pub fn acquire_single_instance() -> anyhow::Result<InstanceGuard> {
    anyhow::bail!("L'application native nécessite Windows.")
}

#[cfg(not(windows))]
pub fn observe_game(_: &str) -> anyhow::Result<CaptureReading> {
    anyhow::bail!("La capture native nécessite Windows.")
}

#[cfg(not(windows))]
pub fn run_overlay(
    _: Receiver<OverlayFrame>,
    _: Sender<UserAction>,
    _: Arc<AtomicBool>,
    _: &std::path::Path,
    _: crate::update::UpdateController,
) -> anyhow::Result<()> {
    anyhow::bail!("L'overlay natif nécessite Windows.")
}
