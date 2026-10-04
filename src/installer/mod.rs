//! Standalone installer. Opening its window is read-only; installation and
//! certificate approval require the corresponding explicit user action.

pub mod engine;
pub mod ui;

include!(concat!(env!("OUT_DIR"), "/setup_payload.rs"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DISPLAY_NAME: &str = "Mayhem Lens";
pub const PACKAGE_NAME: &str = "Aleqsd.MayhemLens";
pub const PUBLISHER: &str = "CN=Alexandre DO-O ALMEIDA";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ready,
    Checking,
    Authorizing,
    Installing,
    Complete,
    Deferred,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ViewState {
    pub phase: Phase,
    pub progress: u8,
    pub status: String,
    pub detail: String,
    pub certificate_trusted: bool,
    pub ocr_available: bool,
    pub installed_version: Option<String>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            phase: Phase::Checking,
            progress: 0,
            status: "Vérification de Windows…".into(),
            detail: "Aucune installation n'a encore été demandée.".into(),
            certificate_trusted: false,
            ocr_available: false,
            installed_version: None,
        }
    }
}
