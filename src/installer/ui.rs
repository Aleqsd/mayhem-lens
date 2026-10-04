//! Native installer UI. Constructing this module creates no window or deployment.
//!
//! The window procedure only paints immutable snapshots and posts commands. The
//! owning loop performs control mutations; deployment runs on an explicit-click
//! worker, and closing during deployment never interrupts Windows.
use super::{DISPLAY_NAME, Phase, VERSION, ViewState, engine};
use anyhow::{Context, Result, ensure};
use std::{
    cell::RefCell,
    fs,
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};
use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{
                BST_CHECKED, BST_UNCHECKED, DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED,
                SetWindowTheme,
            },
            HiDpi::{
                AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
            },
            Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus, VK_ESCAPE, VK_RETURN},
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, PCWSTR, w},
};

const WIDTH: i32 = 820;
const HEIGHT: i32 = 560;
const INSTALL: u32 = 10;
const QUIT: u32 = 11;
const CONSENT: u32 = 12;
const STATUS: u32 = 13;
const DETAIL: u32 = 14;
const COMMAND_EVENT: u32 = WM_APP + 140;
const CLOSE_EVENT: u32 = WM_APP + 141;
const DPI_EVENT: u32 = WM_APP + 142;
const WORKER_EVENT: u32 = WM_APP + 143;
const BACKGROUND: COLORREF = color(15, 21, 34);
const PANEL: COLORREF = color(19, 29, 46);
const GOLD: COLORREF = color(244, 193, 74);
const WHITE: COLORREF = color(241, 244, 249);
const MUTED: COLORREF = color(156, 169, 189);
const LINE: COLORREF = color(43, 55, 76);

thread_local! {
    // No Rust state pointer is stored in the HWND. Each paint clones this value,
    // releases the borrow, then calls Win32, including synchronous callbacks.
    static PAINT: RefCell<Option<PaintState>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct PaintState {
    view: ViewState,
    dpi: u32,
    background_brush: usize,
    primary_enabled: bool,
    primary_label: String,
    consent_checked: bool,
    consent_enabled: bool,
}

enum WorkerMessage {
    Preflight(Result<ViewState>),
    Progress(ViewState),
    Finished(Result<ViewState>),
    Launched(Result<()>),
}

struct InstallerWindow {
    hwnd: HWND,
    install: HWND,
    quit: HWND,
    consent: HWND,
    status: HWND,
    detail: HWND,
    body_font: Font,
    button_font: Font,
    brush: Brush,
    dpi: u32,
    view: ViewState,
    preflight_done: bool,
    installing: bool,
    launching: bool,
    receiver: Receiver<WorkerMessage>,
    sender: Sender<WorkerMessage>,
}

/// Opens the installer only when explicitly invoked by its executable.
pub fn run() -> Result<()> {
    // SAFETY: process DPI mode is selected before creating any UI. A manifest
    // may already have selected it, so an access-denied result is harmless.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None)? }.0);
    // SAFETY: this process-lifetime procedure captures no state. The brush is
    // supplied by WM_ERASEBKGND/painting rather than a class-owned GDI object.
    let atom = unsafe {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            hIcon: LoadIconW(Some(instance), PCWSTR(std::ptr::without_provenance(1)))
                .unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("MayhemLensInstaller"),
            ..Default::default()
        })
    };
    ensure!(atom != 0, "Impossible de créer la fenêtre d’installation");
    let dpi = unsafe { GetDpiForSystem() }.max(96);
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let mut bounds = rectangle(0, 0, WIDTH, HEIGHT, dpi);
    unsafe { AdjustWindowRectExForDpi(&mut bounds, style, false, WINDOW_EX_STYLE(0), dpi) }?;
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("MayhemLensInstaller"),
            &HSTRING::from(format!("{DISPLAY_NAME} — Installation")),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            Some(instance),
            None,
        )
    }?;
    let (sender, receiver) = mpsc::channel();
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let mut window = InstallerWindow {
        hwnd,
        install: HWND::default(),
        quit: HWND::default(),
        consent: HWND::default(),
        status: HWND::default(),
        detail: HWND::default(),
        body_font: Font::new(13, false, dpi)?,
        button_font: Font::new(14, true, dpi)?,
        brush: Brush::new(BACKGROUND)?,
        dpi,
        view: checking_view(),
        preflight_done: false,
        installing: false,
        launching: false,
        receiver,
        sender,
    };
    window.create_controls()?;
    window.refresh()?;
    ensure!(
        unsafe { SetTimer(Some(hwnd), 1, 80, None) } != 0,
        "Timer indisponible"
    );
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    window.start_preflight();
    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        ensure!(result != -1, "La boucle de l’installateur a échoué");
        if result == 0 {
            break;
        }
        if message.hwnd == hwnd {
            match message.message {
                COMMAND_EVENT => {
                    window.command(message.wParam.0 as u32)?;
                    continue;
                }
                CLOSE_EVENT => {
                    window.close_requested()?;
                    continue;
                }
                WORKER_EVENT => {
                    window.receive()?;
                    continue;
                }
                DPI_EVENT => {
                    window.layout()?;
                    window.refresh()?;
                    continue;
                }
                _ => {}
            }
        }
        if message.message == WM_KEYDOWN {
            if message.wParam.0 == VK_ESCAPE.0 as usize {
                window.close_requested()?;
                continue;
            }
            if message.wParam.0 == VK_RETURN.0 as usize {
                let focus = unsafe { GetFocus() };
                window.command(if focus == window.quit { QUIT } else { INSTALL })?;
                continue;
            }
        }
        // Native controls provide Tab, Shift+Tab, mnemonic and Space behaviour.
        if !unsafe { IsDialogMessageW(hwnd, &message) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    Ok(())
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND => {
            if (wp.0 >> 16) as u32 & 0xffff == BN_CLICKED {
                let _ = unsafe {
                    PostMessageW(Some(hwnd), COMMAND_EVENT, WPARAM(wp.0 & 0xffff), LPARAM(0))
                };
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { PostMessageW(Some(hwnd), CLOSE_EVENT, WPARAM(0), LPARAM(0)) };
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = unsafe { PostMessageW(Some(hwnd), WORKER_EVENT, WPARAM(0), LPARAM(0)) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: Windows owns this RECT throughout this message callback.
            let bounds = unsafe { *(lp.0 as *const RECT) };
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    bounds.left,
                    bounds.top,
                    bounds.right - bounds.left,
                    bounds.bottom - bounds.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
            };
            let _ = unsafe { PostMessageW(Some(hwnd), DPI_EVENT, WPARAM(0), LPARAM(0)) };
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            if let Some(state) = snapshot() {
                let dc = HDC(wp.0 as *mut _);
                unsafe {
                    SetTextColor(dc, WHITE);
                    SetBkColor(dc, BACKGROUND);
                }
                LRESULT(state.background_brush as isize)
            } else {
                unsafe { DefWindowProcW(hwnd, message, wp, lp) }
            }
        }
        WM_DRAWITEM => {
            if let Some(state) = snapshot() {
                // SAFETY: WM_DRAWITEM supplies this valid structure until return.
                let item = unsafe { &*(lp.0 as *const DRAWITEMSTRUCT) };
                let is_primary = item.CtlID == INSTALL;
                let label = if is_primary {
                    state.primary_label.as_str()
                } else {
                    "&Quitter"
                };
                let _ = draw_button(
                    item.hDC,
                    item.rcItem,
                    label,
                    is_primary,
                    item.itemState.0 & ODS_DISABLED.0 == 0,
                    item.itemState.0 & ODS_SELECTED.0 != 0,
                    item.itemState.0 & ODS_FOCUS.0 != 0,
                    state.dpi,
                );
                LRESULT(1)
            } else {
                LRESULT(0)
            }
        }
        WM_PAINT => {
            let mut info = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut info) };
            if let Some(state) = snapshot() {
                let _ = paint_client(dc, &state, false);
            }
            unsafe {
                let _ = EndPaint(hwnd, &info);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wp, lp) },
    }
}

impl InstallerWindow {
    fn control(&self, id: u32, label: &str, class: &str, style: WINDOW_STYLE) -> Result<HWND> {
        // SAFETY: the owner HWND and module are live; Windows copies both strings.
        Ok(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                &HSTRING::from(class),
                &HSTRING::from(label),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                1,
                1,
                Some(self.hwnd),
                Some(HMENU(id as usize as *mut _)),
                Some(HINSTANCE(GetModuleHandleW(None)?.0)),
                None,
            )
        }?)
    }

    fn create_controls(&mut self) -> Result<()> {
        self.consent = self.control(CONSENT,
            "J’&autorise l’ajout du certificat Mayhem Lens.\r\nWindows demandera mon accord administrateur.",
            "BUTTON", WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | BS_MULTILINE as u32))?;
        // An empty theme selects classic drawing so WM_CTLCOLORBTN supplies white
        // text on our dark background. It remains a real accessible checkbox.
        unsafe { SetWindowTheme(self.consent, w!(""), w!("")) }?;
        self.quit = self.control(
            QUIT,
            "&Quitter",
            "BUTTON",
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        )?;
        self.install = self.control(
            INSTALL,
            "&Installer",
            "BUTTON",
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        )?;
        self.status = self.control(STATUS, "", "STATIC", WINDOW_STYLE(0))?;
        self.detail = self.control(DETAIL, "", "STATIC", WINDOW_STYLE(0))?;
        unsafe {
            SendMessageW(
                self.consent,
                BM_SETCHECK,
                Some(WPARAM(BST_UNCHECKED.0 as usize)),
                None,
            );
        }
        self.layout()
    }

    fn layout(&mut self) -> Result<()> {
        self.dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
        let body_font = Font::new(13, false, self.dpi)?;
        let button_font = Font::new(14, true, self.dpi)?;
        for (hwnd, x, y, width, height) in [
            (self.status, 418, 284, 352, 28),
            (self.detail, 418, 316, 352, 86),
            (self.consent, 418, 453, 352, 44),
            (self.quit, 418, 509, 126, 40),
            (self.install, 556, 509, 214, 40),
        ] {
            unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    scaled(x, self.dpi),
                    scaled(y, self.dpi),
                    scaled(width, self.dpi),
                    scaled(height, self.dpi),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(
                        if hwnd == self.status || hwnd == self.install || hwnd == self.quit {
                            button_font.0.0 as usize
                        } else {
                            body_font.0.0 as usize
                        },
                    )),
                    Some(LPARAM(1)),
                );
            }
        }
        self.body_font = body_font;
        self.button_font = button_font;
        Ok(())
    }

    fn start_preflight(&self) {
        let sender = self.sender.clone();
        // Read-only worker: no certificate, installation, COM deployment or UI.
        thread::spawn(move || {
            let _ = sender.send(WorkerMessage::Preflight(engine::preflight()));
        });
    }

    fn consent_checked(&self) -> bool {
        unsafe { SendMessageW(self.consent, BM_GETCHECK, None, None) }.0 == BST_CHECKED.0 as isize
    }

    fn primary_enabled(&self) -> bool {
        self.preflight_done
            && !self.installing
            && !self.launching
            && (self.view.certificate_trusted
                || self.consent_checked()
                || matches!(self.view.phase, Phase::Complete | Phase::Deferred))
    }

    fn refresh(&self) -> Result<()> {
        let consent_enabled = self.preflight_done
            && !self.installing
            && !self.launching
            && !self.view.certificate_trusted
            && !matches!(self.view.phase, Phase::Complete | Phase::Deferred);
        let primary_label = match self.view.phase {
            Phase::Complete => "&Lancer Mayhem Lens",
            Phase::Deferred => "&Terminer",
            Phase::Failed => "&Réessayer",
            _ if self.installing => "Installation en cours…",
            _ => "&Installer",
        };
        PAINT.with(|paint| {
            *paint.borrow_mut() = Some(PaintState {
                view: self.view.clone(),
                dpi: self.dpi,
                background_brush: self.brush.0.0 as usize,
                primary_enabled: self.primary_enabled(),
                primary_label: primary_label.into(),
                consent_checked: self.consent_checked(),
                consent_enabled,
            })
        });
        unsafe {
            SetWindowTextW(
                self.status,
                &HSTRING::from(short_text(&self.view.status, 100)),
            )?;
            SetWindowTextW(
                self.detail,
                &HSTRING::from(short_text(&self.view.detail, 420)),
            )?;
            SetWindowTextW(self.install, &HSTRING::from(primary_label))?;
            if self.view.certificate_trusted {
                SetWindowTextW(
                    self.consent,
                    w!(
                        "Certificat Mayhem Lens déjà approuvé.\r\nAucune nouvelle autorisation nécessaire."
                    ),
                )?;
            } else {
                SetWindowTextW(
                    self.consent,
                    w!(
                        "J’&autorise l’ajout du certificat Mayhem Lens.\r\nWindows demandera mon accord administrateur."
                    ),
                )?;
            }
            let _ = EnableWindow(self.consent, consent_enabled);
            let _ = EnableWindow(self.install, self.primary_enabled());
            let _ = EnableWindow(self.quit, !self.installing);
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            let _ = InvalidateRect(Some(self.install), None, false);
            let _ = InvalidateRect(Some(self.quit), None, false);
        }
        Ok(())
    }

    fn command(&mut self, id: u32) -> Result<()> {
        match id {
            QUIT => self.close_requested(),
            CONSENT => self.refresh(),
            INSTALL if self.primary_enabled() => {
                if self.view.phase == Phase::Deferred {
                    return self.close_requested();
                }
                if self.view.phase == Phase::Complete {
                    self.launching = true;
                    self.view.status = "Ouverture de Mayhem Lens…".into();
                    self.refresh()?;
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let _ = sender.send(WorkerMessage::Launched(engine::launch()));
                    });
                    return Ok(());
                }
                // Consent is captured at the click, never restored from disk or
                // inferred from an enabled button. The engine rechecks trust.
                let approve = !self.view.certificate_trusted && self.consent_checked();
                self.installing = true;
                self.view.phase = if approve {
                    Phase::Authorizing
                } else {
                    Phase::Installing
                };
                self.view.progress = 0;
                self.view.status = "Installation de Mayhem Lens…".into();
                self.view.detail = if approve {
                    "Accepte la demande Windows pour approuver le certificat de cet installateur."
                } else {
                    "Windows installe l’application pour ton compte."
                }
                .into();
                self.refresh()?;
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let updates = sender.clone();
                    let result = engine::install(approve, move |view| {
                        let _ = updates.send(WorkerMessage::Progress(view));
                    });
                    let _ = sender.send(WorkerMessage::Finished(result));
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn receive(&mut self) -> Result<()> {
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                WorkerMessage::Preflight(result) => {
                    self.preflight_done = true;
                    self.apply_result(result);
                    if !self.view.certificate_trusted {
                        unsafe {
                            let _ = SetFocus(Some(self.consent));
                        }
                    }
                }
                WorkerMessage::Progress(view) => {
                    self.view = clean_view(view);
                }
                WorkerMessage::Finished(result) => {
                    self.installing = false;
                    self.apply_result(result);
                }
                WorkerMessage::Launched(result) => {
                    self.launching = false;
                    match result {
                        Ok(()) => {
                            self.view.status = "Ouverture demandée.".into();
                            self.view.detail = "Après le démarrage, retrouve l’app près de l’horloge. Fermer ses réglages la laisse active ; « Quitter » dans le menu de l’icône l’arrête.".into();
                        }
                        Err(error) => {
                            self.view.status = "L’application n’a pas pu démarrer.".into();
                            self.view.detail = short_text(&error.to_string(), 420);
                        }
                    }
                }
            }
            self.refresh()?;
        }
        Ok(())
    }

    fn apply_result(&mut self, result: Result<ViewState>) {
        match result {
            Ok(view) => {
                self.view = clean_view(view);
            }
            Err(error) => {
                self.view.phase = Phase::Failed;
                self.view.status = "L’opération n’a pas abouti.".into();
                self.view.detail = short_text(&error.to_string(), 420);
            }
        }
    }

    fn close_requested(&mut self) -> Result<()> {
        if self.installing {
            self.view.detail = "L’installation Windows est en cours. Cette fenêtre pourra être fermée dès sa fin ; aucune opération ne sera interrompue.".into();
            self.refresh()
        } else {
            unsafe { DestroyWindow(self.hwnd) }?;
            self.hwnd = HWND::default();
            Ok(())
        }
    }
}

impl Drop for InstallerWindow {
    fn drop(&mut self) {
        if !self.hwnd.0.is_null() {
            let _ = unsafe { DestroyWindow(self.hwnd) };
        }
        PAINT.with(|paint| *paint.borrow_mut() = None);
    }
}

fn snapshot() -> Option<PaintState> {
    PAINT.with(|paint| paint.borrow().clone())
}

fn checking_view() -> ViewState {
    ViewState {
        phase: Phase::Checking,
        progress: 0,
        status: "Vérification de Windows…".into(),
        detail: "L’installation démarrera uniquement lorsque tu cliqueras sur Installer.".into(),
        certificate_trusted: false,
        ocr_available: false,
        installed_version: None,
    }
}

fn clean_view(mut view: ViewState) -> ViewState {
    view.progress = view.progress.min(100);
    view.status = short_text(&view.status, 100);
    view.detail = short_text(&view.detail, 420);
    view.installed_version = view
        .installed_version
        .map(|version| short_text(&version, 40));
    view
}

fn short_text(value: &str, maximum: usize) -> String {
    let mut text = String::new();
    let mut count = 0;
    for character in value.chars() {
        if character.is_control() && character != '\n' && character != '\r' {
            continue;
        }
        if count == maximum {
            text.push('…');
            break;
        }
        text.push(character);
        count += 1;
    }
    text
}

const fn color(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}

fn scaled(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}
fn rectangle(x: i32, y: i32, width: i32, height: i32, dpi: u32) -> RECT {
    RECT {
        left: scaled(x, dpi),
        top: scaled(y, dpi),
        right: scaled(x + width, dpi),
        bottom: scaled(y + height, dpi),
    }
}

struct Brush(HBRUSH);
impl Brush {
    fn new(color: COLORREF) -> Result<Self> {
        let handle = unsafe { CreateSolidBrush(color) };
        ensure!(!handle.0.is_null(), "Pinceau GDI indisponible");
        Ok(Self(handle))
    }
}
impl Drop for Brush {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

struct Font(HFONT);
impl Font {
    fn new(size: i32, bold: bool, dpi: u32) -> Result<Self> {
        let handle = unsafe {
            CreateFontW(
                -scaled(size, dpi),
                0,
                0,
                0,
                if bold { 600 } else { 400 },
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                DEFAULT_PITCH.0 as u32,
                w!("Segoe UI"),
            )
        };
        ensure!(!handle.0.is_null(), "Police GDI indisponible");
        Ok(Self(handle))
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

struct Selected {
    dc: HDC,
    previous: HGDIOBJ,
}
impl Selected {
    fn new(dc: HDC, object: HGDIOBJ) -> Self {
        Self {
            dc,
            previous: unsafe { SelectObject(dc, object) },
        }
    }
}
impl Drop for Selected {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
        }
    }
}

fn fill(dc: HDC, bounds: RECT, color: COLORREF) -> Result<()> {
    let brush = Brush::new(color)?;
    unsafe {
        let _ = FillRect(dc, &bounds, brush.0);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn text(
    dc: HDC,
    label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    size: i32,
    bold: bool,
    color: COLORREF,
    dpi: u32,
) -> Result<()> {
    let font = Font::new(size, bold, dpi)?;
    let _selected = Selected::new(dc, font.0.into());
    let mut bounds = rectangle(x, y, width, height, dpi);
    let mut label: Vec<u16> = label.encode_utf16().collect();
    unsafe {
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT);
        DrawTextW(
            dc,
            &mut label,
            &mut bounds,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        );
    }
    Ok(())
}

fn line(
    dc: HDC,
    from: (i32, i32),
    to: (i32, i32),
    color: COLORREF,
    width: i32,
    dpi: u32,
) -> Result<()> {
    let pen = unsafe { CreatePen(PS_SOLID, scaled(width, dpi).max(1), color) };
    ensure!(!pen.0.is_null(), "Crayon GDI indisponible");
    {
        let _selected = Selected::new(dc, pen.into());
        unsafe {
            let _ = MoveToEx(dc, scaled(from.0, dpi), scaled(from.1, dpi), None);
            let _ = LineTo(dc, scaled(to.0, dpi), scaled(to.1, dpi));
        }
    }
    let _ = unsafe { DeleteObject(pen.into()) };
    Ok(())
}

fn logo(dc: HDC, x: i32, y: i32, size: i32, dpi: u32) -> Result<()> {
    let ring = unsafe { CreatePen(PS_SOLID, scaled(1, dpi).max(1), LINE) };
    ensure!(!ring.0.is_null(), "Crayon GDI indisponible");
    {
        let _pen = Selected::new(dc, ring.into());
        let _brush = Selected::new(dc, unsafe { GetStockObject(HOLLOW_BRUSH) });
        let bounds = rectangle(x - 9, y - 9, size + 18, size + 18, dpi);
        unsafe {
            let _ = Ellipse(dc, bounds.left, bounds.top, bounds.right, bounds.bottom);
        }
        let bounds = rectangle(x - 19, y - 19, size + 38, size + 38, dpi);
        unsafe {
            let _ = Ellipse(dc, bounds.left, bounds.top, bounds.right, bounds.bottom);
        }
    }
    let _ = unsafe { DeleteObject(ring.into()) };
    let point =
        |horizontal: i32, vertical: i32| (x + size * horizontal / 100, y + size * vertical / 100);
    let points = [
        point(16, 80),
        point(16, 20),
        point(50, 59),
        point(84, 20),
        point(84, 80),
    ];
    for pair in points.windows(2) {
        line(dc, pair[0], pair[1], GOLD, 5, dpi)?;
    }
    Ok(())
}

fn paint_client(dc: HDC, state: &PaintState, preview: bool) -> Result<()> {
    let dpi = state.dpi;
    fill(dc, rectangle(0, 0, WIDTH, HEIGHT, dpi), BACKGROUND)?;
    fill(dc, rectangle(0, 0, 368, HEIGHT, dpi), PANEL)?;
    fill(dc, rectangle(367, 0, 1, HEIGHT, dpi), LINE)?;
    // Sparse geometric rings make the brand visible without any game assets.
    logo(dc, 52, 55, 47, dpi)?;
    text(dc, DISPLAY_NAME, 124, 55, 206, 44, 26, true, WHITE, dpi)?;
    text(
        dc,
        "TON CHAMPION.\nTES MEILLEURS CHOIX.",
        40,
        173,
        300,
        85,
        26,
        true,
        WHITE,
        dpi,
    )?;
    for (number, y, title, detail) in [
        (
            "01",
            283,
            "Des tiers pour ton champion",
            "Augmentations et routes de builds Mayhem.",
        ),
        (
            "02",
            351,
            "Une lecture locale",
            "Les données sont en cache pendant le choix.",
        ),
        (
            "03",
            419,
            "Un overlay discret",
            "Les clics et le focus restent dans le jeu.",
        ),
    ] {
        text(dc, number, 40, y + 1, 34, 25, 13, true, GOLD, dpi)?;
        text(dc, title, 82, y, 260, 26, 15, true, WHITE, dpi)?;
        text(dc, detail, 82, y + 26, 244, 36, 12, false, MUTED, dpi)?;
    }
    text(
        dc,
        "ARAM MAYHEM  /  FR · EN",
        40,
        519,
        280,
        20,
        11,
        true,
        GOLD,
        dpi,
    )?;
    text(dc, "INSTALLATION", 418, 44, 352, 20, 11, true, GOLD, dpi)?;
    text(
        dc,
        "Prêt pour\nle Mayhem.",
        418,
        81,
        352,
        88,
        33,
        true,
        WHITE,
        dpi,
    )?;
    text(
        dc,
        &format!("VERSION {VERSION}"),
        418,
        178,
        352,
        22,
        13,
        true,
        WHITE,
        dpi,
    )?;
    let installed = state
        .view
        .installed_version
        .as_ref()
        .map(|version| format!("Version installée : {version}"))
        .unwrap_or_else(|| "Première installation pour ce compte Windows.".into());
    text(dc, &installed, 418, 202, 352, 21, 12, false, MUTED, dpi)?;
    let checking = state.view.phase == Phase::Checking;
    let ocr = if checking {
        "OCR · vérification en cours"
    } else if state.view.ocr_available {
        "OCR Windows détecté · langues à régler dans l’app"
    } else {
        "OCR non détecté · à configurer dans Windows"
    };
    text(
        dc,
        ocr,
        418,
        230,
        352,
        32,
        12,
        false,
        if !checking && !state.view.ocr_available {
            GOLD
        } else {
            MUTED
        },
        dpi,
    )?;
    fill(dc, rectangle(418, 266, 352, 1, dpi), LINE)?;
    if preview {
        text(
            dc,
            &state.view.status,
            418,
            284,
            352,
            28,
            14,
            true,
            WHITE,
            dpi,
        )?;
        text(
            dc,
            &state.view.detail,
            418,
            316,
            352,
            86,
            13,
            false,
            WHITE,
            dpi,
        )?;
    }
    fill(dc, rectangle(418, 414, 352, 5, dpi), LINE)?;
    let progress = state.view.progress.min(100) as i32;
    if progress > 0 {
        fill(dc, rectangle(418, 414, 352 * progress / 100, 5, dpi), GOLD)?;
    }
    let progress_label = if matches!(state.view.phase, Phase::Ready | Phase::Checking) {
        "L’overlay reste fermé pendant l’installation.".into()
    } else {
        format!("Progression  {progress} %")
    };
    text(
        dc,
        &progress_label,
        418,
        428,
        352,
        18,
        11,
        false,
        MUTED,
        dpi,
    )?;
    if preview {
        preview_consent(dc, state)?;
        draw_button(
            dc,
            rectangle(418, 509, 126, 40, dpi),
            "&Quitter",
            false,
            true,
            false,
            false,
            dpi,
        )?;
        draw_button(
            dc,
            rectangle(556, 509, 214, 40, dpi),
            &state.primary_label,
            true,
            state.primary_enabled,
            false,
            false,
            dpi,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_button(
    dc: HDC,
    bounds: RECT,
    label: &str,
    primary: bool,
    enabled: bool,
    pressed: bool,
    focused: bool,
    dpi: u32,
) -> Result<()> {
    let fill_color = if primary && enabled {
        if pressed { color(218, 166, 52) } else { GOLD }
    } else if primary {
        color(58, 64, 74)
    } else {
        PANEL
    };
    fill(dc, bounds, fill_color)?;
    let pen = unsafe {
        CreatePen(
            PS_SOLID,
            scaled(1, dpi).max(1),
            if primary && enabled { GOLD } else { LINE },
        )
    };
    ensure!(!pen.0.is_null(), "Crayon GDI indisponible");
    {
        let _selected = Selected::new(dc, pen.into());
        let _brush = Selected::new(dc, unsafe { GetStockObject(HOLLOW_BRUSH) });
        unsafe {
            let _ = Rectangle(dc, bounds.left, bounds.top, bounds.right, bounds.bottom);
        }
    }
    let _ = unsafe { DeleteObject(pen.into()) };
    let font = Font::new(14, true, dpi)?;
    let _font = Selected::new(dc, font.0.into());
    let mut label: Vec<u16> = label.encode_utf16().collect();
    let mut text_bounds = bounds;
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(
            dc,
            if primary && enabled {
                BACKGROUND
            } else if enabled {
                WHITE
            } else {
                MUTED
            },
        );
        DrawTextW(
            dc,
            &mut label,
            &mut text_bounds,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        if focused {
            let inset = scaled(4, dpi);
            let focus = RECT {
                left: bounds.left + inset,
                top: bounds.top + inset,
                right: bounds.right - inset,
                bottom: bounds.bottom - inset,
            };
            let _ = DrawFocusRect(dc, &focus);
        }
    }
    Ok(())
}

fn preview_consent(dc: HDC, state: &PaintState) -> Result<()> {
    let dpi = state.dpi;
    let checkbox = rectangle(418, 463, 13, 13, dpi);
    fill(
        dc,
        checkbox,
        if state.consent_enabled { WHITE } else { LINE },
    )?;
    if state.consent_checked {
        line(dc, (420, 469), (423, 472), BACKGROUND, 2, dpi)?;
        line(dc, (423, 472), (429, 465), BACKGROUND, 2, dpi)?;
    }
    let label = if state.view.certificate_trusted {
        "Certificat Mayhem Lens déjà approuvé.\nAucune nouvelle autorisation nécessaire."
    } else {
        "J’autorise l’ajout du certificat Mayhem Lens.\nWindows demandera mon accord administrateur."
    };
    text(dc, label, 438, 454, 332, 44, 13, false, WHITE, dpi)
}

/// Renders the same client painter into a memory-only bitmap. It never obtains a
/// screen DC, creates a HWND, calls preflight or starts any installer operation.
pub fn render_preview(path: &Path) -> Result<()> {
    let dc = unsafe { CreateCompatibleDC(None) };
    ensure!(!dc.0.is_null(), "DC mémoire indisponible");
    struct MemoryDc(HDC);
    impl Drop for MemoryDc {
        fn drop(&mut self) {
            let _ = unsafe { DeleteDC(self.0) };
        }
    }
    let dc = MemoryDc(dc);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: WIDTH,
            biHeight: -HEIGHT,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = std::ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(Some(dc.0), &info, DIB_RGB_COLORS, &mut pixels, None, 0) }?;
    struct Bitmap(HBITMAP);
    impl Drop for Bitmap {
        fn drop(&mut self) {
            let _ = unsafe { DeleteObject(self.0.into()) };
        }
    }
    let bitmap = Bitmap(bitmap);
    ensure!(!pixels.is_null(), "Pixels mémoire indisponibles");
    let _selected = Selected::new(dc.0, bitmap.0.into());
    let state = PaintState {
        view: ViewState { phase: Phase::Ready, progress: 0,
            status: "Prêt à installer".into(),
            detail: "Mayhem Lens sera installé pour ton compte Windows.\nCoche l’autorisation ci-dessous pour approuver son certificat, puis clique sur Installer.".into(),
            certificate_trusted: false, ocr_available: true, installed_version: None },
        dpi: 96, background_brush: 0, primary_enabled: false,
        primary_label: "&Installer".into(), consent_checked: false, consent_enabled: true,
    };
    paint_client(dc.0, &state, true)?;
    // GDI calls may be batched; finish them before reading our own DIB memory.
    unsafe {
        let _ = GdiFlush();
    }
    let pixel_count = (WIDTH * HEIGHT * 4) as usize;
    let mut bytes = Vec::with_capacity(pixel_count + 54);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&((pixel_count + 54) as u32).to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&WIDTH.to_le_bytes());
    bytes.extend_from_slice(&(-HEIGHT).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(pixel_count as u32).to_le_bytes());
    bytes.extend_from_slice(&[0; 16]);
    // SAFETY: this DIB owns WIDTH*HEIGHT*4 valid top-down BGRA bytes. No external
    // screen pixels can enter the buffer, and selected bitmap/DC outlive the read.
    bytes
        .extend_from_slice(unsafe { std::slice::from_raw_parts(pixels as *const u8, pixel_count) });
    fs::write(path, bytes).context("Enregistrement de l’aperçu impossible")
}

#[cfg(test)]
mod tests {
    use super::short_text;

    #[test]
    fn error_text_is_bounded_in_unicode_and_rejects_control_characters() {
        assert_eq!(short_text("éé\0éé\t", 3), "ééé…");
        assert_eq!(short_text("Windows\r\nPrêt", 40), "Windows\r\nPrêt");
    }
}
