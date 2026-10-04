use super::{Badge, CaptureReading, CaptureWork, Observation, Rect, UserAction};
use crate::{
    ocr_cache::{RegionOcrCache, RegionOcrKey},
    visual_gate::{self, VisualGate},
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    cell::RefCell,
    fs, ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Foundation::TimeSpan,
    Globalization::Language,
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
            GraphicsCaptureSession,
        },
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        Imaging::{BitmapPixelFormat, SoftwareBitmap},
        SizeInt32,
    },
    Media::Ocr::OcrEngine,
    Storage::Streams::Buffer,
    Win32::{
        Foundation::{
            COLORREF, CloseHandle, ERROR_ALREADY_EXISTS, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
            GetLastError, HANDLE, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE,
            WPARAM,
        },
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
                },
                D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
                D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
                D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
                D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1DCRenderTarget,
                ID2D1Factory,
            },
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            DirectWrite::{
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_MEASURING_MODE_NATURAL, DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory,
                IDWriteFactory,
            },
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                IDXGIDevice,
            },
            Gdi::{
                AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
                CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
                HBITMAP, HDC, HGDIOBJ, SelectObject,
            },
        },
        Storage::Packaging::Appx::{
            GetCurrentPackageFamilyName, GetCurrentPackageFullName, PACKAGE_FAMILY_NAME_MAX_LENGTH,
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::CreateMutexW,
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
                IBufferByteAccess, RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize,
            },
        },
        UI::{
            HiDpi::{
                DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                SetThreadDpiAwarenessContext,
            },
            Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey},
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                Shell_NotifyIconW, ShellExecuteW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, EnumWindows, GetClassNameW, GetCursorPos,
                GetForegroundWindow, GetWindowRect, HMENU, IDI_INFORMATION, IsIconic,
                IsWindowVisible, LoadIconW, MA_NOACTIVATE, MF_CHECKED, MF_GRAYED, MF_SEPARATOR,
                MF_STRING, MSG, PM_REMOVE, PeekMessageW, PostMessageW, RegisterClassW, SW_HIDE,
                SW_SHOW, SW_SHOWNOACTIVATE, SetForegroundWindow, ShowWindow, TPM_RETURNCMD,
                TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow,
                WM_APP, WM_CONTEXTMENU, WM_HOTKEY, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_NULL,
                WM_QUIT, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
    core::{BOOL, HSTRING, Interface, PWSTR, factory, w},
};

const HOTKEY_SCAN: i32 = 1;
const HOTKEY_QUIT: i32 = 2;
const HOTKEY_SLOT_1: i32 = 3;
const HOTKEY_SLOT_2: i32 = 4;
const HOTKEY_SLOT_3: i32 = 5;
const FRAME_TIMEOUT: Duration = Duration::from_millis(750);
const TRAY_MESSAGE: u32 = WM_APP + 32;
const MENU_SCAN: u32 = 100;
const MENU_STAGE_UNKNOWN: u32 = 110;
const MENU_STAGE_1: u32 = 111;
const MENU_STAGE_4: u32 = 114;
const MENU_SETTINGS: u32 = 120;
const MENU_QUIT: u32 = 121;
const MENU_UPDATE: u32 = 122;
const DISPLAY_STATUS_INTERVAL: Duration = Duration::from_secs(1);

thread_local! {
    static CAPTURE_WORKER: RefCell<Option<CaptureWorker>> = const { RefCell::new(None) };
}

struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        // SAFETY: balanced on this same worker thread by Drop, after its COM objects.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.context("Initialisation WinRT du worker")?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

struct DpiContext(DPI_AWARENESS_CONTEXT);
impl DpiContext {
    fn new() -> Self {
        // SAFETY: changes only the current thread; Drop restores its prior context.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
impl Drop for DpiContext {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

fn package_identity() -> bool {
    let mut length = 0;
    // SAFETY: null buffer requests the required length and never reads package data.
    unsafe { GetCurrentPackageFullName(&mut length, None) == ERROR_INSUFFICIENT_BUFFER }
}

/// Returns the package-owned persistent directory visible to host diagnostics.
/// This uses only the process identity API: no COM, file access or directory creation.
pub fn package_data_directory() -> Option<std::path::PathBuf> {
    let mut length = 0;
    // SAFETY: a null buffer asks Windows for the UTF-16 size including its NUL.
    let required = unsafe { GetCurrentPackageFamilyName(&mut length, None) };
    if required != ERROR_INSUFFICIENT_BUFFER
        || length == 0
        || length > PACKAGE_FAMILY_NAME_MAX_LENGTH + 1
    {
        return None;
    }
    let mut buffer = vec![0_u16; length as usize];
    // SAFETY: the writable buffer contains exactly the number of UTF-16 units
    // supplied to Windows, and it remains alive throughout this synchronous call.
    let result =
        unsafe { GetCurrentPackageFamilyName(&mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
    if result != ERROR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|value| *value == 0)?;
    let family = String::from_utf16(&buffer[..end]).ok()?;
    if family.is_empty() {
        return None;
    }
    Some(
        std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
            .join("Packages")
            .join(family)
            .join("LocalState")
            .join("MayhemLens"),
    )
}

pub fn diagnostics() -> Result<String> {
    let packaged = package_identity();
    if !packaged {
        return Ok("Identité MSIX absente. Windows OCR n'est officiellement pris en charge qu'avec une identité de package. L'overlay n'a pas été lancé.".into());
    }
    let _apartment = Apartment::new()?;
    let languages = OcrEngine::AvailableRecognizerLanguages()?
        .into_iter()
        .map(|language| language.LanguageTag().map(|tag| tag.to_string()))
        .collect::<windows::core::Result<Vec<_>>>()?;
    Ok(format!(
        "Identité MSIX présente. Langues OCR installées : {}. Capture WGC disponible : {}. Aucun overlay lancé.",
        languages.join(", "),
        GraphicsCaptureSession::IsSupported()?
    ))
}

/// Read-only preflight. Does not create a window, device, capture session or OCR
/// engine. The engine is initialized only on its long-lived capture worker.
pub fn ensure_environment_ready() -> Result<()> {
    ensure!(
        package_identity(),
        "Windows OCR exige une identité de package. Installer le MSIX puis lancer l'application enregistrée ; l'EXE nu n'est pas une voie supportée."
    );
    let _apartment = Apartment::new()?;
    ensure!(
        GraphicsCaptureSession::IsSupported()?,
        "Windows Graphics Capture indisponible"
    );
    Ok(())
}

pub fn ensure_ready(language: &str) -> Result<()> {
    ensure_environment_ready()?;
    let _apartment = Apartment::new()?;
    let requested = requested_languages(language)?;
    let mut available = false;
    for tag in requested {
        let lang = Language::CreateLanguage(&HSTRING::from(*tag))?;
        available |= OcrEngine::IsLanguageSupported(&lang)?;
    }
    ensure!(
        available,
        "Installer la fonctionnalité OCR Windows de la langue configurée ({})",
        requested.join(", ")
    );
    Ok(())
}

fn requested_languages(language: &str) -> Result<&'static [&'static str]> {
    match language.to_ascii_lowercase().as_str() {
        "fr" | "fr-fr" => Ok(&["fr-FR"]),
        "en" | "en-us" | "en-gb" => Ok(&["en-US"]),
        "auto" | "fr-en" => Ok(&["fr-FR", "en-US"]),
        _ => bail!("Langue OCR inconnue : choisir fr, en ou auto"),
    }
}

unsafe extern "system" fn inspect_game(hwnd: HWND, state: LPARAM) -> BOOL {
    // SAFETY: EnumWindows calls this synchronously; state points to the live local HWND.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return true.into();
        }
        let mut class = [0_u16; 128];
        let length = GetClassNameW(hwnd, &mut class);
        if length > 0 && String::from_utf16_lossy(&class[..length as usize]) == "RiotWindowClass" {
            *(state.0 as *mut HWND) = hwnd;
            return false.into();
        }
        true.into()
    }
}

fn game_window() -> Option<HWND> {
    let mut result = HWND::default();
    // SAFETY: callback and out pointer live throughout this synchronous enumeration.
    let _ = unsafe {
        EnumWindows(
            Some(inspect_game),
            LPARAM((&mut result as *mut HWND) as isize),
        )
    };
    (!result.0.is_null()).then_some(result)
}

pub fn game_window_visible() -> bool {
    game_window().is_some_and(|hwnd| unsafe { GetForegroundWindow() == hwnd })
}

pub fn reading_is_current(bounds: Rect) -> bool {
    game_window().is_some_and(|hwnd| {
        (unsafe { GetForegroundWindow() == hwnd })
            && window_bounds(hwnd).is_ok_and(|current| current == bounds)
    })
}

fn window_bounds(hwnd: HWND) -> Result<Rect> {
    let _dpi = DpiContext::new();
    let mut rect = RECT::default();
    // SAFETY: OS-owned live HWND, initialized writable RECT; no game mutation.
    unsafe { GetWindowRect(hwnd, &mut rect) }?;
    let result = Rect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    };
    ensure!(
        result.width > 0 && result.height > 0,
        "Fenêtre du jeu de taille nulle"
    );
    Ok(result)
}

pub fn invalidate_observations() {
    CAPTURE_WORKER.with(|cell| {
        if let Some(worker) = cell.borrow_mut().as_mut() {
            worker.invalidate();
        }
    });
}

pub fn calibrate_offers(rects: &[Rect]) {
    CAPTURE_WORKER.with(|cell| {
        if let Some(worker) = cell.borrow_mut().as_mut() {
            worker.calibration.feedback(rects);
            if rects.is_empty() {
                worker.last_signature = None;
                worker.last_observations.clear();
                worker.regions.reset();
            }
        }
    });
}

pub fn reset_calibration() {
    CAPTURE_WORKER.with(|cell| {
        if let Some(worker) = cell.borrow_mut().as_mut() {
            worker.calibration.reset();
            worker.invalidate();
        }
    });
}

pub fn observe_game(language: &str, force: bool, has_offers: bool) -> Result<CaptureReading> {
    let hwnd = game_window().context("Fenêtre LoL visible introuvable")?;
    ensure!(
        unsafe { GetForegroundWindow() == hwnd },
        "LoL n'est pas au premier plan"
    );
    let bounds = window_bounds(hwnd)?;
    CAPTURE_WORKER.with(|cell| {
        let mut state = cell.borrow_mut();
        if state
            .as_ref()
            .is_none_or(|worker| worker.language != language)
        {
            *state = Some(CaptureWorker::new(language)?);
        }
        let worker = state.as_mut().context("Worker capture indisponible")?;
        if force {
            worker.invalidate();
        }
        match worker.observe(hwnd, bounds, has_offers) {
            Ok(reading) => Ok(reading),
            Err(error) => {
                // Discard potentially lost GPU resources; the next call recreates them.
                *state = None;
                Err(error)
            }
        }
    })
}

struct CaptureSource {
    hwnd: HWND,
    size: SizeInt32,
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
}
impl Drop for CaptureSource {
    fn drop(&mut self) {
        let _ = self.pool.Close();
    }
}

struct CaptureWorker {
    language: String,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    runtime_device: IDirect3DDevice,
    engines: Vec<OcrEngine>,
    source: Option<CaptureSource>,
    staging: Option<ID3D11Texture2D>,
    staging_size: (u32, u32),
    last_signature: Option<(u64, Rect, Rect, usize, usize)>,
    last_observations: Vec<Observation>,
    regions: RegionOcrCache,
    last_full_ocr: Option<Instant>,
    force_next: bool,
    last_bounds: Option<Rect>,
    calibration: crate::calibration::Calibration,
    // Must drop last, after the WinRT/COM fields declared above.
    _apartment: Apartment,
}

impl CaptureWorker {
    fn new(language: &str) -> Result<Self> {
        ensure!(
            package_identity(),
            "Windows OCR exige une identité de package. Installer le MSIX puis lancer l'application enregistrée ; l'EXE nu n'est pas une voie supportée."
        );
        let apartment = Apartment::new()?;
        ensure!(
            GraphicsCaptureSession::IsSupported()?,
            "Windows Graphics Capture indisponible"
        );
        let requested = requested_languages(language)?;
        let mut engines = Vec::new();
        for tag in requested {
            let lang = Language::CreateLanguage(&HSTRING::from(*tag))?;
            if OcrEngine::IsLanguageSupported(&lang)? {
                engines.push(OcrEngine::TryCreateFromLanguage(&lang)?);
            }
        }
        ensure!(
            !engines.is_empty(),
            "Aucune langue OCR demandée n'est installée dans Windows ({}).",
            requested.join(", ")
        );
        let mut device = None;
        let mut context = None;
        // SAFETY: OS D3D11 device creation writes owned COM references to valid locals.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device = device.context("D3D11 n'a pas retourné de device")?;
        let context = context.context("D3D11 n'a pas retourné de contexte")?;
        let dxgi: IDXGIDevice = device.cast()?;
        let runtime_device = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        Ok(Self {
            language: language.into(),
            device,
            context,
            runtime_device,
            engines,
            source: None,
            staging: None,
            staging_size: (0, 0),
            last_signature: None,
            last_observations: Vec::new(),
            regions: RegionOcrCache::new(),
            last_full_ocr: None,
            force_next: false,
            last_bounds: None,
            calibration: crate::calibration::Calibration::default(),
            _apartment: apartment,
        })
    }

    fn invalidate(&mut self) {
        self.last_signature = None;
        self.last_observations.clear();
        self.regions.reset();
        self.last_full_ocr = None;
        self.force_next = true;
    }

    fn observe(&mut self, hwnd: HWND, bounds: Rect, has_offers: bool) -> Result<CaptureReading> {
        if self.last_bounds != Some(bounds) {
            self.invalidate();
            self.calibration.reset();
            self.last_bounds = Some(bounds);
        }
        let mut calibration = self.calibration.next(bounds);
        let interop: IGraphicsCaptureItemInterop =
            factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        // SAFETY: this targets only the visible game HWND identified above, never memory.
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let size = item.Size()?;
        if self
            .source
            .as_ref()
            .is_none_or(|source| source.hwnd != hwnd || source.size != size)
        {
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &self.runtime_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                1,
                size,
            )?;
            self.source = Some(CaptureSource {
                hwnd,
                size,
                item,
                pool,
            });
            self.invalidate();
            self.calibration.reset();
            calibration = self.calibration.next(bounds);
        }
        let regions = self.calibration.regions();
        let probe_due = full_probe_due(self.last_full_ocr, Instant::now(), regions.is_some());
        let full = self.force_next
            || probe_due
            || calibration.mode != crate::calibration::CalibrationMode::Learned
            || regions.is_none();
        if full && regions.is_some() {
            calibration.roi = crate::calibration::discovery(bounds);
            calibration.mode = crate::calibration::CalibrationMode::PeriodicDiscovery;
        }
        let source = self.source.as_ref().context("Source WGC absente")?;
        // Drain leftovers before starting a fresh, short-lived session. No capture
        // session is kept active between polls, during Alt-Tab, or after OCR.
        while let Ok(frame) = source.pool.TryGetNextFrame() {
            let _ = frame.Close();
        }
        ensure!(
            unsafe { GetForegroundWindow() == hwnd } && window_bounds(hwnd)? == bounds,
            "Le jeu a changé avant le démarrage de la capture"
        );
        let session = source.pool.CreateCaptureSession(&source.item)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetMinUpdateInterval(TimeSpan {
            Duration: 1_000_000,
        });
        session.StartCapture()?;
        let frame = next_frame(&source.pool);
        let _ = session.Close();
        let frame = frame?;
        let image_result = self.copy_title_band(&frame, bounds, calibration.roi);
        let _ = frame.Close();
        let image = image_result?;
        // Foreground and geometry are rechecked after capture to reject stale work.
        ensure!(
            unsafe { GetForegroundWindow() == hwnd } && window_bounds(hwnd)? == bounds,
            "Le jeu a changé de fenêtre ou de position pendant la capture"
        );
        let gate = visual_gate::inspect(&image.pixels, image.width, image.height);
        let mut work = CaptureWork {
            visual_gate: gate,
            mode: "full",
            ocr_regions: 0,
            cached_regions: 0,
        };
        if !should_recognize(
            gate,
            regions.is_some() || has_offers,
            self.force_next,
            probe_due,
        ) {
            self.last_signature = None;
            self.last_observations.clear();
            self.regions.reset();
            work.mode = "visualOnly";
            return Ok(CaptureReading {
                observations: Vec::new(),
                game_bounds: bounds,
                calibration,
                work,
            });
        }
        let signature = full.then(|| {
            (
                fingerprint(&image.pixels),
                bounds,
                Rect {
                    x: image.left as i32,
                    y: image.top as i32,
                    width: image.source_width as i32,
                    height: image.source_height as i32,
                },
                image.frame_width,
                image.frame_height,
            )
        });
        if full && !self.force_next && !probe_due && self.last_signature == signature {
            ensure!(
                unsafe { GetForegroundWindow() == hwnd } && window_bounds(hwnd)? == bounds,
                "Le jeu a changé pendant la reconnaissance en cache"
            );
            work.mode = "fullCache";
            work.cached_regions = 1;
            return Ok(CaptureReading {
                observations: self.last_observations.clone(),
                game_bounds: bounds,
                calibration,
                work,
            });
        }
        let observations = if full {
            let mut keys = Vec::new();
            if let Some(regions) = regions {
                for (index, region) in regions.into_iter().enumerate() {
                    keys.push((index, image.crop(bounds, region)?.cache_key(region)));
                }
            }
            let result = self.recognize(image, bounds)?;
            // A broad probe can segment text better than a regional OCR. Replace
            // all old regional results with this current, complete-frame reading.
            self.regions.reset();
            for (index, key) in keys {
                let reading = result
                    .iter()
                    .filter(|o| self.calibration.owns_observation(index, o.rect))
                    .cloned()
                    .collect();
                self.regions.put(index, key, reading);
            }
            self.last_signature = signature;
            self.last_observations = result.clone();
            self.last_full_ocr = Some(Instant::now());
            self.force_next = false;
            work.ocr_regions = 1;
            result
        } else {
            work.mode = "regions";
            let mut combined = Vec::new();
            for (index, region) in regions
                .context("Régions OCR absentes")?
                .into_iter()
                .enumerate()
            {
                let cropped = image.crop(bounds, region)?;
                let key = cropped.cache_key(region);
                let read = if let Some(cached) = self.regions.get(index, key) {
                    work.cached_regions += 1;
                    cached.to_vec()
                } else {
                    let mut read = self.recognize(cropped, bounds)?;
                    read.retain(|o| self.calibration.owns_observation(index, o.rect));
                    work.ocr_regions += 1;
                    self.regions.put(index, key, read.clone());
                    read
                };
                for observation in read {
                    if !combined.contains(&observation) {
                        combined.push(observation);
                    }
                }
            }
            // Do not let an old whole-band signature bypass newer slot reads.
            self.last_signature = None;
            self.last_observations.clear();
            combined
        };
        ensure!(
            unsafe { GetForegroundWindow() == hwnd } && window_bounds(hwnd)? == bounds,
            "Le jeu a changé pendant la reconnaissance"
        );
        Ok(CaptureReading {
            observations,
            game_bounds: bounds,
            calibration,
            work,
        })
    }

    fn recognize(&self, mut image: CapturedBand, bounds: Rect) -> Result<Vec<Observation>> {
        // Windows OCR has a finite bitmap size. Reduce only this ROI, retaining
        // its native dimensions for projection back to the game's physical pixels.
        image.fit_ocr(OcrEngine::MaxImageDimension()? as usize)?;
        let buffer = Buffer::Create(image.pixels.len().try_into()?)?;
        buffer.SetLength(image.pixels.len().try_into()?)?;
        let access: IBufferByteAccess = buffer.cast()?;
        // SAFETY: the buffer owns exactly pixels.len() writable bytes; neither
        // allocation overlaps and both remain alive during the copy.
        unsafe {
            ptr::copy_nonoverlapping(image.pixels.as_ptr(), access.Buffer()?, image.pixels.len())
        };
        let bitmap = SoftwareBitmap::Create(
            BitmapPixelFormat::Bgra8,
            image.width.try_into()?,
            image.height.try_into()?,
        )?;
        bitmap.CopyFromBuffer(&buffer)?;
        let mut observations = Vec::new();
        for engine in &self.engines {
            let result = engine.RecognizeAsync(&bitmap)?.join()?;
            for line in result.Lines()? {
                // Windows OCR can put titles from different cards on one line.
                // Split on the wide inter-card gap, preserving intra-title words.
                let mut groups: Vec<Vec<(String, windows::Foundation::Rect)>> = Vec::new();
                for word in line.Words()? {
                    let rect = word.BoundingRect()?;
                    let split =
                        groups
                            .last()
                            .and_then(|group| group.last())
                            .is_none_or(|(_, prior)| {
                                rect.X - (prior.X + prior.Width)
                                    > 48.0_f32.max(rect.Height.max(prior.Height) * 2.5)
                            });
                    if split {
                        groups.push(Vec::new());
                    }
                    if let Some(group) = groups.last_mut() {
                        group.push((word.Text()?.to_string(), rect));
                    }
                }
                for group in groups {
                    let text = group
                        .iter()
                        .map(|(text, _)| text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if text.trim().is_empty() {
                        continue;
                    }
                    let Some(rect) = group.iter().map(|(_, rect)| *rect).reduce(union_rect) else {
                        continue;
                    };
                    let observation = Observation {
                        text,
                        rect: image.project(rect, bounds),
                    };
                    if !observations.contains(&observation) {
                        observations.push(observation);
                    }
                }
            }
        }
        add_wrapped_titles(&mut observations);
        let _ = bitmap.Close();
        Ok(observations)
    }

    fn copy_title_band(
        &mut self,
        frame: &Direct3D11CaptureFrame,
        bounds: Rect,
        roi: Rect,
    ) -> Result<CapturedBand> {
        let size = frame.ContentSize()?;
        ensure!(
            size.Width >= 320 && size.Height >= 240,
            "Frame WGC trop petite"
        );
        // The learned ROI is in physical screen pixels. Project into WGC's frame
        // separately: frame dimensions can differ from window bounds/DPI.
        let frame_roi = crate::calibration::frame_region(bounds, roi, size.Width, size.Height)
            .context("Zone OCR de calibrage invalide")?;
        let left = frame_roi.x as u32;
        let top = frame_roi.y as u32;
        let width = frame_roi.width as u32;
        let height = frame_roi.height as u32;
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        // SAFETY: WGC's surface implements the documented DXGI access interop.
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        ensure!(
            left + width <= desc.Width && top + height <= desc.Height,
            "Zone OCR hors de la texture WGC"
        );
        if self.staging_size != (width, height) {
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            unsafe {
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))
            }?;
            self.staging = staging;
            self.staging_size = (width, height);
        }
        let staging = self
            .staging
            .as_ref()
            .context("Texture de recadrage absente")?;
        let source_box = D3D11_BOX {
            left,
            top,
            front: 0,
            right: left + width,
            bottom: top + height,
            back: 1,
        };
        let mut mapping = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: validated box; both resources belong to this worker/device. Only
        // this small ROI is read back, not the complete screen or game texture.
        unsafe {
            self.context
                .CopySubresourceRegion(staging, 0, 0, 0, 0, &texture, 0, Some(&source_box));
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapping))?;
        }
        let stride = width as usize * 4;
        let mut pixels = vec![0_u8; stride * height as usize];
        // SAFETY: Map supplies at least RowPitch bytes for each of height rows,
        // each row contains Width BGRA pixels, and the destination length matches.
        unsafe {
            for row in 0..height as usize {
                ptr::copy_nonoverlapping(
                    (mapping.pData as *const u8).add(row * mapping.RowPitch as usize),
                    pixels.as_mut_ptr().add(row * stride),
                    stride,
                );
            }
            self.context.Unmap(staging, 0);
        }
        // Capture alpha is not useful to the OCR image; make it explicitly opaque.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        Ok(CapturedBand {
            pixels,
            left,
            top,
            width: width as usize,
            height: height as usize,
            source_width: width as usize,
            source_height: height as usize,
            frame_width: size.Width as usize,
            frame_height: size.Height as usize,
        })
    }
}

fn union_rect(
    a: windows::Foundation::Rect,
    b: windows::Foundation::Rect,
) -> windows::Foundation::Rect {
    let x = a.X.min(b.X);
    let y = a.Y.min(b.Y);
    windows::Foundation::Rect {
        X: x,
        Y: y,
        Width: (a.X + a.Width).max(b.X + b.Width) - x,
        Height: (a.Y + a.Height).max(b.Y + b.Height) - y,
    }
}

fn add_wrapped_titles(observations: &mut Vec<Observation>) {
    let originals = observations.clone();
    for top in &originals {
        for bottom in &originals {
            let a = top.rect;
            let b = bottom.rect;
            let gap = b.y - (a.y + a.height);
            let overlap = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
            if a.height <= 0
                || b.height <= 0
                || gap < -2
                || gap > a.height.max(b.height)
                || overlap <= 0
                || overlap * 2 < a.width.min(b.width)
                || ((a.x + a.width / 2) - (b.x + b.width / 2)).abs() > a.width.max(b.width) / 3
                || top.text.split_whitespace().count() + bottom.text.split_whitespace().count() > 12
            {
                continue;
            }
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            let joined = Observation {
                text: format!("{} {}", top.text, bottom.text),
                rect: Rect {
                    x,
                    y,
                    width: (a.x + a.width).max(b.x + b.width) - x,
                    height: (a.y + a.height).max(b.y + b.height) - y,
                },
            };
            if !observations.contains(&joined) {
                observations.push(joined);
            }
        }
    }
}

fn next_frame(pool: &Direct3D11CaptureFramePool) -> Result<Direct3D11CaptureFrame> {
    let deadline = Instant::now() + FRAME_TIMEOUT;
    loop {
        if let Ok(frame) = pool.TryGetNextFrame() {
            return Ok(frame);
        }
        ensure!(
            Instant::now() < deadline,
            "Aucune frame WGC reçue dans le délai de capture"
        );
        thread::sleep(Duration::from_millis(4));
    }
}

struct CapturedBand {
    pixels: Vec<u8>,
    left: u32,
    top: u32,
    width: usize,
    height: usize,
    source_width: usize,
    source_height: usize,
    frame_width: usize,
    frame_height: usize,
}

impl CapturedBand {
    fn cache_key(&self, region: Rect) -> RegionOcrKey {
        RegionOcrKey {
            region,
            frame_bounds: Rect {
                x: self.left as i32,
                y: self.top as i32,
                width: self.source_width as i32,
                height: self.source_height as i32,
            },
            frame_width: self.frame_width,
            frame_height: self.frame_height,
            pixels_hash: fingerprint(&self.pixels),
        }
    }
    fn crop(&self, bounds: Rect, region: Rect) -> Result<Self> {
        ensure!(
            self.width == self.source_width && self.height == self.source_height,
            "Recadrage après réduction OCR interdit"
        );
        let frame_region = crate::calibration::frame_region(
            bounds,
            region,
            self.frame_width.try_into()?,
            self.frame_height.try_into()?,
        )
        .context("Région OCR invalide")?;
        let left = frame_region.x as usize;
        let top = frame_region.y as usize;
        let width = frame_region.width as usize;
        let height = frame_region.height as usize;
        ensure!(
            left >= self.left as usize
                && top >= self.top as usize
                && left + width <= self.left as usize + self.width
                && top + height <= self.top as usize + self.height,
            "Région OCR hors de la bande capturée"
        );
        let offset_x = left - self.left as usize;
        let offset_y = top - self.top as usize;
        let mut pixels = Vec::with_capacity(width * height * 4);
        for row in offset_y..offset_y + height {
            let start = (row * self.width + offset_x) * 4;
            pixels.extend_from_slice(&self.pixels[start..start + width * 4]);
        }
        Ok(Self {
            pixels,
            left: left as u32,
            top: top as u32,
            width,
            height,
            source_width: width,
            source_height: height,
            frame_width: self.frame_width,
            frame_height: self.frame_height,
        })
    }

    fn project(&self, rect: windows::Foundation::Rect, bounds: Rect) -> Rect {
        let scale_x = bounds.width as f32 / self.frame_width as f32;
        let scale_y = bounds.height as f32 / self.frame_height as f32;
        let ocr_x = self.source_width as f32 / self.width as f32;
        let ocr_y = self.source_height as f32 / self.height as f32;
        Rect {
            x: bounds.x + ((self.left as f32 + rect.X * ocr_x) * scale_x).round() as i32,
            y: bounds.y + ((self.top as f32 + rect.Y * ocr_y) * scale_y).round() as i32,
            width: (rect.Width * ocr_x * scale_x).ceil() as i32,
            height: (rect.Height * ocr_y * scale_y).ceil() as i32,
        }
    }

    fn fit_ocr(&mut self, maximum: usize) -> Result<()> {
        ensure!(maximum > 0, "Dimensions maximales OCR invalides");
        if self.width.max(self.height) <= maximum {
            return Ok(());
        }
        let ratio = maximum as f64 / self.width.max(self.height) as f64;
        let width = ((self.width as f64 * ratio).floor() as usize).max(1);
        let height = ((self.height as f64 * ratio).floor() as usize).max(1);
        let mut pixels = vec![0_u8; width * height * 4];
        // Bilinear sampling preserves title edges better than discarding pixels.
        for y in 0..height {
            let source_y = ((y as f64 + 0.5) * self.height as f64 / height as f64 - 0.5)
                .clamp(0.0, (self.height - 1) as f64);
            let y0 = source_y.floor() as usize;
            let y1 = (y0 + 1).min(self.height - 1);
            let fy = source_y - y0 as f64;
            for x in 0..width {
                let source_x = ((x as f64 + 0.5) * self.width as f64 / width as f64 - 0.5)
                    .clamp(0.0, (self.width - 1) as f64);
                let x0 = source_x.floor() as usize;
                let x1 = (x0 + 1).min(self.width - 1);
                let fx = source_x - x0 as f64;
                for channel in 0..3 {
                    let sample =
                        |sx, sy| f64::from(self.pixels[(sy * self.width + sx) * 4 + channel]);
                    let top = sample(x0, y0) * (1.0 - fx) + sample(x1, y0) * fx;
                    let bottom = sample(x0, y1) * (1.0 - fx) + sample(x1, y1) * fx;
                    pixels[(y * width + x) * 4 + channel] =
                        (top * (1.0 - fy) + bottom * fy).round() as u8;
                }
                pixels[(y * width + x) * 4 + 3] = 255;
            }
        }
        self.pixels = pixels;
        self.width = width;
        self.height = height;
        Ok(())
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    // Exact content signature: rerolls invalidate immediately. Animation can
    // cause extra OCR; it cannot conceal a changed title by a similarity threshold.
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn should_recognize(gate: VisualGate, learned: bool, forced: bool, probe_due: bool) -> bool {
    // Visual absence is only a hint: learned slots always compare fresh pixels,
    // and the periodic full probe catches sampling, color and layout misses.
    learned || forced || probe_due || gate == VisualGate::Candidate
}

fn full_probe_due(last: Option<Instant>, now: Instant, learned: bool) -> bool {
    last.is_none_or(|last| {
        now.saturating_duration_since(last) >= Duration::from_secs(if learned { 5 } else { 3 })
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct HotkeyWarning {
    // Both fields are controlled locally, never formatted OS/source error text.
    shortcut: String,
    error_code: String,
}

struct HotkeyRegistrations {
    registered: Vec<i32>,
    unavailable: Vec<HotkeyWarning>,
}

fn register_hotkeys_with(
    shortcuts: &crate::config::ShortcutConfig,
    mut register: impl FnMut(i32, crate::config::ParsedShortcut) -> std::result::Result<(), i32>,
) -> HotkeyRegistrations {
    let mut registrations = HotkeyRegistrations {
        registered: Vec::new(),
        unavailable: Vec::new(),
    };
    for (id, shortcut) in [
        HOTKEY_SCAN,
        HOTKEY_QUIT,
        HOTKEY_SLOT_1,
        HOTKEY_SLOT_2,
        HOTKEY_SLOT_3,
    ]
    .into_iter()
    .zip(shortcuts.values())
    {
        // Validated configuration normally makes this branch infallible; do not
        // register an invented binding if a caller supplied malformed settings.
        let result = crate::config::parse_shortcut(shortcut)
            .map_err(|_| 0x8007_0057_u32 as i32)
            .and_then(|binding| register(id, binding));
        match result {
            Ok(()) => registrations.registered.push(id),
            Err(code) => registrations.unavailable.push(HotkeyWarning {
                shortcut: shortcut.to_owned(),
                error_code: format!("0x{:08X}", code as u32),
            }),
        }
    }
    registrations
}

struct Hotkeys(HotkeyRegistrations);
impl Hotkeys {
    fn new(shortcuts: &crate::config::ShortcutConfig) -> Self {
        Self(register_hotkeys_with(shortcuts, |id, binding| {
            // SAFETY: thread-owned hotkey registration, no keyboard input injection.
            unsafe {
                RegisterHotKey(
                    None,
                    id,
                    windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS(
                        binding.modifiers,
                    ) | MOD_NOREPEAT,
                    binding.virtual_key,
                )
            }
            .map_err(|error| error.code().0)
        }))
    }

    fn available(&self, id: i32) -> bool {
        self.0.registered.contains(&id)
    }

    fn reload(&mut self, shortcuts: &crate::config::ShortcutConfig) {
        // Unregister before re-registering unchanged IDs on the same UI thread.
        for id in self.0.registered.drain(..) {
            let _ = unsafe { UnregisterHotKey(None, id) };
        }
        *self = Self::new(shortcuts);
    }
}
impl Drop for Hotkeys {
    fn drop(&mut self) {
        for id in &self.0.registered {
            let _ = unsafe { UnregisterHotKey(None, *id) };
        }
    }
}

unsafe extern "system" fn badge_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    // SAFETY: standard forwarding of the OS-provided message and live HWND.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

struct BadgeWindow(HWND);
impl Drop for BadgeWindow {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

/// Held by runtime coordination before it starts any worker or window loop.
pub struct InstanceGuard(HANDLE);
impl InstanceGuard {
    fn new() -> Result<Self> {
        // SAFETY: named process-lifetime handle, no initially owned lock to release.
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\MayhemLens")) }?;
        let instance = Self(handle);
        ensure!(
            unsafe { GetLastError() } != ERROR_ALREADY_EXISTS,
            "Mayhem Lens est déjà lancé dans cette session Windows"
        );
        Ok(instance)
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub fn acquire_single_instance() -> Result<InstanceGuard> {
    InstanceGuard::new()
}

struct PopupMenu(HMENU);
impl Drop for PopupMenu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

struct Tray {
    data: NOTIFYICONDATAW,
    // Drop removes the notification before this owned HWND is destroyed.
    owner: BadgeWindow,
}
impl Tray {
    fn new(instance: HINSTANCE) -> Result<Self> {
        // This hidden owner exists only in the explicitly launched runtime.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("MayhemLensPassiveBadge"),
                w!("Mayhem Lens"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )
        }?;
        let owner = BadgeWindow(hwnd);
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_MESSAGE,
            hIcon: unsafe { LoadIconW(None, IDI_INFORMATION) }?,
            ..Default::default()
        };
        for (index, character) in "Mayhem Lens — tiers ARAM Mayhem".encode_utf16().enumerate() {
            if index >= data.szTip.len() - 1 {
                break;
            }
            data.szTip[index] = character;
        }
        ensure!(
            unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool(),
            "Impossible de créer l'icône Mayhem Lens dans la zone de notification"
        );
        Ok(Self { data, owner })
    }

    fn accepts(&self, message: &MSG) -> bool {
        message.hwnd == self.owner.0
            && message.message == TRAY_MESSAGE
            && matches!(
                message.lParam.0 as u32,
                WM_LBUTTONUP | WM_RBUTTONUP | WM_CONTEXTMENU
            )
    }

    fn show_menu(
        &self,
        actions: &Sender<UserAction>,
        stop: &AtomicBool,
        config_path: &std::path::Path,
        updates: &crate::update::UpdateController,
        hotkeys: &Hotkeys,
        settings: &mut Option<crate::settings::SettingsWindow>,
    ) -> Result<()> {
        let menu = PopupMenu(unsafe { CreatePopupMenu() }?);
        let config = crate::config::Config::load(config_path)?;
        let english = config.language == "en";
        let update_status = updates.status();
        let update_ready = matches!(
            update_status.phase,
            crate::update::UpdatePhase::ReadyOnRestart | crate::update::UpdatePhase::Registered
        );
        let scan_label = if english {
            "Scan when back in game"
        } else {
            "Scanner au retour au jeu"
        };
        let scan = HSTRING::from(if hotkeys.available(HOTKEY_SCAN) {
            format!("{scan_label}  ({})", config.shortcuts.scan)
        } else {
            scan_label.into()
        });
        // A menu opens only after an explicit click on the tray icon. Passive
        // badge updates never call SetForegroundWindow or generate game input.
        unsafe {
            AppendMenuW(menu.0, MF_STRING, MENU_SCAN as usize, &scan)?;
            AppendMenuW(menu.0, MF_SEPARATOR, 0, None)?;
        }
        for warning in &hotkeys.0.unavailable {
            let label = if english {
                format!("Shortcut unavailable: {}", warning.shortcut)
            } else {
                format!("Raccourci indisponible : {}", warning.shortcut)
            };
            unsafe { AppendMenuW(menu.0, MF_STRING | MF_GRAYED, 0, &HSTRING::from(label)) }?;
        }
        if !hotkeys.0.unavailable.is_empty() {
            unsafe { AppendMenuW(menu.0, MF_SEPARATOR, 0, None) }?;
        }
        let stage = config.offer_stage;
        // Automatic is an explicit mode; Unknown disables inference entirely.
        const MENU_STAGE_AUTO: u32 = 209;
        unsafe {
            AppendMenuW(
                menu.0,
                if config.auto_stage && stage.is_none() {
                    MF_STRING | MF_CHECKED
                } else {
                    MF_STRING
                },
                MENU_STAGE_AUTO as usize,
                &HSTRING::from(if english {
                    "Automatic stage"
                } else {
                    "Stade automatique"
                }),
            )
        }?;
        let unknown_flags = if stage.is_none() && !config.auto_stage {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        unsafe {
            AppendMenuW(
                menu.0,
                unknown_flags,
                MENU_STAGE_UNKNOWN as usize,
                &HSTRING::from(if english {
                    "Unknown stage"
                } else {
                    "Stade inconnu"
                }),
            )
        }?;
        for value in 1..=4 {
            let label = HSTRING::from(if english {
                format!("Stage {value}")
            } else {
                format!("Stade {value}")
            });
            let flags = if stage == Some(value as u8) {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            unsafe { AppendMenuW(menu.0, flags, (MENU_STAGE_1 + value - 1) as usize, &label) }?;
        }
        unsafe {
            AppendMenuW(menu.0, MF_SEPARATOR, 0, None)?;
            AppendMenuW(
                menu.0,
                MF_STRING | MF_GRAYED,
                0,
                &HSTRING::from(update_status.summary(&config.language)),
            )?;
            AppendMenuW(
                menu.0,
                if update_status.busy() {
                    MF_STRING | MF_GRAYED
                } else {
                    MF_STRING
                },
                MENU_UPDATE as usize,
                &HSTRING::from(if english {
                    "Check / prepare update"
                } else {
                    "Rechercher / préparer une mise à jour"
                }),
            )?;
            AppendMenuW(menu.0, MF_SEPARATOR, 0, None)?;
            AppendMenuW(
                menu.0,
                MF_STRING,
                MENU_SETTINGS as usize,
                &HSTRING::from(if english {
                    "Settings…"
                } else {
                    "Réglages…"
                }),
            )?;
            AppendMenuW(
                menu.0,
                MF_STRING,
                MENU_QUIT as usize,
                &HSTRING::from({
                    let label = match (english, update_ready) {
                        (true, true) => "Quit to apply update",
                        (false, true) => "Quitter pour appliquer la mise à jour",
                        (true, false) => "Quit",
                        (false, false) => "Quitter",
                    };
                    if hotkeys.available(HOTKEY_QUIT) {
                        format!("{label}  ({})", config.shortcuts.quit)
                    } else {
                        label.into()
                    }
                }),
            )?;
        }
        let mut point = POINT::default();
        let command = unsafe {
            GetCursorPos(&mut point)?;
            let _ = SetForegroundWindow(self.owner.0);
            let result = TrackPopupMenu(
                menu.0,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                self.owner.0,
                None,
            );
            let _ = PostMessageW(Some(self.owner.0), WM_NULL, WPARAM(0), LPARAM(0));
            result.0 as u32
        };
        match command {
            MENU_SCAN => {
                let _ = actions.send(UserAction::ForceScan);
            }
            MENU_STAGE_UNKNOWN => {
                let _ = actions.send(UserAction::SetStage(None));
            }
            MENU_STAGE_AUTO => {
                let _ = actions.send(UserAction::SetAutoStage(true));
            }
            MENU_STAGE_1..=MENU_STAGE_4 => {
                let _ = actions.send(UserAction::SetStage(Some(
                    (command - MENU_STAGE_1 + 1) as u8,
                )));
            }
            MENU_SETTINGS => {
                if let Some(window) = settings.as_ref().filter(|window| window.is_open()) {
                    window.show();
                } else {
                    *settings = Some(crate::settings::SettingsWindow::open(
                        self.owner.0,
                        config_path,
                        updates.clone(),
                    )?);
                }
            }
            MENU_UPDATE => {
                if update_status.phase == crate::update::UpdatePhase::Unsupported {
                    // Unsupported Windows builds open the release instructions.
                    // This happens only after the explicit menu action.
                    let url = HSTRING::from(crate::update::RELEASES_URL);
                    let result = unsafe {
                        ShellExecuteW(Some(self.owner.0), w!("open"), &url, None, None, SW_SHOW)
                    };
                    ensure!(
                        result.0 as isize > 32,
                        "Impossible d'ouvrir le canal de mise à jour"
                    );
                } else {
                    let _ = updates.request_update();
                }
            }
            MENU_QUIT => stop.store(true, Ordering::Release),
            _ => {}
        }
        Ok(())
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data) };
    }
}

struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Dib {
    fn new(width: i32, height: i32) -> Result<Self> {
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = unsafe { CreateCompatibleDC(None) };
        ensure!(!dc.0.is_null(), "Impossible de créer le DC de badge");
        let mut bits = ptr::null_mut();
        let bitmap = match unsafe {
            CreateDIBSection(Some(dc), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = unsafe { DeleteDC(dc) };
                return Err(error.into());
            }
        };
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        Ok(Self {
            dc,
            bitmap,
            previous,
        })
    }
}
impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: restore selection before deleting these exclusively owned GDI objects.
        unsafe {
            let _ = SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

struct BadgeRenderer {
    target: ID2D1DCRenderTarget,
    dwrite: IDWriteFactory,
}
impl BadgeRenderer {
    fn new() -> Result<Self> {
        // SAFETY: thread-confined D2D/DirectWrite COM factories and render target.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = d2d.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            })?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Self { target, dwrite })
        }
    }

    fn paint(&self, window: HWND, badge: &Badge, bounds: Rect, opacity: u8) -> Result<()> {
        let dib = Dib::new(bounds.width, bounds.height)?;
        // Geometry has already been scaled by coordination. Only text and
        // padding use that same scale here; never scale the HWND a second time.
        let scale = badge.scale.clamp(0.4, 2.55);
        // SAFETY: valid memory DC selected with a BGRA bitmap, valid bounds and
        // thread-owned COM renderer. D2D supplies premultiplied per-pixel alpha.
        unsafe {
            let title = self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                17.0 * scale,
                w!("fr-FR"),
            )?;
            let detail = self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0 * scale,
                w!("fr-FR"),
            )?;
            detail.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            self.target.BindDC(
                dib.dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: bounds.width,
                    bottom: bounds.height,
                },
            )?;
            let background = self.target.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.04,
                    g: 0.05,
                    b: 0.075,
                    a: 1.0,
                },
                None,
            )?;
            let heading = self.target.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 0.82,
                    b: 0.35,
                    a: 1.0,
                },
                None,
            )?;
            let body = self.target.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.88,
                    g: 0.91,
                    b: 0.95,
                    a: 1.0,
                },
                None,
            )?;
            self.target.BeginDraw();
            self.target.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            self.target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 0.0,
                        top: 0.0,
                        right: bounds.width as f32,
                        bottom: bounds.height as f32,
                    },
                    radiusX: 7.0 * scale,
                    radiusY: 7.0 * scale,
                },
                &background,
            );
            self.target.DrawText(
                &badge.title.encode_utf16().collect::<Vec<_>>(),
                &title,
                &D2D_RECT_F {
                    left: 10.0 * scale,
                    top: 5.0 * scale,
                    right: bounds.width as f32 - 10.0 * scale,
                    bottom: 29.0 * scale,
                },
                &heading,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.target.DrawText(
                &badge.detail.encode_utf16().collect::<Vec<_>>(),
                &detail,
                &D2D_RECT_F {
                    left: 10.0 * scale,
                    top: 30.0 * scale,
                    right: bounds.width as f32 - 10.0 * scale,
                    bottom: bounds.height as f32 - 4.0 * scale,
                },
                &body,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.target.EndDraw(None, None)?;
            UpdateLayeredWindow(
                window,
                None,
                Some(&POINT {
                    x: bounds.x,
                    y: bounds.y,
                }),
                Some(&SIZE {
                    cx: bounds.width,
                    cy: bounds.height,
                }),
                Some(dib.dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: ((u16::from(opacity) * 255 + 50) / 100) as u8,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                }),
                ULW_ALPHA,
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum DisplayReason {
    Visible,
    NoBadges,
    GameUnavailable,
    HiddenForeground,
    GeometryUnavailable,
    GeometryChanged,
    TtlExpired,
    Stopped,
}

#[derive(Clone, Copy)]
struct DisplayContext {
    stopped: bool,
    game_visible: bool,
    game_foreground: bool,
    game_geometry: Option<Rect>,
    published_geometry: Option<Rect>,
    badge_count: usize,
    fresh: bool,
}
impl DisplayContext {
    fn reason(&self) -> DisplayReason {
        if self.stopped {
            DisplayReason::Stopped
        } else if !self.game_visible {
            DisplayReason::GameUnavailable
        } else if !self.game_foreground {
            DisplayReason::HiddenForeground
        } else if self.game_geometry.is_none() {
            DisplayReason::GeometryUnavailable
        } else if self.badge_count == 0 {
            DisplayReason::NoBadges
        } else if !self.fresh {
            DisplayReason::TtlExpired
        } else if self.game_geometry != self.published_geometry {
            DisplayReason::GeometryChanged
        } else {
            DisplayReason::Visible
        }
    }
}

#[derive(Serialize)]
struct DisplayRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}
impl From<Rect> for DisplayRect {
    fn from(rect: Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DisplayStatus {
    timestamp_unix_ms: u64,
    process_id: u32,
    game_visible: bool,
    game_foreground: bool,
    window_count: usize,
    badges_requested_count: usize,
    visible_window_count: usize,
    visible_windows: Vec<DisplayRect>,
    game_bounds: Option<DisplayRect>,
    published_game_bounds: Option<DisplayRect>,
    display_reason: DisplayReason,
    last_badges_age_ms: Option<u64>,
    freshness_ttl_ms: u64,
    hotkey_warnings: Vec<HotkeyWarning>,
}

/// Observations concern our owned HWNDs, not players, captures or recognized text.
/// The disk stays off the render thread, with one bounded pending snapshot.
struct DisplayPublisher {
    sender: SyncSender<DisplayStatus>,
    last_snapshot: Option<Instant>,
}
impl DisplayPublisher {
    fn new() -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<DisplayStatus>(1);
        thread::Builder::new()
            .name("mayhem-display-status".into())
            .spawn(move || {
                let mut last_write: Option<Instant> = None;
                let mut failed = false;
                while let Ok(status) = receiver.recv() {
                    if let Some(previous) = last_write {
                        thread::sleep(DISPLAY_STATUS_INTERVAL.saturating_sub(previous.elapsed()));
                    }
                    let result = persist_display_status(&status);
                    last_write = Some(Instant::now());
                    match result {
                        Ok(()) => failed = false,
                        Err(error) if !failed => {
                            eprintln!("État affichage : {error:#}");
                            failed = true;
                        }
                        Err(_) => {}
                    }
                }
            })
            .context("Création du worker d'état affichage")?;
        Ok(Self {
            sender,
            last_snapshot: None,
        })
    }

    fn due(&self) -> bool {
        self.last_snapshot
            .is_none_or(|previous| previous.elapsed() >= DISPLAY_STATUS_INTERVAL)
    }

    fn publish(&mut self, status: DisplayStatus) {
        self.last_snapshot = Some(Instant::now());
        // A slow disk can skip a snapshot; it must never stall the overlay.
        let _ = self.sender.try_send(status);
    }
}

fn persist_display_status(status: &DisplayStatus) -> Result<()> {
    let directory = crate::config::app_directory();
    fs::create_dir_all(&directory)?;
    let path = directory.join("display-status.json");
    let temporary = directory.join("display-status.json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(status)?)?;
    fs::rename(temporary, path).context("Enregistrement atomique de l'état affichage")
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn display_status(
    windows: &[BadgeWindow],
    context: &DisplayContext,
    last_received: Option<Instant>,
    scan_interval_ms: u64,
    hotkeys: &Hotkeys,
) -> DisplayStatus {
    // SAFETY: every queried HWND is owned by this display thread and remains
    // alive throughout the snapshot. This checks the native WS_VISIBLE state;
    // it is not a screenshot or proof that other windows cannot occlude pixels.
    let visible: Vec<_> = windows
        .iter()
        .filter(|window| unsafe { IsWindowVisible(window.0) }.as_bool())
        .collect();
    let visible_windows = visible
        .iter()
        .filter_map(|window| window_bounds(window.0).ok().map(DisplayRect::from))
        .collect();
    DisplayStatus {
        timestamp_unix_ms: duration_ms(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
        ),
        process_id: std::process::id(),
        game_visible: context.game_visible,
        game_foreground: context.game_foreground,
        window_count: windows.len(),
        badges_requested_count: context.badge_count,
        visible_window_count: visible.len(),
        visible_windows,
        game_bounds: context.game_geometry.map(DisplayRect::from),
        published_game_bounds: context.published_geometry.map(DisplayRect::from),
        display_reason: context.reason(),
        last_badges_age_ms: last_received.map(|received| duration_ms(received.elapsed())),
        freshness_ttl_ms: scan_interval_ms.saturating_add(1_500),
        hotkey_warnings: hotkeys.0.unavailable.clone(),
    }
}

pub fn run_overlay(
    receiver: Receiver<super::OverlayFrame>,
    actions: Sender<UserAction>,
    stop: Arc<AtomicBool>,
    config_path: &std::path::Path,
    updates: crate::update::UpdateController,
) -> Result<()> {
    let mut config = crate::config::Config::load(config_path)?;
    let mut scan_interval_ms = config.scan_interval_ms;
    let mut settings_read = Instant::now();
    let _apartment = Apartment::new()?;
    let _dpi = DpiContext::new();
    // Shortcut conflicts must not prevent tray controls and passive rendering.
    let mut hotkeys = Hotkeys::new(&config.shortcuts);
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None)? }.0);
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(badge_proc),
        hInstance: instance,
        lpszClassName: w!("MayhemLensPassiveBadge"),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&window_class) };
    ensure!(atom != 0, "Impossible d'enregistrer la fenêtre overlay");
    let tray = Tray::new(instance)?;
    let mut settings_window: Option<crate::settings::SettingsWindow> = None;
    let renderer = BadgeRenderer::new()?;
    let mut display = match DisplayPublisher::new() {
        Ok(display) => Some(display),
        Err(error) => {
            // Diagnostics are optional and must not prevent rendering.
            eprintln!("État affichage indisponible : {error:#}");
            None
        }
    };
    let mut windows = Vec::<BadgeWindow>::new();
    let mut current = Vec::<Badge>::new();
    let mut published_geometry = None;
    let mut last_received = None;
    let mut showing = false;
    let mut redraw = false;
    while !stop.load(Ordering::Acquire) {
        if settings_read.elapsed() >= Duration::from_secs(2) {
            if let Ok(next) = crate::config::Config::load(config_path) {
                scan_interval_ms = next.scan_interval_ms;
                if next.shortcuts != config.shortcuts {
                    hotkeys.reload(&next.shortcuts);
                }
                if next.overlay_opacity != config.overlay_opacity {
                    redraw = true;
                }
                config = next;
            }
            settings_read = Instant::now();
        }
        let mut message = MSG::default();
        // SAFETY: processes only this thread's queue; no messages sent to LoL.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if let Some(settings) = settings_window.as_mut() {
                let handled = settings.process_message(&message);
                if settings.take_saved() {
                    // A save is observed immediately, not only by periodic reload.
                    if let Ok(next) = crate::config::Config::load(config_path) {
                        scan_interval_ms = next.scan_interval_ms;
                        hotkeys.reload(&next.shortcuts);
                        if next.overlay_opacity != config.overlay_opacity {
                            redraw = true;
                        }
                        config = next;
                    }
                }
                if !settings.is_open() {
                    settings_window = None;
                }
                if handled {
                    continue;
                }
            }
            if message.message == WM_QUIT {
                stop.store(true, Ordering::Release);
                break;
            }
            if tray.accepts(&message) {
                for window in &windows {
                    let _ = unsafe { ShowWindow(window.0, SW_HIDE) };
                }
                showing = false;
                if let Err(error) = tray.show_menu(
                    &actions,
                    &stop,
                    config_path,
                    &updates,
                    &hotkeys,
                    &mut settings_window,
                ) {
                    eprintln!("Menu Mayhem Lens : {error:#}");
                }
            } else if message.message == WM_HOTKEY {
                match message.wParam.0 as i32 {
                    HOTKEY_QUIT => stop.store(true, Ordering::Release),
                    HOTKEY_SCAN if game_window_visible() => {
                        let _ = actions.send(UserAction::ForceScan);
                    }
                    id @ HOTKEY_SLOT_1..=HOTKEY_SLOT_3 if game_window_visible() => {
                        let _ =
                            actions.send(UserAction::SelectSlot((id - HOTKEY_SLOT_1 + 1) as u8));
                    }
                    _ => {}
                }
            } else {
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        loop {
            match receiver.try_recv() {
                Ok(next) => {
                    last_received = Some(Instant::now());
                    if next.badges != current {
                        current = next.badges;
                        redraw = true;
                    }
                    // Retain the capture's geometry, not the window geometry at
                    // receipt: a resize can happen while OCR is still running.
                    published_geometry = next.game_bounds;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    stop.store(true, Ordering::Release);
                    break;
                }
            }
        }
        let game = game_window();
        let game_foreground = game.is_some_and(|hwnd| unsafe { GetForegroundWindow() == hwnd });
        let game_geometry = game.and_then(|hwnd| window_bounds(hwnd).ok());
        let context = DisplayContext {
            stopped: stop.load(Ordering::Acquire),
            game_visible: game.is_some(),
            game_foreground,
            game_geometry,
            published_geometry,
            badge_count: current.len(),
            fresh: badges_fresh(Instant::now(), last_received, scan_interval_ms),
        };
        let should_show = context.reason() == DisplayReason::Visible;
        if !should_show {
            if showing {
                for window in &windows {
                    let _ = unsafe { ShowWindow(window.0, SW_HIDE) };
                }
            }
            showing = false;
        } else {
            if redraw {
                windows.clear();
                for badge in &current {
                    let rect = safe_badge_bounds(
                        badge.rect,
                        game_geometry.context("Géométrie du jeu absente")?,
                    );
                    // SAFETY: owns a passive top-level HWND, never the game HWND.
                    let hwnd = unsafe {
                        CreateWindowExW(
                            WS_EX_LAYERED
                                | WS_EX_TRANSPARENT
                                | WS_EX_NOACTIVATE
                                | WS_EX_TOOLWINDOW
                                | WS_EX_TOPMOST,
                            w!("MayhemLensPassiveBadge"),
                            w!("Mayhem Lens"),
                            WS_POPUP,
                            rect.x,
                            rect.y,
                            rect.width,
                            rect.height,
                            None,
                            None,
                            Some(instance),
                            None,
                        )
                    }?;
                    let window = BadgeWindow(hwnd);
                    renderer.paint(hwnd, badge, rect, config.overlay_opacity)?;
                    windows.push(window);
                }
                redraw = false;
                showing = false;
            }
            if !showing {
                for window in &windows {
                    let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
                }
            }
            showing = true;
        }
        if let Some(display) = &mut display
            && display.due()
        {
            display.publish(display_status(
                &windows,
                &context,
                last_received,
                scan_interval_ms,
                &hotkeys,
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
    // HWNDs and hotkeys drop on their owning thread even when an error unwinds.
    Ok(())
}

fn badges_fresh(now: Instant, received: Option<Instant>, scan_interval_ms: u64) -> bool {
    received.is_some_and(|received| {
        now.saturating_duration_since(received)
            < Duration::from_millis(scan_interval_ms.saturating_add(1_500))
    })
}

fn safe_badge_bounds(rect: Rect, game: Rect) -> Rect {
    let width = rect.width.clamp(1, game.width.max(1));
    let height = rect.height.clamp(1, game.height.max(1));
    Rect {
        x: rect.x.clamp(game.x, game.x + game.width - width),
        y: rect.y.clamp(game.y, game.y + game.height - height),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_hints_cannot_block_forced_active_or_periodic_recognition() {
        for gate in [VisualGate::Absent, VisualGate::Uncertain] {
            assert!(!should_recognize(gate, false, false, false));
            assert!(should_recognize(gate, true, false, false));
            assert!(should_recognize(gate, false, true, false));
            assert!(should_recognize(gate, false, false, true));
        }
        assert!(should_recognize(VisualGate::Candidate, false, false, false));
    }

    #[test]
    fn full_probes_remain_due_after_idle_and_learned_deadlines() {
        let now = Instant::now();
        assert!(full_probe_due(None, now, false));
        assert!(!full_probe_due(
            Some(now),
            now + Duration::from_millis(2999),
            false
        ));
        assert!(full_probe_due(
            Some(now),
            now + Duration::from_secs(3),
            false
        ));
        assert!(!full_probe_due(
            Some(now),
            now + Duration::from_millis(4999),
            true
        ));
        assert!(full_probe_due(
            Some(now),
            now + Duration::from_secs(5),
            true
        ));
        // A slower configured scan can arrive after the target deadline.
        assert!(full_probe_due(
            Some(now),
            now + Duration::from_secs(8),
            true
        ));
    }

    #[test]
    fn regional_crops_isolate_rerolls_and_preserve_frame_projection() {
        let bounds = Rect {
            x: -200,
            y: -100,
            width: 200,
            height: 100,
        };
        let mut image = CapturedBand {
            pixels: [20, 20, 20, 255].repeat(80 * 30),
            left: 10,
            top: 10,
            width: 80,
            height: 30,
            source_width: 80,
            source_height: 30,
            frame_width: 100,
            frame_height: 50,
        };
        let first = Rect {
            x: -180,
            y: -60,
            width: 50,
            height: 20,
        };
        let second = Rect { x: -130, ..first };
        let before = image.crop(bounds, first).unwrap();
        let unchanged = image.crop(bounds, second).unwrap();
        // A changed glyph in the second line, beyond a previous short title.
        let offset = ((28 - 10) * image.width + (32 - 10)) * 4;
        image.pixels[offset] = 240;
        let after = image.crop(bounds, first).unwrap();
        assert_ne!(fingerprint(&before.pixels), fingerprint(&after.pixels));
        assert_eq!(
            fingerprint(&unchanged.pixels),
            fingerprint(&image.crop(bounds, second).unwrap().pixels)
        );
        assert_eq!(
            after.project(
                windows::Foundation::Rect {
                    X: 0.0,
                    Y: 0.0,
                    Width: 25.0,
                    Height: 10.0
                },
                bounds
            ),
            first
        );
        assert!(
            image
                .crop(
                    bounds,
                    Rect {
                        x: bounds.x,
                        ..first
                    }
                )
                .is_err()
        );
        image.fit_ocr(40).unwrap();
        assert!(image.crop(bounds, first).is_err());
    }

    #[test]
    fn hotkey_conflicts_preserve_other_bindings_and_safe_warning_codes() {
        let conflict = 0x8007_0581_u32 as i32;
        let mut attempted = Vec::new();
        let defaults = crate::config::ShortcutConfig::default();
        let registrations = register_hotkeys_with(&defaults, |id, key| {
            attempted.push((id, key));
            if matches!(id, HOTKEY_SCAN | HOTKEY_SLOT_2) {
                Err(conflict)
            } else {
                Ok(())
            }
        });
        // A conflict does not short-circuit later bindings, and only successes
        // enter the list that the real owning thread will unregister on Drop.
        assert_eq!(attempted.len(), 5);
        assert_eq!(
            registrations.registered,
            [HOTKEY_QUIT, HOTKEY_SLOT_1, HOTKEY_SLOT_3]
        );
        assert_eq!(
            registrations.unavailable,
            [
                HotkeyWarning {
                    shortcut: "Ctrl+Shift+M".into(),
                    error_code: "0x80070581".into(),
                },
                HotkeyWarning {
                    shortcut: "Ctrl+Shift+2".into(),
                    error_code: "0x80070581".into(),
                },
            ]
        );
        let warnings = serde_json::to_value(&registrations.unavailable).unwrap();
        assert_eq!(warnings[0]["shortcut"], "Ctrl+Shift+M");
        assert_eq!(warnings[0]["errorCode"], "0x80070581");
        assert_eq!(warnings[0].as_object().unwrap().len(), 2);

        // Even with no global bindings available, registration returns a usable
        // runtime state. No Windows hotkey API is invoked by either fake backend.
        let unavailable = register_hotkeys_with(&defaults, |_, _| Err(conflict));
        assert!(unavailable.registered.is_empty());
        assert_eq!(unavailable.unavailable.len(), 5);
    }

    #[test]
    fn display_visibility_rejects_stale_background_and_moved_game_geometry() {
        let game = Rect {
            x: -2_560,
            y: 0,
            width: 2_560,
            height: 1_440,
        };
        let ready = DisplayContext {
            stopped: false,
            game_visible: true,
            game_foreground: true,
            game_geometry: Some(game),
            published_geometry: Some(game),
            badge_count: 3,
            fresh: true,
        };
        assert_eq!(ready.reason(), DisplayReason::Visible);
        let hidden = [
            (
                DisplayContext {
                    stopped: true,
                    ..ready
                },
                DisplayReason::Stopped,
            ),
            (
                DisplayContext {
                    game_visible: false,
                    ..ready
                },
                DisplayReason::GameUnavailable,
            ),
            (
                DisplayContext {
                    game_foreground: false,
                    ..ready
                },
                DisplayReason::HiddenForeground,
            ),
            (
                DisplayContext {
                    game_geometry: None,
                    ..ready
                },
                DisplayReason::GeometryUnavailable,
            ),
            (
                DisplayContext {
                    badge_count: 0,
                    ..ready
                },
                DisplayReason::NoBadges,
            ),
            (
                DisplayContext {
                    fresh: false,
                    ..ready
                },
                DisplayReason::TtlExpired,
            ),
            (
                DisplayContext {
                    published_geometry: Some(Rect { x: 0, ..game }),
                    ..ready
                },
                DisplayReason::GeometryChanged,
            ),
        ];
        for (context, reason) in hidden {
            assert_eq!(context.reason(), reason);
            assert_ne!(context.reason(), DisplayReason::Visible);
        }
    }

    #[test]
    fn stalled_ocr_expires_badges_at_the_configured_scan_budget() {
        let now = Instant::now();
        assert!(!badges_fresh(now, None, 900));
        assert!(badges_fresh(
            now,
            Some(now - Duration::from_millis(2_399)),
            900
        ));
        assert!(!badges_fresh(
            now,
            Some(now - Duration::from_millis(2_400)),
            900
        ));
        assert!(!badges_fresh(
            now,
            Some(now - Duration::from_secs(60)),
            5_000
        ));
        // Changing the configured interval changes the allowance, not the
        // receipt timestamp; a fresh identical result extends it on reception.
        assert!(badges_fresh(
            now,
            Some(now - Duration::from_millis(3_000)),
            5_000
        ));
        assert!(!badges_fresh(
            now,
            Some(now - Duration::from_millis(3_000)),
            900
        ));
        assert!(badges_fresh(now, Some(now), 900));
    }

    #[test]
    fn oversized_roi_is_resized_without_losing_native_coordinate_scale() {
        let mut image = CapturedBand {
            pixels: [40, 80, 120, 255].repeat(8 * 4),
            left: 2,
            top: 1,
            width: 8,
            height: 4,
            source_width: 8,
            source_height: 4,
            frame_width: 20,
            frame_height: 10,
        };
        image.fit_ocr(4).unwrap();
        assert_eq!((image.width, image.height), (4, 2));
        assert_eq!((image.source_width, image.source_height), (8, 4));
        assert_eq!((image.left, image.top), (2, 1));
        assert_eq!(image.pixels, [40, 80, 120, 255].repeat(4 * 2));
        // Preserve location on a monitor to the left of the primary one, even
        // when capture pixels and physical window pixels have a different scale.
        let projected = image.project(
            windows::Foundation::Rect {
                X: 2.0,
                Y: 1.0,
                Width: 1.0,
                Height: 1.0,
            },
            Rect {
                x: -1920,
                y: 0,
                width: 40,
                height: 20,
            },
        );
        assert_eq!(
            projected,
            Rect {
                x: -1908,
                y: 6,
                width: 4,
                height: 4
            }
        );
    }

    #[test]
    fn wrapped_titles_join_only_inside_the_same_card() {
        let mut observations = vec![
            Observation {
                text: "Champion".into(),
                rect: Rect {
                    x: 100,
                    y: 100,
                    width: 90,
                    height: 20,
                },
            },
            Observation {
                text: "d'Urf".into(),
                rect: Rect {
                    x: 120,
                    y: 125,
                    width: 50,
                    height: 20,
                },
            },
            Observation {
                text: "Goliath".into(),
                rect: Rect {
                    x: 400,
                    y: 100,
                    width: 90,
                    height: 20,
                },
            },
        ];
        add_wrapped_titles(&mut observations);
        assert!(
            observations
                .iter()
                .any(|observation| observation.text == "Champion d'Urf")
        );
        assert!(
            !observations
                .iter()
                .any(|observation| observation.text.contains("Goliath ")
                    || observation.text.ends_with(" Goliath"))
        );
    }
}
