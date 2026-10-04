//! Native MSIX updates. Loading this module performs no network or deployment.
//! The controller runs both launch and manual requests on one background worker.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

pub const APPINSTALLER_URL: &str =
    "https://github.com/Aleqsd/mayhem-lens/releases/latest/download/MayhemLens.appinstaller";
const PACKAGE_NAME: &str = "Aleqsd.MayhemLens";
const PACKAGE_PUBLISHER: &str = "CN=Alexandre DO-O ALMEIDA";
const MINIMUM_DEFERRED_BUILD: u32 = 22_621;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Checking,
    UpToDate,
    Available,
    Preparing,
    ReadyOnRestart,
    Registered,
    Unassociated,
    Unsupported,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub phase: UpdatePhase,
    pub installed_version: Option<String>,
    pub checked_at_unix: u64,
    pub detail: String,
}

impl UpdateStatus {
    fn new(phase: UpdatePhase, version: Option<String>, detail: impl Into<String>) -> Self {
        Self {
            phase,
            installed_version: version,
            checked_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            detail: detail.into(),
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, UpdatePhase::Checking | UpdatePhase::Preparing)
    }

    pub fn summary(&self, language: &str) -> &'static str {
        match (self.phase, language == "en") {
            (UpdatePhase::Checking, false) => "Recherche de mise à jour…",
            (UpdatePhase::Checking, true) => "Checking for updates…",
            (UpdatePhase::UpToDate, false) => "Application à jour",
            (UpdatePhase::UpToDate, true) => "Application is up to date",
            (UpdatePhase::Available, false) => "Mise à jour disponible",
            (UpdatePhase::Available, true) => "Update available",
            (UpdatePhase::Preparing, false) => "Préparation de la mise à jour…",
            (UpdatePhase::Preparing, true) => "Preparing update…",
            (UpdatePhase::ReadyOnRestart, false) => "Mise à jour préparée — prochain lancement",
            (UpdatePhase::ReadyOnRestart, true) => "Update prepared — next launch",
            (UpdatePhase::Registered, false) => "Mise à jour enregistrée — relancer l’overlay",
            (UpdatePhase::Registered, true) => "Update registered — relaunch the overlay",
            (UpdatePhase::Unassociated, false) => "Canal de mise à jour non associé",
            (UpdatePhase::Unassociated, true) => "Update channel is not associated",
            (UpdatePhase::Unsupported, false) => "Mise à jour différée : Windows 11 22H2 requis",
            (UpdatePhase::Unsupported, true) => "Deferred updates require Windows 11 22H2",
            (UpdatePhase::Error, false) => "Mise à jour indisponible — réessayer",
            (UpdatePhase::Error, true) => "Update unavailable — retry",
        }
    }
}

/// Clones share one bounded request queue and one state. Dropping the last
/// controller disconnects the worker; shutdown never joins a network operation.
#[derive(Clone)]
pub struct UpdateController {
    request: SyncSender<()>,
    state: Arc<Mutex<UpdateStatus>>,
}

impl UpdateController {
    pub fn start() -> Result<Self> {
        let (request, requests) = mpsc::sync_channel(1);
        let state = Arc::new(Mutex::new(UpdateStatus::new(
            UpdatePhase::Checking,
            None,
            "",
        )));
        let worker_state = Arc::clone(&state);
        thread::Builder::new()
            .name("mayhem-update".into())
            .spawn(move || {
                // This first request happens for every actual app launch, including
                // an execution alias, independently of App Installer's launch hook.
                run_request(&worker_state);
                while requests.recv().is_ok() {
                    run_request(&worker_state);
                }
            })
            .context("Création du worker de mise à jour")?;
        Ok(Self { request, state })
    }

    pub fn request_update(&self) -> bool {
        !self.status().busy() && self.request.try_send(()).is_ok()
    }

    pub fn status(&self) -> UpdateStatus {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| {
                UpdateStatus::new(UpdatePhase::Error, None, "État de mise à jour indisponible")
            })
    }
}

pub fn status_path() -> PathBuf {
    crate::config::app_directory().join("update-status.json")
}

fn publish(state: &Mutex<UpdateStatus>, status: UpdateStatus) {
    if let Ok(mut current) = state.lock() {
        *current = status.clone();
    }
    // Persist only update status/version/errors, never credentials or game data.
    let save = (|| -> Result<()> {
        let path = status_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&status)?)?;
        fs::rename(temporary, path)?;
        Ok(())
    })();
    if let Err(error) = save {
        eprintln!("État mise à jour : {error:#}");
    }
}

fn run_request(state: &Mutex<UpdateStatus>) {
    publish(state, UpdateStatus::new(UpdatePhase::Checking, None, ""));
    let status = check_and_stage_with(|status| publish(state, status));
    match status {
        Ok(status) => publish(state, status),
        Err(error) => publish(
            state,
            UpdateStatus::new(UpdatePhase::Error, None, format!("{error:#}")),
        ),
    }
}

/// Read-only check. Does not start an installer or prepare a deployment.
pub fn check() -> Result<UpdateStatus> {
    #[cfg(windows)]
    {
        windows_update::check()
    }
    #[cfg(not(windows))]
    {
        Ok(UpdateStatus::new(
            UpdatePhase::Unsupported,
            None,
            "Windows requis",
        ))
    }
}

/// Queues a signed MSIX update using Windows. It never requests shutdown,
/// restarts the app, opens a window, or executes a downloaded binary itself.
pub fn check_and_stage() -> Result<UpdateStatus> {
    check_and_stage_with(|_| {})
}

fn check_and_stage_with(progress: impl Fn(UpdateStatus)) -> Result<UpdateStatus> {
    #[cfg(windows)]
    {
        windows_update::check_and_stage(progress)
    }
    #[cfg(not(windows))]
    {
        let _ = progress;
        check()
    }
}

fn windows_build(packed_version: &str) -> Result<u32> {
    let value: u64 = packed_version
        .parse()
        .context("Version Windows WinRT invalide")?;
    Ok(((value >> 16) & 0xffff) as u32)
}

#[cfg(windows)]
fn availability_phase(
    availability: windows::ApplicationModel::PackageUpdateAvailability,
) -> UpdatePhase {
    use windows::ApplicationModel::PackageUpdateAvailability;
    match availability {
        PackageUpdateAvailability::NoUpdates => UpdatePhase::UpToDate,
        PackageUpdateAvailability::Available | PackageUpdateAvailability::Required => {
            UpdatePhase::Available
        }
        // Unknown includes failures unrelated to App Installer association.
        // Only GetAppInstallerInfo below can establish that it is absent.
        _ => UpdatePhase::Error,
    }
}

fn staging_allowed(phase: UpdatePhase) -> bool {
    matches!(phase, UpdatePhase::Available | UpdatePhase::Unassociated)
}

fn deployed_phase(is_registered: bool) -> UpdatePhase {
    if is_registered {
        UpdatePhase::Registered
    } else {
        UpdatePhase::ReadyOnRestart
    }
}

#[cfg(windows)]
mod windows_update {
    use super::*;
    use anyhow::ensure;
    use windows::{
        ApplicationModel::Package,
        Foundation::Uri,
        Management::Deployment::{AddPackageOptions, PackageManager},
        System::Profile::AnalyticsInfo,
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        core::HSTRING,
    };

    struct Apartment;
    impl Apartment {
        fn new() -> Result<Self> {
            // SAFETY: this dedicated caller thread balances WinRT after all its
            // package/deployment objects have been dropped.
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }

    struct PackageContext {
        package: Package,
        manager: PackageManager,
        version: String,
        build: u32,
        _apartment: Apartment,
    }
    impl PackageContext {
        fn new() -> Result<Self> {
            let apartment = Apartment::new()?;
            let build = windows_build(
                &AnalyticsInfo::VersionInfo()?
                    .DeviceFamilyVersion()?
                    .to_string(),
            )?;
            let current =
                Package::Current().context("L'autoupdate exige l'application MSIX installée")?;
            let id = current.Id()?;
            ensure!(
                id.Name()? == PACKAGE_NAME && id.Publisher()? == PACKAGE_PUBLISHER,
                "L'identité du package ne correspond pas au canal Mayhem Lens"
            );
            let value = id.Version()?;
            let version = format!(
                "{}.{}.{}.{}",
                value.Major, value.Minor, value.Build, value.Revision
            );
            let manager = PackageManager::new()?;
            // CheckUpdateAvailabilityAsync on Package.Current itself can return
            // AccessDenied. Microsoft's documented workaround uses this instance.
            let package = manager
                .FindPackageByUserSecurityIdPackageFullName(&HSTRING::new(), &id.FullName()?)?;
            Ok(Self {
                package,
                manager,
                version,
                build,
                _apartment: apartment,
            })
        }

        fn check(&self) -> Result<UpdateStatus> {
            if self.build < MINIMUM_DEFERRED_BUILD {
                return Ok(UpdateStatus::new(
                    UpdatePhase::Unsupported,
                    Some(self.version.clone()),
                    format!("Windows build {}", self.build),
                ));
            }
            let info = match self.package.GetAppInstallerInfo() {
                Ok(info) => info,
                // A raw-MSIX install has no AppInstallerInfo and maps WinRT null
                // to E_POINTER. Do not turn unrelated failures into 'up to date'.
                Err(error) if error.code().0 == 0x80004003_u32 as i32 => {
                    return Ok(UpdateStatus::new(
                        UpdatePhase::Unassociated,
                        Some(self.version.clone()),
                        "",
                    ));
                }
                Err(error) => return Err(error).context("Lecture du canal App Installer"),
            };
            let associated_uri = info.Uri()?.AbsoluteUri()?.to_string();
            if associated_uri != APPINSTALLER_URL {
                return Ok(UpdateStatus::new(
                    UpdatePhase::Unassociated,
                    Some(self.version.clone()),
                    "Canal stable non associé",
                ));
            }
            let result = self.package.CheckUpdateAvailabilityAsync()?.join()?;
            let availability = result.Availability()?;
            let phase = availability_phase(availability);
            let detail = if phase == UpdatePhase::Error {
                format!(
                    "Disponibilité mise à jour indéterminée ({}, HRESULT {:08X})",
                    availability.0,
                    result.ExtendedError()?.0 as u32
                )
            } else {
                String::new()
            };
            Ok(UpdateStatus::new(phase, Some(self.version.clone()), detail))
        }
    }

    pub fn check() -> Result<UpdateStatus> {
        PackageContext::new()?.check()
    }

    pub fn check_and_stage(progress: impl Fn(UpdateStatus)) -> Result<UpdateStatus> {
        let context = PackageContext::new()?;
        let checked = context.check()?;
        if !staging_allowed(checked.phase) {
            return Ok(checked);
        }
        progress(UpdateStatus::new(
            UpdatePhase::Preparing,
            Some(context.version.clone()),
            "",
        ));
        let options = AddPackageOptions::new()?;
        options.SetDeferRegistrationWhenPackagesAreInUse(true)?;
        options.SetForceAppShutdown(false)?;
        options.SetForceTargetAppShutdown(false)?;
        options.SetAllowUnsigned(false)?;
        // Since build 22556 this API accepts .appinstaller URIs. Target 22621 is
        // the first stable Windows 11 release with that support. Windows downloads,
        // validates trusted MSIX signatures and preserves App Installer association.
        let uri = Uri::CreateUri(&HSTRING::from(APPINSTALLER_URL))?;
        let result = context
            .manager
            .AddPackageByUriAsync(&uri, &options)?
            .join()?;
        let code = result.ExtendedErrorCode()?;
        ensure!(
            code.is_ok(),
            "Déploiement MSIX refusé ({:08X}) : {}",
            code.0 as u32,
            result.ErrorText()?
        );
        let phase = deployed_phase(result.IsRegistered()?);
        Ok(UpdateStatus::new(phase, Some(context.version.clone()), ""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_windows_version_distinguishes_supported_deferred_installer_uri() {
        let packed = |build: u64| ((10_u64 << 48) | (build << 16) | 1).to_string();
        assert_eq!(windows_build(&packed(22_000)).unwrap(), 22_000);
        assert!(windows_build(&packed(22_000)).unwrap() < MINIMUM_DEFERRED_BUILD);
        assert!(windows_build(&packed(22_621)).unwrap() >= MINIMUM_DEFERRED_BUILD);
        assert!(windows_build("invalid").is_err());
    }
    #[test]
    #[cfg(windows)]
    fn unavailable_results_cannot_report_up_to_date_or_start_deployment() {
        use windows::ApplicationModel::PackageUpdateAvailability as Availability;
        assert_eq!(
            availability_phase(Availability::NoUpdates),
            UpdatePhase::UpToDate
        );
        assert!(!staging_allowed(availability_phase(
            Availability::NoUpdates
        )));
        for availability in [
            Availability::Unknown,
            Availability::Error,
            Availability(999),
        ] {
            assert_eq!(availability_phase(availability), UpdatePhase::Error);
            assert!(!staging_allowed(availability_phase(availability)));
        }
        for availability in [Availability::Available, Availability::Required] {
            assert_eq!(availability_phase(availability), UpdatePhase::Available);
            assert!(staging_allowed(availability_phase(availability)));
        }
    }

    #[test]
    fn staging_requires_available_update_or_explicit_missing_association() {
        assert!(staging_allowed(UpdatePhase::Available));
        assert!(staging_allowed(UpdatePhase::Unassociated));
        for phase in [
            UpdatePhase::Checking,
            UpdatePhase::UpToDate,
            UpdatePhase::Preparing,
            UpdatePhase::ReadyOnRestart,
            UpdatePhase::Registered,
            UpdatePhase::Unsupported,
            UpdatePhase::Error,
        ] {
            assert!(!staging_allowed(phase));
        }
    }

    #[test]
    fn deployment_completion_preserves_pending_registration() {
        assert_eq!(deployed_phase(false), UpdatePhase::ReadyOnRestart);
        assert_eq!(deployed_phase(true), UpdatePhase::Registered);
        assert_ne!(deployed_phase(false), UpdatePhase::UpToDate);
        assert_ne!(deployed_phase(true), UpdatePhase::UpToDate);
    }
}
