//! Desktop Duplication readback for the visible game's OCR region.
//!
//! No Windows Graphics Capture session, border toggle, capture picker or game
//! hook is involved. A private GPU texture retains only the game window so a
//! healthy duplication timeout can still recrop a changed OCR region. Only the
//! requested ROI is copied into a staging texture and CPU memory. The caller must check the game
//! foreground/window identity before and after this operation and call `reset`
//! whenever that identity, its session or foreground ownership changes.

use super::Rect;
use anyhow::{Context, Result, bail, ensure};
use std::ptr;
use windows::{
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
                ID3D11DeviceContext, ID3D11Texture2D,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT,
                DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC, IDXGIAdapter, IDXGIFactory1,
                IDXGIOutput1, IDXGIOutputDuplication,
            },
            Gdi::{HMONITOR, MONITOR_DEFAULTTONULL, MonitorFromRect},
        },
        UI::HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            SetThreadDpiAwarenessContext,
        },
    },
    core::Interface,
};

const FRAME_WAIT_MS: u32 = 200;

/// Packed BGRA pixels with offsets relative to the physical game window.
#[derive(Clone, Debug)]
pub(crate) struct DesktopBand {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub left: u32,
    pub top: u32,
    pub source_width: usize,
    pub source_height: usize,
    pub frame_width: usize,
    pub frame_height: usize,
    /// False for a healthy duplication timeout or a pointer-only update. Such a
    /// result must not be reported as a newly presented desktop image.
    pub fresh_frame: bool,
}

/// Lazy: constructing this value does not initialize a GPU or capture anything.
#[derive(Default)]
pub(crate) struct DesktopCapture {
    source: Option<DesktopSource>,
}

impl DesktopCapture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.source = None;
    }

    pub fn capture(&mut self, bounds: Rect, roi: Rect) -> Result<DesktopBand> {
        let _dpi = CaptureDpi::new();
        let result = self.capture_inner(bounds, roi);
        if result.is_err() {
            // Includes ACCESS_LOST, mode changes, device loss and rejected
            // geometry. Never reuse either pixels or an invalid duplication.
            self.reset();
        }
        result
    }

    fn capture_inner(&mut self, bounds: Rect, roi: Rect) -> Result<DesktopBand> {
        let game_rect = native_rect(bounds)?;
        // SAFETY: the initialized RECT remains live; this only queries an OS
        // monitor handle, and uses physical coordinates in this thread's DPI context.
        let monitor = unsafe { MonitorFromRect(&game_rect, MONITOR_DEFAULTTONULL) };
        ensure!(
            !monitor.0.is_null(),
            "La fenêtre LoL n'est sur aucun écran actif"
        );

        let reuse = if let Some(source) = &self.source {
            source.current_for(monitor, bounds)?
        } else {
            false
        };
        if !reuse {
            self.source = None;
            self.source = Some(DesktopSource::new(monitor, bounds)?);
        }
        self.source
            .as_mut()
            .context("Source Desktop Duplication absente")?
            .capture(bounds, roi)
    }
}

struct CaptureDpi(DPI_AWARENESS_CONTEXT);
impl CaptureDpi {
    fn new() -> Self {
        // SAFETY: changes only the capture thread; Drop restores its previous context.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
impl Drop for CaptureDpi {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // SAFETY: this saved context belongs to the same thread and is restored once.
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

struct DesktopSource {
    output: IDXGIOutput1,
    description: DXGI_OUTPUT_DESC,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    staging_size: (u32, u32),
    game_frame: Option<GameFrame>,
    cached: Option<CachedBand>,
}

/// Owned independently of the acquired DXGI surface. No frame lease or CPU
/// image of the complete game window is retained here.
struct GameFrame {
    bounds: Rect,
    texture: ID3D11Texture2D,
}

impl DesktopSource {
    fn new(monitor: HMONITOR, bounds: Rect) -> Result<Self> {
        // SAFETY: typed factory creation returns an owned COM reference, with no
        // graphics capture operation or UI request.
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }?;
        let mut adapter_index = 0;
        loop {
            // SAFETY: the index is an ordinary adapter enumeration parameter.
            let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
                Ok(adapter) => adapter,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(error).context("Énumération des adaptateurs DXGI"),
            };
            let mut output_index = 0;
            loop {
                // SAFETY: the output index is enumerated on this live adapter.
                let output = match unsafe { adapter.EnumOutputs(output_index) } {
                    Ok(output) => output,
                    Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(error) => return Err(error).context("Énumération des écrans DXGI"),
                };
                // SAFETY: read-only descriptor query on this owned output.
                let description = unsafe { output.GetDesc() }?;
                if description.Monitor == monitor && description.AttachedToDesktop.as_bool() {
                    validate_output(description, bounds)?;
                    let output: IDXGIOutput1 = output.cast()?;
                    let adapter: IDXGIAdapter = adapter.cast()?;
                    return Self::on_output(output, description, adapter);
                }
                output_index += 1;
            }
            adapter_index += 1;
        }
        bail!("Aucune sortie DXGI active ne correspond à l'écran de LoL")
    }

    fn on_output(
        output: IDXGIOutput1,
        description: DXGI_OUTPUT_DESC,
        adapter: IDXGIAdapter,
    ) -> Result<Self> {
        let mut device = None;
        let mut context = None;
        // SAFETY: both out-pointers are initialized owned COM slots. The explicit
        // adapter is the one owning the selected output; UNKNOWN is required
        // when an adapter is supplied, rather than the implicit default GPU.
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .context("Création du device D3D11 sur l'adaptateur de l'écran LoL")?;
        let device = device.context("D3D11 n'a pas retourné de device")?;
        let context = context.context("D3D11 n'a pas retourné de contexte")?;
        // SAFETY: this device was created on this output's exact adapter. Desktop
        // Duplication uses the OS desktop surface without touching the game process.
        let duplication = unsafe { output.DuplicateOutput(&device) }
            .context("Initialisation Desktop Duplication de l'écran LoL")?;
        // SAFETY: read-only descriptor query on the owned duplication interface.
        let duplicate_desc = unsafe { duplication.GetDesc() };
        ensure!(
            duplicate_desc.Rotation == DXGI_MODE_ROTATION_IDENTITY,
            "La capture ne prend pas encore en charge un écran pivoté"
        );
        let screen = rect_from_native(description.DesktopCoordinates)?;
        ensure!(
            duplicate_desc.ModeDesc.Width == screen.width as u32
                && duplicate_desc.ModeDesc.Height == screen.height as u32
                && duplicate_desc.ModeDesc.Format == DXGI_FORMAT_B8G8R8A8_UNORM,
            "Dimensions ou format Desktop Duplication non pris en charge"
        );
        Ok(Self {
            output,
            description,
            device,
            context,
            duplication,
            staging: None,
            staging_size: (0, 0),
            game_frame: None,
            cached: None,
        })
    }

    fn current_for(&self, monitor: HMONITOR, bounds: Rect) -> Result<bool> {
        // SAFETY: read-only descriptor query on the current owned output.
        let current = unsafe { self.output.GetDesc() }?;
        if current.Monitor != monitor
            || !current.AttachedToDesktop.as_bool()
            || current.DesktopCoordinates != self.description.DesktopCoordinates
            || current.Rotation != self.description.Rotation
        {
            return Ok(false);
        }
        validate_output(current, bounds)?;
        Ok(true)
    }

    fn capture(&mut self, bounds: Rect, roi: Rect) -> Result<DesktopBand> {
        let game_region = desktop_region(
            bounds,
            bounds,
            rect_from_native(self.description.DesktopCoordinates)?,
        )?;
        // Validate the requested crop before acquiring any GPU frame.
        let region = game_frame_region(Some(bounds), bounds, roi)?;
        let key = CacheKey { bounds, roi };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        // SAFETY: initialized out-parameters stay live for the bounded acquire;
        // no prior frame is retained between calls, including error paths.
        let acquired = unsafe {
            self.duplication
                .AcquireNextFrame(FRAME_WAIT_MS, &mut info, &mut resource)
        };
        match acquired {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                if let Some(band) = timeout_band(&self.cached, key) {
                    return Ok(band);
                }
                // The duplication is healthy and no desktop image has changed.
                // A learned/discovery ROI transition can recrop the privately
                // owned GPU window snapshot without acquiring a new desktop frame.
                let snapshot = self.game_frame.as_ref();
                let region = game_frame_region(snapshot.map(|frame| frame.bounds), bounds, roi)?;
                let texture = snapshot
                    .context("Aucune image DXGI de la fenêtre LoL en cache")?
                    .texture
                    .clone();
                let band = self.copy_region(&texture, region, bounds, false)?;
                self.cached = Some(CachedBand {
                    key,
                    band: band.clone(),
                });
                return Ok(band);
            }
            Err(error) => return Err(error).context("Acquisition de l'image Desktop Duplication"),
        }
        // This guard is established immediately after successful acquisition,
        // before any resource validation, COM cast, allocation or GPU operation.
        let frame = FrameLease::new(self.duplication.clone());
        ensure!(
            !info.ProtectedContentMaskedOut.as_bool(),
            "Windows a masqué du contenu protégé dans l'image capturée"
        );
        let resource = resource.context("DXGI n'a pas retourné d'image desktop")?;
        let texture: ID3D11Texture2D = resource.cast()?;
        let result = (|| {
            self.cached = None;
            self.store_game_frame(&texture, game_region, bounds)?;
            let owned = self
                .game_frame
                .as_ref()
                .context("Image GPU privée de la fenêtre LoL absente")?
                .texture
                .clone();
            // Map in copy_region waits for both GPU copies to finish before
            // ReleaseFrame makes the acquired desktop surface invalid.
            self.copy_region(&owned, region, bounds, info.LastPresentTime != 0)
        })();
        // Release even when readback fails. Report a ReleaseFrame failure rather
        // than accepting a frame that might leave the interface unusable.
        let released = frame.release();
        let band = result?;
        released.context("Libération de l'image Desktop Duplication")?;
        self.cached = Some(CachedBand {
            key,
            band: band.clone(),
        });
        Ok(band)
    }

    fn store_game_frame(
        &mut self,
        desktop: &ID3D11Texture2D,
        region: DesktopRegion,
        bounds: Rect,
    ) -> Result<()> {
        validate_texture_region(desktop, region)?;
        if self
            .game_frame
            .as_ref()
            .is_none_or(|frame| frame.bounds != bounds)
        {
            let description = game_texture_description(bounds)?;
            let mut texture = None;
            // SAFETY: the private DEFAULT texture covers only the validated game
            // window, has no CPU access, and belongs to this output's device.
            unsafe {
                self.device
                    .CreateTexture2D(&description, None, Some(&mut texture))
            }?;
            self.game_frame = Some(GameFrame {
                bounds,
                texture: texture.context("Texture GPU privée de la fenêtre LoL absente")?,
            });
        }
        let owned = &self
            .game_frame
            .as_ref()
            .context("Texture GPU privée de la fenêtre LoL absente")?
            .texture;
        let source_box = texture_box(region);
        // SAFETY: source coordinates were checked against the acquired desktop;
        // the destination has exactly the game's dimensions. No other monitor
        // or pixels outside the game window are copied to the private snapshot.
        unsafe {
            self.context
                .CopySubresourceRegion(owned, 0, 0, 0, 0, desktop, 0, Some(&source_box));
        }
        Ok(())
    }

    fn copy_region(
        &mut self,
        texture: &ID3D11Texture2D,
        region: DesktopRegion,
        bounds: Rect,
        fresh_frame: bool,
    ) -> Result<DesktopBand> {
        validate_texture_region(texture, region)?;
        if self.staging_size != (region.width, region.height) {
            let description = D3D11_TEXTURE2D_DESC {
                Width: region.width,
                Height: region.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut staging = None;
            // SAFETY: the description has validated positive ROI dimensions;
            // the initialized output slot receives an owned ROI-only texture.
            unsafe {
                self.device
                    .CreateTexture2D(&description, None, Some(&mut staging))
            }?;
            self.staging = Some(staging.context("Texture ROI D3D11 absente")?);
            self.staging_size = (region.width, region.height);
        }
        let staging = self
            .staging
            .as_ref()
            .context("Texture de recadrage absente")?;
        let stride = region.width as usize * 4;
        let length = stride
            .checked_mul(region.height as usize)
            .context("Région OCR trop grande")?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(length)
            .context("Allocation de la région OCR")?;
        pixels.resize(length, 0);
        let source_box = texture_box(region);
        let mut mapping = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: both resources are on the selected adapter's device; the source
        // box was checked against the still-acquired texture. Only ROI pixels are
        // copied to CPU-readable memory; the full desktop is never stored here.
        unsafe {
            self.context
                .CopySubresourceRegion(staging, 0, 0, 0, 0, texture, 0, Some(&source_box));
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapping))?;
        }
        let mapped = MappingLease {
            context: &self.context,
            texture: staging,
        };
        ensure!(
            !mapping.pData.is_null() && mapping.RowPitch as usize >= stride,
            "Mémoire de lecture D3D11 invalide"
        );
        // SAFETY: Map supplies at least RowPitch bytes for each ROI row. The
        // validated stride fits each row and the packed destination allocation.
        unsafe {
            for row in 0..region.height as usize {
                ptr::copy_nonoverlapping(
                    (mapping.pData as *const u8).add(row * mapping.RowPitch as usize),
                    pixels.as_mut_ptr().add(row * stride),
                    stride,
                );
            }
        }
        drop(mapped);
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        Ok(DesktopBand {
            pixels,
            width: region.width as usize,
            height: region.height as usize,
            left: region.window_left,
            top: region.window_top,
            source_width: region.width as usize,
            source_height: region.height as usize,
            frame_width: bounds.width as usize,
            frame_height: bounds.height as usize,
            fresh_frame,
        })
    }
}

struct FrameLease {
    duplication: Option<IDXGIOutputDuplication>,
}
impl FrameLease {
    fn new(duplication: IDXGIOutputDuplication) -> Self {
        Self {
            duplication: Some(duplication),
        }
    }

    fn release(mut self) -> windows::core::Result<()> {
        let duplication = self.duplication.take().expect("live frame lease");
        // SAFETY: this guard exists only after successful AcquireNextFrame and
        // consumes its sole release obligation; no frame texture is used afterward.
        unsafe { duplication.ReleaseFrame() }
    }
}
impl Drop for FrameLease {
    fn drop(&mut self) {
        if let Some(duplication) = self.duplication.take() {
            // SAFETY: early-return cleanup releases the one acquired frame once.
            let _ = unsafe { duplication.ReleaseFrame() };
        }
    }
}

struct MappingLease<'a> {
    context: &'a ID3D11DeviceContext,
    texture: &'a ID3D11Texture2D,
}
impl Drop for MappingLease<'_> {
    fn drop(&mut self) {
        // SAFETY: created only after successful Map; this same resource/subresource
        // is unmapped exactly once, including failed pitch validation.
        unsafe { self.context.Unmap(self.texture, 0) };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CacheKey {
    bounds: Rect,
    roi: Rect,
}
struct CachedBand {
    key: CacheKey,
    band: DesktopBand,
}

fn timeout_band(cached: &Option<CachedBand>, key: CacheKey) -> Option<DesktopBand> {
    cached
        .as_ref()
        .filter(|cached| cached.key == key)
        .map(|cached| {
            let mut band = cached.band.clone();
            band.fresh_frame = false;
            band
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DesktopRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    window_left: u32,
    window_top: u32,
}

fn game_texture_description(bounds: Rect) -> Result<D3D11_TEXTURE2D_DESC> {
    native_rect(bounds)?;
    Ok(D3D11_TEXTURE2D_DESC {
        Width: bounds.width.try_into()?,
        Height: bounds.height.try_into()?,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        ..Default::default()
    })
}

/// A changed OCR ROI is safe only when the retained game snapshot has the exact
/// same physical window bounds. Coordinates here are private-texture offsets.
fn game_frame_region(
    snapshot_bounds: Option<Rect>,
    bounds: Rect,
    roi: Rect,
) -> Result<DesktopRegion> {
    ensure!(
        snapshot_bounds == Some(bounds),
        "Image GPU de la fenêtre LoL absente ou périmée"
    );
    desktop_region(bounds, roi, bounds)
}

fn validate_texture_region(texture: &ID3D11Texture2D, region: DesktopRegion) -> Result<()> {
    let mut description = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: this read-only descriptor query targets either an acquired desktop
    // surface under its live frame lease or our independently owned GPU texture.
    unsafe { texture.GetDesc(&mut description) };
    ensure!(
        description.Format == DXGI_FORMAT_B8G8R8A8_UNORM
            && description.SampleDesc.Count == 1
            && region
                .x
                .checked_add(region.width)
                .is_some_and(|right| right <= description.Width)
            && region
                .y
                .checked_add(region.height)
                .is_some_and(|bottom| bottom <= description.Height),
        "La région de capture dépasse la texture GPU"
    );
    Ok(())
}

fn texture_box(region: DesktopRegion) -> D3D11_BOX {
    D3D11_BOX {
        left: region.x,
        top: region.y,
        front: 0,
        right: region.x + region.width,
        bottom: region.y + region.height,
        back: 1,
    }
}

fn validate_output(description: DXGI_OUTPUT_DESC, bounds: Rect) -> Result<()> {
    ensure!(
        description.AttachedToDesktop.as_bool(),
        "L'écran LoL est déconnecté"
    );
    ensure!(
        description.Rotation == DXGI_MODE_ROTATION_IDENTITY,
        "La capture ne prend pas encore en charge un écran pivoté"
    );
    ensure!(
        contains(rect_from_native(description.DesktopCoordinates)?, bounds),
        "La fenêtre LoL doit être entièrement sur un seul écran"
    );
    Ok(())
}

fn desktop_region(bounds: Rect, roi: Rect, screen: Rect) -> Result<DesktopRegion> {
    ensure!(
        contains(screen, bounds),
        "La fenêtre LoL dépasse l'écran capturé"
    );
    ensure!(
        contains(bounds, roi),
        "La région OCR dépasse la fenêtre LoL"
    );
    Ok(DesktopRegion {
        x: (i64::from(roi.x) - i64::from(screen.x)).try_into()?,
        y: (i64::from(roi.y) - i64::from(screen.y)).try_into()?,
        width: roi.width.try_into()?,
        height: roi.height.try_into()?,
        window_left: (i64::from(roi.x) - i64::from(bounds.x)).try_into()?,
        window_top: (i64::from(roi.y) - i64::from(bounds.y)).try_into()?,
    })
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer.width > 0
        && outer.height > 0
        && inner.width > 0
        && inner.height > 0
        && inner.x >= outer.x
        && inner.y >= outer.y
        && i64::from(inner.x) + i64::from(inner.width)
            <= i64::from(outer.x) + i64::from(outer.width)
        && i64::from(inner.y) + i64::from(inner.height)
            <= i64::from(outer.y) + i64::from(outer.height)
}

fn native_rect(rect: Rect) -> Result<RECT> {
    ensure!(
        rect.width > 0 && rect.height > 0,
        "Dimensions de fenêtre invalides"
    );
    Ok(RECT {
        left: rect.x,
        top: rect.y,
        right: rect
            .x
            .checked_add(rect.width)
            .context("Coordonnée droite invalide")?,
        bottom: rect
            .y
            .checked_add(rect.height)
            .context("Coordonnée basse invalide")?,
    })
}

fn rect_from_native(rect: RECT) -> Result<Rect> {
    let result = Rect {
        x: rect.left,
        y: rect.top,
        width: rect
            .right
            .checked_sub(rect.left)
            .context("Largeur d'écran invalide")?,
        height: rect
            .bottom
            .checked_sub(rect.top)
            .context("Hauteur d'écran invalide")?,
    };
    ensure!(
        result.width > 0 && result.height > 0,
        "Dimensions d'écran invalides"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_monitor_coordinates_project_to_output_and_window_independently() {
        let screen = Rect {
            x: -2560,
            y: -1440,
            width: 2560,
            height: 1440,
        };
        let bounds = Rect {
            x: -2400,
            y: -1300,
            width: 1920,
            height: 1080,
        };
        let roi = Rect {
            x: -2280,
            y: -1190,
            width: 1680,
            height: 700,
        };
        let region = desktop_region(bounds, roi, screen).unwrap();
        assert_eq!(
            region,
            DesktopRegion {
                x: 280,
                y: 250,
                width: 1680,
                height: 700,
                window_left: 120,
                window_top: 110
            }
        );
    }

    #[test]
    fn crossing_an_output_or_game_boundary_is_rejected_without_cropping() {
        let screen = Rect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let roi = Rect {
            x: 150,
            y: 150,
            width: 1600,
            height: 700,
        };
        assert!(
            desktop_region(
                Rect {
                    x: -1,
                    y: 0,
                    width: 2560,
                    height: 1440
                },
                roi,
                screen
            )
            .is_err()
        );
        assert!(
            desktop_region(
                screen,
                Rect {
                    x: 2400,
                    y: 100,
                    width: 200,
                    height: 500
                },
                screen
            )
            .is_err()
        );
        assert!(desktop_region(screen, Rect { width: 0, ..roi }, screen).is_err());
    }

    #[test]
    fn physical_projection_does_not_apply_a_second_dpi_scale() {
        let bounds = Rect {
            x: 3840,
            y: 0,
            width: 3840,
            height: 2160,
        };
        let roi = Rect {
            x: 4140,
            y: 300,
            width: 2400,
            height: 900,
        };
        let region = desktop_region(bounds, roi, bounds).unwrap();
        assert_eq!(region.x, 300);
        assert_eq!(region.window_left, 300);
        assert_eq!((region.width, region.height), (2400, 900));
    }

    #[test]
    fn a_timeout_never_changes_region_identity_or_claims_a_fresh_frame() {
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let roi = Rect {
            x: 100,
            y: 100,
            width: 1,
            height: 1,
        };
        let key = CacheKey { bounds, roi };
        let cached = Some(CachedBand {
            key,
            band: DesktopBand {
                pixels: vec![10, 20, 30, 255],
                width: 1,
                height: 1,
                left: 100,
                top: 100,
                source_width: 1,
                source_height: 1,
                frame_width: 1920,
                frame_height: 1080,
                fresh_frame: true,
            },
        });
        let band = timeout_band(&cached, key).unwrap();
        assert!(!band.fresh_frame);
        assert_eq!(band.pixels, [10, 20, 30, 255]);
        assert!(
            timeout_band(
                &cached,
                CacheKey {
                    roi: Rect { x: 101, ..roi },
                    ..key
                }
            )
            .is_none()
        );
        assert!(
            timeout_band(
                &cached,
                CacheKey {
                    bounds: Rect {
                        width: 2560,
                        ..bounds
                    },
                    ..key
                }
            )
            .is_none()
        );
        assert!(timeout_band(&None, key).is_none());
    }

    #[test]
    fn a_static_game_snapshot_allows_discovery_learned_and_discovery_recrops() {
        let bounds = Rect {
            x: -2560,
            y: 120,
            width: 2560,
            height: 1440,
        };
        let discovery = Rect {
            x: -2407,
            y: 264,
            width: 2252,
            height: 1123,
        };
        let learned = Rect {
            x: -2407,
            y: 264,
            width: 2252,
            height: 530,
        };
        // Neither narrowing after two stable reads nor a later broad probe needs
        // a newly presented desktop image: the private snapshot covers both.
        for roi in [discovery, learned, discovery] {
            let region = game_frame_region(Some(bounds), bounds, roi).unwrap();
            assert_eq!(region.x, 153);
            assert_eq!(region.y, 144);
            assert_eq!(
                (region.width, region.height),
                (roi.width as u32, roi.height as u32)
            );
            assert_eq!(region.x, region.window_left);
            assert_eq!(region.y, region.window_top);
        }
    }

    #[test]
    fn a_private_snapshot_cannot_supply_another_window_geometry_or_external_pixels() {
        let bounds = Rect {
            x: 200,
            y: 100,
            width: 1920,
            height: 1080,
        };
        let roi = Rect {
            x: 320,
            y: 210,
            width: 1680,
            height: 700,
        };
        assert!(game_frame_region(None, bounds, roi).is_err());
        for changed in [
            Rect { x: 201, ..bounds },
            Rect {
                width: 1919,
                ..bounds
            },
        ] {
            assert!(game_frame_region(Some(changed), bounds, roi).is_err());
        }
        assert!(game_frame_region(Some(bounds), bounds, Rect { x: 199, ..roi }).is_err());
    }

    #[test]
    fn the_retained_game_snapshot_is_gpu_only_and_sized_to_the_game_window() {
        let bounds = Rect {
            x: -1000,
            y: 300,
            width: 2560,
            height: 1440,
        };
        let description = game_texture_description(bounds).unwrap();
        assert_eq!((description.Width, description.Height), (2560, 1440));
        assert_eq!(description.Usage, D3D11_USAGE_DEFAULT);
        assert_eq!(description.CPUAccessFlags, 0);
        assert_eq!(description.BindFlags, 0);
        assert_eq!(description.Format, DXGI_FORMAT_B8G8R8A8_UNORM);
        assert_eq!(description.SampleDesc.Count, 1);
        assert!(game_texture_description(Rect { width: 0, ..bounds }).is_err());
        assert!(
            game_texture_description(Rect {
                x: i32::MAX,
                ..bounds
            })
            .is_err()
        );
    }

    #[test]
    fn zero_sized_and_overflowing_native_rectangles_are_rejected() {
        assert!(
            native_rect(Rect {
                x: i32::MAX - 10,
                y: 0,
                width: 100,
                height: 100
            })
            .is_err()
        );
        assert!(
            native_rect(Rect {
                width: 100,
                height: 0,
                ..Rect::default()
            })
            .is_err()
        );
        assert!(
            rect_from_native(RECT {
                left: i32::MIN,
                top: 0,
                right: i32::MAX,
                bottom: 100
            })
            .is_err()
        );
    }
}
