//! Preferences editing is explicit and local. Loading this module creates no UI.
use crate::config::{self, Config};
use anyhow::Result;
use std::path::Path;

/// Merge only editable preferences into the latest file. Session choices and
/// personal rules may have changed while the settings window was open.
pub fn save_preferences(path: &Path, preferences: &Config) -> Result<Config> {
    save_preferences_with_stage(path, preferences, true)
}

pub fn save_preferences_with_stage(
    path: &Path,
    preferences: &Config,
    stage_edited: bool,
) -> Result<Config> {
    preferences.validate()?;
    config::modify(path, |latest| {
        latest.language = preferences.language.clone();
        latest.scan_interval_ms = preferences.scan_interval_ms;
        if stage_edited {
            latest.offer_stage = preferences.offer_stage;
            latest.auto_stage = preferences.auto_stage;
        }
        latest.show_builds = preferences.show_builds;
        latest.minimum_match_confidence = preferences.minimum_match_confidence;
        latest.overlay_scale = preferences.overlay_scale;
        latest.overlay_opacity = preferences.overlay_opacity;
        latest.overlay_offset_x = preferences.overlay_offset_x;
        latest.overlay_offset_y = preferences.overlay_offset_y;
        latest.shortcuts = preferences.shortcuts.clone();
        Ok(())
    })
}

pub use window::SettingsWindow;

mod window {
    use super::*;
    use crate::{config::ShortcutConfig, update::UpdateController};
    use anyhow::{Context, ensure};
    use std::{path::PathBuf, sync::OnceLock};
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
            Graphics::Gdi::{
                CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
                DEFAULT_QUALITY, DeleteObject, HBRUSH, HFONT, OUT_DEFAULT_PRECIS,
            },
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow},
                Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus, VK_ESCAPE, VK_RETURN},
                WindowsAndMessaging::*,
            },
        },
        core::{HSTRING, w},
    };

    const SAVE: u32 = 1;
    const CANCEL: u32 = 2;
    const DEFAULTS: u32 = 3;
    const GENERAL: u32 = 10;
    const SHORTCUTS: u32 = 11;
    const UPDATES: u32 = 12;
    const LANGUAGE: u32 = 20;
    const INTERVAL: u32 = 21;
    const CONFIDENCE: u32 = 22;
    const BUILDS: u32 = 23;
    const AUTO_STAGE: u32 = 24;
    const STAGE: u32 = 25;
    const SCALE: u32 = 26;
    const OPACITY: u32 = 27;
    const OFFSET_X: u32 = 28;
    const OFFSET_Y: u32 = 29;
    const SCAN_KEY: u32 = 30;
    const QUIT_KEY: u32 = 31;
    const SLOT_1_KEY: u32 = 32;
    const SLOT_2_KEY: u32 = 33;
    const SLOT_3_KEY: u32 = 34;
    const UPDATE_TEXT: u32 = 40;
    const UPDATE_CHECK: u32 = 41;
    const UPDATE_PREPARE: u32 = 42;
    const ERROR_TEXT: u32 = 50;
    const COMMAND_EVENT: u32 = WM_APP + 77;
    const DPI_EVENT: u32 = WM_APP + 78;
    const UPDATE_EVENT: u32 = WM_APP + 79;
    const CLOSE_EVENT: u32 = WM_APP + 80;
    const CLIENT_WIDTH: i32 = 560;
    const CLIENT_HEIGHT: i32 = 440;

    #[derive(Clone, Copy)]
    struct Bounds {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    struct Control {
        hwnd: HWND,
        id: u32,
        page: Option<u8>,
        bounds: Bounds,
    }

    pub struct SettingsWindow {
        hwnd: HWND,
        path: PathBuf,
        updates: UpdateController,
        controls: Vec<Control>,
        font: HFONT,
        dpi: u32,
        page: u8,
        english: bool,
        saved: bool,
        stage_edited: bool,
        last_update_text: String,
    }

    // This procedure keeps no Rust state pointer. Synchronous control callbacks
    // only post work to the owning loop, avoiding reentrant mutable references.
    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        match message {
            WM_COMMAND => {
                let id = wp.0 as u32 & 0xffff;
                let notification = (wp.0 >> 16) as u32 & 0xffff;
                if notification == 0 || (id == STAGE && notification == CBN_SELCHANGE) {
                    let _ = unsafe {
                        PostMessageW(Some(hwnd), COMMAND_EVENT, WPARAM(id as usize), LPARAM(0))
                    };
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = unsafe { PostMessageW(Some(hwnd), CLOSE_EVENT, WPARAM(0), LPARAM(0)) };
                LRESULT(0)
            }
            WM_DPICHANGED => {
                // SAFETY: Windows owns this RECT for the duration of this call.
                let rect = unsafe { &*(lp.0 as *const RECT) };
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                };
                let _ = unsafe { PostMessageW(Some(hwnd), DPI_EVENT, WPARAM(0), LPARAM(0)) };
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = unsafe { PostMessageW(Some(hwnd), UPDATE_EVENT, WPARAM(0), LPARAM(0)) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wp, lp) },
        }
    }

    impl SettingsWindow {
        /// Called exclusively by the user-selected tray action, on the UI thread.
        pub fn open(owner: HWND, path: &Path, updates: UpdateController) -> Result<Self> {
            static CLASS: OnceLock<bool> = OnceLock::new();
            let instance = HINSTANCE(unsafe { GetModuleHandleW(None)? }.0);
            ensure!(
                *CLASS.get_or_init(|| {
                    // SAFETY: process-lifetime class procedure, no captured state.
                    unsafe {
                        RegisterClassW(&WNDCLASSW {
                            lpfnWndProc: Some(procedure),
                            hInstance: instance,
                            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as usize as *mut _),
                            lpszClassName: w!("MayhemLensSettings"),
                            ..Default::default()
                        }) != 0
                    }
                }),
                "Impossible d'enregistrer la fenêtre de réglages"
            );
            let config = Config::load(path)?;
            let english = config.language == "en";
            let dpi = unsafe { GetDpiForSystem() }.max(96);
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: scaled(CLIENT_WIDTH, dpi),
                bottom: scaled(CLIENT_HEIGHT, dpi),
            };
            unsafe { AdjustWindowRectExForDpi(&mut rect, style, false, WS_EX_CONTROLPARENT, dpi) }?;
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_CONTROLPARENT,
                    w!("MayhemLensSettings"),
                    &HSTRING::from(if english {
                        "Mayhem Lens — Settings"
                    } else {
                        "Mayhem Lens — Réglages"
                    }),
                    style,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    Some(owner),
                    None,
                    Some(instance),
                    None,
                )
            }?;
            let mut window = Self {
                hwnd,
                path: path.to_owned(),
                updates,
                controls: Vec::new(),
                font: HFONT::default(),
                dpi,
                page: 0,
                english,
                saved: false,
                stage_edited: false,
                last_update_text: String::new(),
            };
            window.dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            window.create_controls()?;
            window.populate(&config)?;
            window.layout()?;
            window.show_page(0);
            window.update_status()?;
            if crate::native::ensure_ready(&config.language).is_err() {
                window.show_error(choose(english,
                    "OCR indisponible : choisir une autre langue ou installer sa fonctionnalité OCR dans Windows.",
                    "OCR unavailable: choose another language or install its Windows OCR feature."));
            }
            unsafe {
                ensure!(
                    SetTimer(Some(hwnd), 1, 500, None) != 0,
                    "Timer des réglages indisponible"
                );
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(window.control(LANGUAGE)?));
            }
            Ok(window)
        }

        pub fn show(&self) {
            // This method is also called only after the explicit menu action.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(self.hwnd);
            }
        }

        pub fn is_open(&self) -> bool {
            !self.hwnd.0.is_null()
        }
        pub fn take_saved(&mut self) -> bool {
            std::mem::take(&mut self.saved)
        }

        /// The native overlay loop continues while this modeless window is open.
        pub fn process_message(&mut self, message: &MSG) -> bool {
            if !self.is_open() {
                return false;
            }
            if message.hwnd == self.hwnd {
                let result = match message.message {
                    COMMAND_EVENT => self.command(message.wParam.0 as u32),
                    DPI_EVENT => {
                        self.dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
                        self.layout()
                    }
                    UPDATE_EVENT => self.update_status(),
                    CLOSE_EVENT => {
                        self.close();
                        Ok(())
                    }
                    _ => return self.keyboard(message),
                };
                if let Err(error) = result {
                    self.show_error(&format!("{error:#}"));
                }
                return true;
            }
            self.keyboard(message)
        }

        fn keyboard(&mut self, message: &MSG) -> bool {
            // Only our window and child controls participate in dialog navigation.
            if message.hwnd != self.hwnd && !unsafe { IsChild(self.hwnd, message.hwnd) }.as_bool() {
                return false;
            }
            if message.message == WM_KEYDOWN {
                if message.wParam.0 == VK_ESCAPE.0 as usize {
                    self.close();
                    return true;
                }
                if message.wParam.0 == VK_RETURN.0 as usize {
                    let focus = unsafe { GetFocus() };
                    let mut name = [0u16; 32];
                    let length = unsafe { GetClassNameW(focus, &mut name) }.max(0) as usize;
                    let class = String::from_utf16_lossy(&name[..length]);
                    if class == "ComboBox"
                        && unsafe { SendMessageW(focus, CB_GETDROPPEDSTATE, None, None) }.0 != 0
                    {
                        return false;
                    }
                    if class == "Button" {
                        unsafe {
                            SendMessageW(focus, BM_CLICK, None, None);
                        }
                    } else if let Err(error) = self.command(SAVE) {
                        self.show_error(&format!("{error:#}"));
                    }
                    return true;
                }
            }
            unsafe { IsDialogMessageW(self.hwnd, message) }.as_bool()
        }

        fn command(&mut self, id: u32) -> Result<()> {
            match id {
                SAVE => {
                    let preferences = self.read_preferences()?;
                    save_preferences_with_stage(&self.path, &preferences, self.stage_edited)?;
                    self.saved = true;
                    self.close();
                }
                CANCEL => self.close(),
                DEFAULTS => {
                    self.stage_edited = true;
                    self.populate(&Config::default())?;
                    self.show_error("");
                }
                GENERAL..=UPDATES => self.show_page((id - GENERAL) as u8),
                AUTO_STAGE => {
                    self.stage_edited = true;
                    self.enable_manual_stage()?;
                }
                STAGE => {
                    self.stage_edited = true;
                    self.set_check(AUTO_STAGE, false)?;
                    self.enable_manual_stage()?;
                }
                UPDATE_CHECK => {
                    let _ = self.updates.request_check();
                    self.update_status()?;
                }
                UPDATE_PREPARE => {
                    let _ = self.updates.request_update();
                    self.update_status()?;
                }
                _ => {}
            }
            Ok(())
        }

        fn show_error(&self, text: &str) {
            if let Ok(hwnd) = self.control(ERROR_TEXT) {
                let _ = unsafe { SetWindowTextW(hwnd, &HSTRING::from(text)) };
            }
        }

        fn update_status(&mut self) -> Result<()> {
            let language = if self.english { "en" } else { "fr" };
            let status = self.updates.status();
            let text = status.display_lines(language).join("\r\n");
            if text != self.last_update_text {
                unsafe { SetWindowTextW(self.control(UPDATE_TEXT)?, &HSTRING::from(&text)) }?;
                self.last_update_text = text;
            }
            unsafe {
                let _ = EnableWindow(self.control(UPDATE_CHECK)?, !status.busy());
                let _ = EnableWindow(self.control(UPDATE_PREPARE)?, !status.busy());
            }
            Ok(())
        }

        fn control(&self, id: u32) -> Result<HWND> {
            self.controls
                .iter()
                .find(|control| control.id == id)
                .map(|control| control.hwnd)
                .context("Contrôle de réglage manquant")
        }

        fn create(
            &mut self,
            id: u32,
            page: Option<u8>,
            class: &str,
            text: &str,
            bounds: Bounds,
            style: WINDOW_STYLE,
        ) -> Result<()> {
            let hwnd = unsafe {
                CreateWindowExW(
                    if class == "EDIT" {
                        WS_EX_CLIENTEDGE
                    } else {
                        WINDOW_EX_STYLE(0)
                    },
                    &HSTRING::from(class),
                    &HSTRING::from(text),
                    WS_CHILD | WS_VISIBLE | style,
                    scaled(bounds.x, self.dpi),
                    scaled(bounds.y, self.dpi),
                    scaled(bounds.width, self.dpi),
                    scaled(bounds.height, self.dpi),
                    Some(self.hwnd),
                    Some(HMENU(id as usize as *mut _)),
                    Some(HINSTANCE(GetModuleHandleW(None)?.0)),
                    None,
                )
            }?;
            self.controls.push(Control {
                hwnd,
                id,
                page,
                bounds,
            });
            if class == "EDIT" {
                unsafe {
                    SendMessageW(hwnd, 0x00c5, Some(WPARAM(64)), None);
                }
            } // EM_LIMITTEXT
            Ok(())
        }

        fn create_controls(&mut self) -> Result<()> {
            for (id, x, french, english) in [
                (GENERAL, 16, "&Général", "&General"),
                (SHORTCUTS, 192, "&Raccourcis", "&Shortcuts"),
                (UPDATES, 368, "&Mises à jour", "&Updates"),
            ] {
                self.create(
                    id,
                    None,
                    "BUTTON",
                    choose(self.english, french, english),
                    Bounds {
                        x,
                        y: 14,
                        width: 168,
                        height: 30,
                    },
                    WS_TABSTOP,
                )?;
            }
            for (id, x, y, french, english) in [
                (LANGUAGE, 16, 60, "&Langue", "&Language"),
                (
                    INTERVAL,
                    288,
                    60,
                    "Lecture, ms (400–5000)",
                    "Scan, ms (400–5000)",
                ),
                (
                    CONFIDENCE,
                    16,
                    112,
                    "Similarité du nom (0,90–1,00)",
                    "Name similarity (0.90–1.00)",
                ),
                (STAGE, 288, 164, "&Stade manuel", "&Manual stage"),
                (SCALE, 16, 216, "Taille (0,75–1,50)", "Scale (0.75–1.50)"),
                (
                    OPACITY,
                    288,
                    216,
                    "Opacité, % (30–100)",
                    "Opacity, % (30–100)",
                ),
                (
                    OFFSET_X,
                    16,
                    268,
                    "Décalage &X, pixels",
                    "&X offset, pixels",
                ),
                (
                    OFFSET_Y,
                    288,
                    268,
                    "Décalage &Y, pixels",
                    "&Y offset, pixels",
                ),
            ] {
                self.create(
                    100 + id,
                    Some(0),
                    "STATIC",
                    choose(self.english, french, english),
                    Bounds {
                        x,
                        y,
                        width: 252,
                        height: 20,
                    },
                    WINDOW_STYLE(0),
                )?;
                let class = if matches!(id, LANGUAGE | STAGE) {
                    "COMBOBOX"
                } else {
                    "EDIT"
                };
                let style = WS_TABSTOP
                    | if class == "COMBOBOX" {
                        WINDOW_STYLE(CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0)
                    } else {
                        WINDOW_STYLE(ES_AUTOHSCROLL as u32)
                    };
                self.create(
                    id,
                    Some(0),
                    class,
                    "",
                    Bounds {
                        x,
                        y: y + 21,
                        width: 252,
                        height: if class == "COMBOBOX" { 150 } else { 25 },
                    },
                    style,
                )?;
            }
            self.create(
                BUILDS,
                Some(0),
                "BUTTON",
                choose(self.english, "Afficher les &builds", "Show &builds"),
                Bounds {
                    x: 288,
                    y: 137,
                    width: 252,
                    height: 25,
                },
                WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
            )?;
            self.create(
                AUTO_STAGE,
                Some(0),
                "BUTTON",
                choose(self.english, "Stade &automatique", "&Automatic stage"),
                Bounds {
                    x: 16,
                    y: 186,
                    width: 252,
                    height: 25,
                },
                WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
            )?;
            self.create(160, Some(0), "STATIC", choose(self.english, "Similarité du nom ≠ probabilité OCR.\r\nAutomatique : uniquement un stade explicite reconnu.", "Name similarity ≠ OCR probability.\r\nAutomatic: only an explicitly recognized stage."), Bounds { x: 16, y: 323, width: 524, height: 39 }, WINDOW_STYLE(0))?;
            for (row, id, french, english) in [
                (0, SCAN_KEY, "&Relire les cartes", "&Scan cards"),
                (1, QUIT_KEY, "&Quitter", "&Quit"),
                (2, SLOT_1_KEY, "Choix &gauche", "&Left choice"),
                (3, SLOT_2_KEY, "Choix &milieu", "&Middle choice"),
                (4, SLOT_3_KEY, "Choix &droite", "&Right choice"),
            ] {
                let y = 64 + row * 48;
                self.create(
                    100 + id,
                    Some(1),
                    "STATIC",
                    choose(self.english, french, english),
                    Bounds {
                        x: 16,
                        y,
                        width: 230,
                        height: 24,
                    },
                    WINDOW_STYLE(0),
                )?;
                self.create(
                    id,
                    Some(1),
                    "EDIT",
                    "",
                    Bounds {
                        x: 268,
                        y,
                        width: 272,
                        height: 26,
                    },
                    WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                )?;
            }
            self.create(161, Some(1), "STATIC", choose(self.english, "Exemples : Ctrl+Shift+M, Alt+F8.\r\nCtrl, Alt ou Win + A–Z, 0–9 ou F1–F24.\r\nUn conflit avec un autre logiciel est signalé dans le menu.", "Examples: Ctrl+Shift+M, Alt+F8.\r\nCtrl, Alt or Win + A–Z, 0–9 or F1–F24.\r\nConflicts with another app are reported in the tray menu."), Bounds { x: 16, y: 310, width: 524, height: 56 }, WINDOW_STYLE(0))?;
            self.create(
                UPDATE_TEXT,
                Some(2),
                "STATIC",
                "",
                Bounds {
                    x: 16,
                    y: 65,
                    width: 524,
                    height: 220,
                },
                WINDOW_STYLE(0),
            )?;
            self.create(
                UPDATE_CHECK,
                Some(2),
                "BUTTON",
                choose(self.english, "&Vérifier", "&Check"),
                Bounds {
                    x: 16,
                    y: 296,
                    width: 252,
                    height: 30,
                },
                WS_TABSTOP,
            )?;
            self.create(
                UPDATE_PREPARE,
                Some(2),
                "BUTTON",
                choose(self.english, "&Préparer la mise à jour", "&Prepare update"),
                Bounds {
                    x: 288,
                    y: 296,
                    width: 252,
                    height: 30,
                },
                WS_TABSTOP,
            )?;
            self.create(
                ERROR_TEXT,
                None,
                "STATIC",
                "",
                Bounds {
                    x: 16,
                    y: 370,
                    width: 524,
                    height: 32,
                },
                WINDOW_STYLE(0),
            )?;
            for (id, x, french, english) in [
                (DEFAULTS, 16, "Valeurs par &défaut", "&Defaults"),
                (CANCEL, 288, "&Annuler", "&Cancel"),
                (SAVE, 414, "&Enregistrer", "&Save"),
            ] {
                self.create(
                    id,
                    None,
                    "BUTTON",
                    choose(self.english, french, english),
                    Bounds {
                        x,
                        y: 405,
                        width: if id == DEFAULTS { 240 } else { 126 },
                        height: 29,
                    },
                    WS_TABSTOP
                        | WINDOW_STYLE(if id == SAVE {
                            BS_DEFPUSHBUTTON as u32
                        } else {
                            0
                        }),
                )?;
            }
            for text in ["Français", "English"] {
                self.combo_add(LANGUAGE, text)?;
            }
            for text in if self.english {
                ["Unknown", "1", "2", "3", "4"]
            } else {
                ["Inconnu", "1", "2", "3", "4"]
            } {
                self.combo_add(STAGE, text)?;
            }
            Ok(())
        }

        fn layout(&mut self) -> Result<()> {
            // The monitor chosen by Windows can differ from the system DPI
            // used before HWND creation. Size both client area and controls
            // from the actual window DPI, also after WM_DPICHANGED.
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: scaled(CLIENT_WIDTH, self.dpi),
                bottom: scaled(CLIENT_HEIGHT, self.dpi),
            };
            unsafe {
                AdjustWindowRectExForDpi(
                    &mut frame,
                    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                    false,
                    WS_EX_CONTROLPARENT,
                    self.dpi,
                )?;
                SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    frame.right - frame.left,
                    frame.bottom - frame.top,
                    SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
            }
            let new_font = unsafe {
                CreateFontW(
                    -scaled(14, self.dpi),
                    0,
                    0,
                    0,
                    400,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    DEFAULT_QUALITY,
                    DEFAULT_PITCH.0 as u32,
                    w!("Segoe UI"),
                )
            };
            ensure!(!new_font.0.is_null(), "Police des réglages indisponible");
            let old = std::mem::replace(&mut self.font, new_font);
            for control in &self.controls {
                let bounds = control.bounds;
                unsafe {
                    SendMessageW(
                        control.hwnd,
                        WM_SETFONT,
                        Some(WPARAM(new_font.0 as usize)),
                        Some(LPARAM(1)),
                    );
                    MoveWindow(
                        control.hwnd,
                        scaled(bounds.x, self.dpi),
                        scaled(bounds.y, self.dpi),
                        scaled(bounds.width, self.dpi),
                        scaled(bounds.height, self.dpi),
                        true,
                    )?;
                }
            }
            if !old.0.is_null() {
                let _ = unsafe { DeleteObject(old.into()) };
            }
            Ok(())
        }

        fn show_page(&mut self, page: u8) {
            self.page = page;
            for control in &self.controls {
                if let Some(control_page) = control.page {
                    let _ = unsafe {
                        ShowWindow(
                            control.hwnd,
                            if control_page == page {
                                SW_SHOW
                            } else {
                                SW_HIDE
                            },
                        )
                    };
                }
            }
            let first = match page {
                1 => SCAN_KEY,
                2 => UPDATE_CHECK,
                _ => LANGUAGE,
            };
            if let Ok(hwnd) = self.control(first) {
                let _ = unsafe { SetFocus(Some(hwnd)) };
            }
        }

        fn combo_add(&self, id: u32, text: &str) -> Result<()> {
            let text = HSTRING::from(text);
            let result = unsafe {
                SendMessageW(
                    self.control(id)?,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(text.as_ptr() as isize)),
                )
            };
            ensure!(result.0 >= 0, "Liste de réglages indisponible");
            Ok(())
        }
        fn combo_set(&self, id: u32, index: usize) -> Result<()> {
            unsafe {
                SendMessageW(self.control(id)?, CB_SETCURSEL, Some(WPARAM(index)), None);
            }
            Ok(())
        }
        fn combo_get(&self, id: u32) -> Result<usize> {
            let value = unsafe { SendMessageW(self.control(id)?, CB_GETCURSEL, None, None) }.0;
            ensure!(value >= 0, "Choix de réglage manquant");
            Ok(value as usize)
        }
        fn set_check(&self, id: u32, checked: bool) -> Result<()> {
            unsafe {
                SendMessageW(
                    self.control(id)?,
                    BM_SETCHECK,
                    Some(WPARAM(usize::from(checked))),
                    None,
                );
            }
            Ok(())
        }
        fn checked(&self, id: u32) -> Result<bool> {
            Ok(unsafe { SendMessageW(self.control(id)?, BM_GETCHECK, None, None) }.0 == 1)
        }
        fn set_text(&self, id: u32, value: impl ToString) -> Result<()> {
            unsafe { SetWindowTextW(self.control(id)?, &HSTRING::from(value.to_string())) }?;
            Ok(())
        }
        fn text(&self, id: u32) -> Result<String> {
            let mut text = [0u16; 65];
            let len = unsafe { GetWindowTextW(self.control(id)?, &mut text) };
            Ok(String::from_utf16_lossy(&text[..len.max(0) as usize])
                .trim()
                .to_owned())
        }
        fn number<T: std::str::FromStr>(&self, id: u32, label: &str) -> Result<T> {
            self.text(id)?.replace(',', ".").parse().map_err(|_| {
                anyhow::anyhow!(
                    "{} : {label}",
                    if self.english {
                        "Invalid number"
                    } else {
                        "Nombre invalide"
                    }
                )
            })
        }
        fn enable_manual_stage(&self) -> Result<()> {
            unsafe {
                let _ = EnableWindow(self.control(STAGE)?, !self.checked(AUTO_STAGE)?);
            }
            Ok(())
        }

        fn populate(&self, config: &Config) -> Result<()> {
            self.combo_set(LANGUAGE, usize::from(config.language == "en"))?;
            self.combo_set(STAGE, config.offer_stage.unwrap_or(0) as usize)?;
            self.set_check(BUILDS, config.show_builds)?;
            self.set_check(
                AUTO_STAGE,
                config.auto_stage && config.offer_stage.is_none(),
            )?;
            self.set_text(INTERVAL, config.scan_interval_ms)?;
            self.set_text(CONFIDENCE, config.minimum_match_confidence)?;
            self.set_text(SCALE, config.overlay_scale)?;
            self.set_text(OPACITY, config.overlay_opacity)?;
            self.set_text(OFFSET_X, config.overlay_offset_x)?;
            self.set_text(OFFSET_Y, config.overlay_offset_y)?;
            for (id, value) in [SCAN_KEY, QUIT_KEY, SLOT_1_KEY, SLOT_2_KEY, SLOT_3_KEY]
                .into_iter()
                .zip(config.shortcuts.values())
            {
                self.set_text(id, value)?;
            }
            self.enable_manual_stage()
        }

        fn read_preferences(&self) -> Result<Config> {
            let auto_stage = self.checked(AUTO_STAGE)?;
            let stage = self.combo_get(STAGE)?;
            let preferences = Config {
                language: if self.combo_get(LANGUAGE)? == 1 {
                    "en"
                } else {
                    "fr"
                }
                .into(),
                scan_interval_ms: self.number(INTERVAL, "400–5000 ms")?,
                minimum_match_confidence: self.number(CONFIDENCE, "0.90–1.00")?,
                show_builds: self.checked(BUILDS)?,
                auto_stage,
                offer_stage: if auto_stage || stage == 0 {
                    None
                } else {
                    Some(stage as u8)
                },
                overlay_scale: self.number(SCALE, "0.75–1.50")?,
                overlay_opacity: self.number(OPACITY, "30–100 %")?,
                overlay_offset_x: self.number(OFFSET_X, "-1000–1000 px")?,
                overlay_offset_y: self.number(OFFSET_Y, "-1000–1000 px")?,
                shortcuts: ShortcutConfig {
                    scan: self.text(SCAN_KEY)?,
                    quit: self.text(QUIT_KEY)?,
                    slot_1: self.text(SLOT_1_KEY)?,
                    slot_2: self.text(SLOT_2_KEY)?,
                    slot_3: self.text(SLOT_3_KEY)?,
                },
                ..Config::default()
            };
            if self.english {
                ensure!(
                    preferences.minimum_match_confidence.is_finite()
                        && (0.90..=1.0).contains(&preferences.minimum_match_confidence),
                    "Name similarity must be between 0.90 and 1.00."
                );
                ensure!(
                    preferences.overlay_scale.is_finite()
                        && (0.75..=1.5).contains(&preferences.overlay_scale),
                    "Overlay scale must be between 0.75 and 1.50."
                );
                ensure!(
                    (400..=5000).contains(&preferences.scan_interval_ms),
                    "Scan interval must be between 400 and 5000 ms."
                );
                ensure!(
                    (30..=100).contains(&preferences.overlay_opacity),
                    "Overlay opacity must be between 30 and 100%."
                );
                ensure!(
                    (-1000..=1000).contains(&preferences.overlay_offset_x)
                        && (-1000..=1000).contains(&preferences.overlay_offset_y),
                    "Overlay offsets must be between -1000 and 1000 pixels."
                );
                let bindings = preferences
                    .shortcuts
                    .values()
                    .into_iter()
                    .map(crate::config::parse_shortcut)
                    .collect::<Result<Vec<_>>>()
                    .map_err(|_| {
                        anyhow::anyhow!("Invalid shortcut: Ctrl/Alt/Win + A–Z, 0–9 or F1–F24.")
                    })?;
                ensure!(
                    !bindings
                        .iter()
                        .enumerate()
                        .any(|(index, binding)| bindings[..index].contains(binding)),
                    "Each action needs a different shortcut."
                );
            }
            preferences.validate()?;
            Ok(preferences)
        }

        fn close(&mut self) {
            if self.is_open() {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), 1);
                    let _ = DestroyWindow(self.hwnd);
                }
                self.hwnd = HWND::default();
            }
        }
    }

    impl Drop for SettingsWindow {
        fn drop(&mut self) {
            self.close();
            if !self.font.0.is_null() {
                let _ = unsafe { DeleteObject(self.font.into()) };
            }
        }
    }

    fn scaled(value: i32, dpi: u32) -> i32 {
        ((value as i64 * dpi as i64 + 48) / 96) as i32
    }
    fn choose<'a>(english: bool, french: &'a str, english_text: &'a str) -> &'a str {
        if english { english_text } else { french }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn preferences_merge_preserves_a_pick_and_rules_saved_after_opening() {
        let directory = std::env::temp_dir().join(format!(
            "MayhemLens-settings-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("config.json");
        let mut preferences = Config {
            language: "en".into(),
            ..Config::default()
        };
        preferences.selected_augments = vec![11];
        Config::default().save(&path).unwrap();
        config::modify(&path, |latest| {
            latest.selected_augments = vec![22, 33];
            latest.synergy_rules.push(crate::domain::SynergyRule {
                id: "synthetic".into(),
                requires: vec![22],
                offered: 44,
                explanation_fr: "Exemple".into(),
                explanation_en: "Example".into(),
            });
            Ok(())
        })
        .unwrap();
        let saved = save_preferences(&path, &preferences).unwrap();
        assert_eq!(saved.language, "en");
        assert_eq!(saved.selected_augments, [22, 33]);
        assert_eq!(saved.synergy_rules.len(), 1);
        preferences.overlay_scale = 10.0;
        assert!(save_preferences(&path, &preferences).is_err());
        assert_eq!(Config::load(&path).unwrap().overlay_scale, 1.0);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn opacity_only_save_cannot_restore_a_stage_from_the_previous_game() {
        let directory = std::env::temp_dir().join(format!(
            "MayhemLens-stage-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("config.json");
        let mut opened = Config {
            offer_stage: Some(2),
            auto_stage: false,
            ..Config::default()
        };
        opened.save(&path).unwrap();
        config::modify(&path, |latest| {
            latest.offer_stage = None;
            latest.auto_stage = true;
            Ok(())
        })
        .unwrap();
        opened.overlay_opacity = 80;
        let saved = save_preferences_with_stage(&path, &opened, false).unwrap();
        assert_eq!(saved.overlay_opacity, 80);
        assert_eq!(saved.offer_stage, None);
        assert!(saved.auto_stage);
        opened.offer_stage = Some(3);
        let explicit = save_preferences_with_stage(&path, &opened, true).unwrap();
        assert_eq!(explicit.offer_stage, Some(3));
        assert!(!explicit.auto_stage);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
