//! Installation of this executable's embedded, pinned MSIX only.
//!
//! Loading this module does nothing. Preflight and payload verification never
//! deploy a package or modify trust. Installation belongs on a worker after an
//! explicit click; only the certificate helper is elevated, never deployment.

use super::{Phase, ViewState};
use anyhow::Result;

#[cfg(any(windows, test))]
use anyhow::{Context, ensure};
#[cfg(any(windows, test))]
use sha2::{Digest, Sha256};

#[cfg(any(windows, test))]
const MINIMUM_BUILD: u32 = 22_621;
#[cfg(any(windows, test))]
const MAX_PACKAGE_BYTES: usize = 128 * 1024 * 1024;
#[cfg(any(windows, test))]
const MAX_CERTIFICATE_BYTES: usize = 64 * 1024;

#[cfg(any(windows, test))]
fn pinned_digest(bytes: &[u8], expected: &str, maximum: usize) -> Result<[u8; 32]> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= maximum,
        "Payload embarqué absent ou trop volumineux"
    );
    ensure!(
        expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Empreinte embarquée SHA-256 invalide"
    );
    let mut pin = [0_u8; 32];
    for (index, value) in pin.iter_mut().enumerate() {
        *value = u8::from_str_radix(&expected[index * 2..index * 2 + 2], 16)
            .context("Empreinte embarquée SHA-256 invalide")?;
    }
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    ensure!(
        actual == pin,
        "Le payload embarqué ne correspond pas à son empreinte"
    );
    Ok(pin)
}

#[cfg(any(windows, test))]
fn supported_platform(build: u32, is_x64: bool) -> Result<()> {
    ensure!(
        build >= MINIMUM_BUILD,
        "Windows 11 22H2 ou plus récent est requis"
    );
    ensure!(is_x64, "Cet installateur nécessite Windows x64");
    Ok(())
}

#[cfg(any(windows, test))]
fn version_text(version: [u16; 4]) -> String {
    format!(
        "{}.{}.{}.{}",
        version[0], version[1], version[2], version[3]
    )
}

#[cfg(any(windows, test))]
fn prevent_downgrade(installed: Option<[u16; 4]>, target: [u16; 4]) -> Result<()> {
    ensure!(
        installed.is_none_or(|version| version <= target),
        "Une version plus récente est déjà installée. Cet installateur ne la remplace pas"
    );
    Ok(())
}

#[cfg(any(windows, test))]
fn deployment_outcome(
    error_code: i32,
    registered: bool,
    observed: Option<[u16; 4]>,
    target: [u16; 4],
) -> Result<Phase> {
    ensure!(
        error_code >= 0,
        "Installation Windows refusée (HRESULT {:08X})",
        error_code as u32
    );
    Ok(if registered && observed == Some(target) {
        Phase::Complete
    } else {
        Phase::Deferred
    })
}

#[cfg(any(windows, test))]
fn ocr_language_supported(tag: &str) -> bool {
    let primary = tag.split('-').next().unwrap_or_default();
    primary.eq_ignore_ascii_case("fr") || primary.eq_ignore_ascii_case("en")
}

/// Validate embedded hashes, public X.509 certificate and MSIX identity only.
/// Uses an owned temporary file; no trust-store write, deployment or GUI.
pub fn verify_payload() -> Result<()> {
    #[cfg(windows)]
    {
        platform::verify_payload()
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("La vérification de l'installateur nécessite Windows")
    }
}

/// Read-only Windows/package/trust checks. OCR absence is a nonblocking warning.
pub fn preflight() -> Result<ViewState> {
    #[cfg(windows)]
    {
        platform::preflight()
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("L'installateur nécessite Windows")
    }
}

/// Called only after an explicit installation click, on a background worker.
/// The boolean authorizes UAC for this executable's public certificate only.
pub fn install(
    approve_certificate: bool,
    progress: impl Fn(ViewState) + Send + 'static,
) -> Result<ViewState> {
    #[cfg(windows)]
    {
        platform::install(approve_certificate, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (approve_certificate, progress);
        anyhow::bail!("L'installation nécessite Windows")
    }
}

/// The dedicated `--approve-certificate` route: success exit 0, error exit 1.
/// Refuses additional arguments, impersonation and a nonelevated process.
pub fn approve_certificate_helper() -> Result<()> {
    #[cfg(windows)]
    {
        platform::approve_certificate_helper()
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("L'approbation du certificat nécessite Windows")
    }
}

/// Launch the installed application only on an explicit final-button click.
pub fn launch() -> Result<()> {
    #[cfg(windows)]
    {
        platform::launch()
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("Le lancement nécessite Windows")
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::installer::{
        CERTIFICATE_SHA256, EXPECTED_PACKAGE_VERSION, PACKAGE_NAME, PACKAGE_SHA256, PAYLOAD,
        PUBLIC_CERTIFICATE, PUBLISHER, VERSION,
    };
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use windows::{
        ApplicationModel::Package,
        Foundation::Uri,
        Management::Deployment::{
            AddPackageOptions, DeploymentProgress, DeploymentResult, PackageManager,
        },
        Media::Ocr::OcrEngine,
        System::Profile::AnalyticsInfo,
        Win32::{
            Foundation::{
                CRYPT_E_NOT_FOUND, CloseHandle, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND,
                ERROR_NO_TOKEN, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
            },
            Security::{
                Cryptography::{
                    CERT_CONTEXT, CERT_OPEN_STORE_FLAGS, CERT_QUERY_ENCODING_TYPE,
                    CERT_STORE_ADD_NEW, CERT_STORE_OPEN_EXISTING_FLAG, CERT_STORE_PROV_SYSTEM_W,
                    CERT_STORE_READONLY_FLAG, CERT_SYSTEM_STORE_LOCAL_MACHINE,
                    CertAddEncodedCertificateToStore, CertCloseStore, CertCreateCertificateContext,
                    CertEnumCertificatesInStore, CertFreeCertificateContext, CertOpenStore,
                    HCERTSTORE, X509_ASN_ENCODING,
                },
                GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
            },
            System::{
                Com::CoCreateGuid,
                SystemInformation::{
                    GetNativeSystemInfo, PROCESSOR_ARCHITECTURE_AMD64, SYSTEM_INFO,
                },
                Threading::{
                    GetCurrentProcess, GetCurrentThread, GetExitCodeProcess, OpenProcessToken,
                    OpenThreadToken, WaitForSingleObject,
                },
                WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
            },
            UI::{
                Shell::{
                    SEE_MASK_FLAG_NO_UI, SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC,
                    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
                },
                WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL},
            },
        },
        core::{Error, HRESULT, HSTRING, PCWSTR, w},
    };
    use windows_future::AsyncOperationProgressHandler;

    struct Apartment;
    impl Apartment {
        fn new() -> Result<Self> {
            // SAFETY: the worker owns this successful initialization and balances
            // it after all of its Windows objects and callbacks have been dropped.
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: paired with this thread's successful RoInitialize.
            unsafe { RoUninitialize() };
        }
    }

    struct Runtime {
        manager: PackageManager,
        _apartment: Apartment,
    }
    impl Runtime {
        fn new() -> Result<Self> {
            let apartment = Apartment::new()?;
            Ok(Self {
                manager: PackageManager::new()?,
                _apartment: apartment,
            })
        }
    }

    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: only owned, real process/token handles enter this wrapper.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn no_impersonation() -> Result<()> {
        let mut token = HANDLE::default();
        // SAFETY: current-thread pseudo handle is not closed. Any actual token
        // returned is owned; no token is installed, reverted or impersonated.
        match unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) } {
            Ok(()) => {
                let _token = OwnedHandle(token);
                anyhow::bail!("L'installateur refuse un thread sous une autre identité")
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_TOKEN.0) => Ok(()),
            Err(_) => anyhow::bail!("Impossible de vérifier l'identité du thread"),
        }
    }

    fn elevated_process() -> Result<bool> {
        no_impersonation()?;
        let mut token = HANDLE::default();
        // SAFETY: TOKEN_QUERY retrieves an owned token for this process only.
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }?;
        let token = OwnedHandle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        // SAFETY: buffer and declared length match TOKEN_ELEVATION exactly.
        unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        }?;
        ensure!(
            returned as usize == size_of::<TOKEN_ELEVATION>(),
            "Identité du processus indisponible"
        );
        Ok(elevation.TokenIsElevated != 0)
    }

    fn ordinary_user() -> Result<()> {
        ensure!(
            !elevated_process()?,
            "Ouvrez l'installateur normalement, sans « Exécuter en tant qu'administrateur »"
        );
        Ok(())
    }

    fn check_platform() -> Result<()> {
        let packed: u64 = AnalyticsInfo::VersionInfo()?
            .DeviceFamilyVersion()?
            .to_string()
            .parse()
            .context("Version Windows indisponible")?;
        let build = ((packed >> 16) & 0xffff) as u32;
        let mut system = SYSTEM_INFO::default();
        // SAFETY: API fills the complete native SYSTEM_INFO structure. This reads
        // architecture only, without depending on environment variables.
        unsafe { GetNativeSystemInfo(&mut system) };
        let native_x64 = unsafe {
            system.Anonymous.Anonymous.wProcessorArchitecture == PROCESSOR_ARCHITECTURE_AMD64
        };
        supported_platform(build, native_x64 && cfg!(target_arch = "x86_64"))
    }

    struct Certificate(*mut CERT_CONTEXT);
    impl Certificate {
        fn embedded() -> Result<Self> {
            pinned_digest(
                PUBLIC_CERTIFICATE,
                CERTIFICATE_SHA256,
                MAX_CERTIFICATE_BYTES,
            )?;
            // SAFETY: the pinned, bounded slice remains valid through decoding;
            // this creates a memory-only context, never a certificate-store entry.
            let context =
                unsafe { CertCreateCertificateContext(X509_ASN_ENCODING, PUBLIC_CERTIFICATE) };
            ensure!(
                !context.is_null(),
                "Le certificat public embarqué n'est pas un certificat X.509 valide"
            );
            let certificate = Self(context);
            ensure!(
                certificate.bytes()? == PUBLIC_CERTIFICATE,
                "Le certificat embarqué doit contenir un seul certificat public DER"
            );
            Ok(certificate)
        }

        fn bytes(&self) -> Result<&[u8]> {
            ensure!(!self.0.is_null(), "Certificat absent");
            // SAFETY: the context belongs to this owner and remains live until
            // drop. Windows owns its encoded buffer for exactly cbCertEncoded.
            let value = unsafe { &*self.0 };
            ensure!(
                !value.pbCertEncoded.is_null() && value.cbCertEncoded != 0,
                "Certificat invalide"
            );
            Ok(unsafe {
                std::slice::from_raw_parts(value.pbCertEncoded, value.cbCertEncoded as usize)
            })
        }
    }
    impl Drop for Certificate {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: releases the context still owned here; enumeration
                // transfers its previous context before calling the next item.
                unsafe {
                    let _ = CertFreeCertificateContext(Some(self.0));
                }
            }
        }
    }

    struct Store(HCERTSTORE);
    impl Store {
        fn open(readonly: bool) -> windows::core::Result<Self> {
            let mut flags = CERT_OPEN_STORE_FLAGS(CERT_SYSTEM_STORE_LOCAL_MACHINE);
            if readonly {
                flags |= CERT_STORE_OPEN_EXISTING_FLAG | CERT_STORE_READONLY_FLAG;
            }
            // SAFETY: provider is a predefined system provider, store name is a
            // static NUL-terminated string, and location is strictly local machine.
            unsafe {
                CertOpenStore(
                    CERT_STORE_PROV_SYSTEM_W,
                    CERT_QUERY_ENCODING_TYPE(0),
                    None,
                    flags,
                    Some(w!("TrustedPeople").as_ptr().cast()),
                )
            }
            .map(Self)
        }
    }
    impl Drop for Store {
        fn drop(&mut self) {
            // SAFETY: all contexts created within store operations are dropped
            // before this owned store; no force-close or store deletion is used.
            unsafe {
                let _ = CertCloseStore(Some(self.0), 0);
            }
        }
    }

    fn certificate_trusted() -> Result<bool> {
        let pin = pinned_digest(
            PUBLIC_CERTIFICATE,
            CERTIFICATE_SHA256,
            MAX_CERTIFICATE_BYTES,
        )?;
        let store = match Store::open(true) {
            Ok(store) => store,
            Err(error) if error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => {
                return Ok(false);
            }
            Err(_) => anyhow::bail!("Impossible de lire LocalMachine\\TrustedPeople"),
        };
        let mut certificate = Certificate(std::ptr::null_mut());
        loop {
            let previous = std::mem::replace(&mut certificate.0, std::ptr::null_mut());
            // SAFETY: CertEnum frees the previous enumeration context itself.
            // The returned context is owned by certificate for all early returns.
            certificate.0 = unsafe {
                CertEnumCertificatesInStore(
                    store.0,
                    (!previous.is_null()).then_some(previous.cast_const()),
                )
            };
            if certificate.0.is_null() {
                let error = Error::from_thread();
                ensure!(
                    error.code() == CRYPT_E_NOT_FOUND,
                    "Lecture du magasin de certificats interrompue"
                );
                return Ok(false);
            }
            let bytes = certificate.bytes()?;
            if bytes.len() <= MAX_CERTIFICATE_BYTES {
                let hash: [u8; 32] = Sha256::digest(bytes).into();
                if hash == pin && bytes == PUBLIC_CERTIFICATE {
                    return Ok(true);
                }
            }
        }
    }

    /// Owns precisely one uniquely created directory and one package filename.
    /// Cleanup is deliberately nonrecursive and cannot remove unexpected files.
    struct ScratchPackage {
        directory: PathBuf,
        path: PathBuf,
        handle: Option<File>,
    }
    impl ScratchPackage {
        fn extract() -> Result<Self> {
            pinned_digest(PAYLOAD, PACKAGE_SHA256, MAX_PACKAGE_BYTES)?;
            let parent = fs::canonicalize(std::env::temp_dir())
                .context("Dossier temporaire Windows indisponible")?;
            ensure!(parent.is_absolute(), "Dossier temporaire invalide");
            // SAFETY: asks Windows for a fresh GUID, without activating any UI.
            let unique = unsafe { CoCreateGuid() }?;
            let directory = parent.join(format!("MayhemLens-setup-{:032x}", unique.to_u128()));
            ensure!(
                directory.parent() == Some(parent.as_path()),
                "Dossier temporaire invalide"
            );
            fs::create_dir(&directory).context("Création du dossier temporaire unique")?;
            let mut owned = Self {
                path: directory.join("MayhemLens.msix"),
                directory,
                handle: None,
            };
            owned.handle = Some(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .share_mode(0)
                    .open(&owned.path)
                    .context("Création exclusive du MSIX embarqué")?,
            );
            let file = owned.handle.as_mut().context("Fichier temporaire absent")?;
            file.write_all(PAYLOAD)
                .context("Extraction du MSIX embarqué")?;
            file.sync_all().context("Finalisation du MSIX embarqué")?;
            drop(owned.handle.take());
            owned.handle = Some(
                OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .open(&owned.path)
                    .context("Verrouillage en lecture du MSIX embarqué")?,
            );
            let mut actual = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            let mut total = 0_usize;
            let file = owned.handle.as_mut().context("Fichier temporaire absent")?;
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                total = total
                    .checked_add(count)
                    .context("Taille du MSIX invalide")?;
                ensure!(total <= PAYLOAD.len(), "Le MSIX extrait a été modifié");
                actual.update(&buffer[..count]);
            }
            let expected: [u8; 32] = Sha256::digest(PAYLOAD).into();
            let actual: [u8; 32] = actual.finalize().into();
            ensure!(
                total == PAYLOAD.len() && actual == expected,
                "Le MSIX extrait ne correspond pas au payload embarqué"
            );
            // The deny-write/delete handle closes the extraction/reopen race and
            // remains held until every PackageReader/deployment operation ends.
            crate::update_package::validate(
                &owned.path,
                PACKAGE_NAME,
                PUBLISHER,
                EXPECTED_PACKAGE_VERSION,
            )?;
            Ok(owned)
        }
    }
    impl Drop for ScratchPackage {
        fn drop(&mut self) {
            drop(self.handle.take());
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir(&self.directory);
        }
    }

    fn verify_in_apartment() -> Result<()> {
        let _certificate = Certificate::embedded()?;
        let _package = ScratchPackage::extract()?;
        Ok(())
    }

    pub(super) fn verify_payload() -> Result<()> {
        let _apartment = Apartment::new()?;
        verify_in_apartment()
    }

    struct Installed {
        package: Package,
        version: [u16; 4],
    }

    fn installed(manager: &PackageManager) -> Result<Option<Installed>> {
        // Empty SID explicitly selects the calling user. Never enumerate other
        // users, launch under an admin credential, or persist a SID/profile.
        let packages = manager.FindPackagesByUserSecurityIdNamePublisher(
            &HSTRING::new(),
            &HSTRING::from(PACKAGE_NAME),
            &HSTRING::from(PUBLISHER),
        )?;
        let mut newest: Option<Installed> = None;
        for package in packages {
            if package.IsResourcePackage()? || package.IsFramework()? {
                continue;
            }
            let id = package.Id()?;
            ensure!(
                id.Name()? == PACKAGE_NAME && id.Publisher()? == PUBLISHER,
                "Identité de l'application installée inattendue"
            );
            let value = id.Version()?;
            let version = [value.Major, value.Minor, value.Build, value.Revision];
            if newest
                .as_ref()
                .is_none_or(|previous| version > previous.version)
            {
                newest = Some(Installed { package, version });
            }
        }
        Ok(newest)
    }

    fn inspect(runtime: &Runtime) -> Result<ViewState> {
        ordinary_user()?;
        check_platform()?;
        verify_in_apartment()?;
        let installed = installed(&runtime.manager)?;
        prevent_downgrade(
            installed.as_ref().map(|package| package.version),
            EXPECTED_PACKAGE_VERSION,
        )?;
        let trusted = certificate_trusted()?;
        let ocr = OcrEngine::AvailableRecognizerLanguages().and_then(|languages| {
            for language in languages {
                if ocr_language_supported(&language.LanguageTag()?.to_string()) {
                    return Ok(true);
                }
            }
            Ok(false)
        });
        let ocr_available = matches!(ocr, Ok(true));
        let mut detail =
            format!("Éditeur : Alexandre DO-O ALMEIDA. Version {VERSION}, Windows x64 compatible.");
        if !trusted {
            detail.push_str(" Ce certificat public de développement demande ton accord et une autorisation Windows.");
        }
        match ocr {
            Ok(false) => detail.push_str(
                " OCR FR/EN à ajouter dans les paramètres Windows avant d'utiliser l'overlay.",
            ),
            Err(_) => detail.push_str(
                " OCR FR/EN non vérifié : contrôle les langues OCR dans les paramètres Windows.",
            ),
            Ok(true) => {}
        }
        Ok(ViewState {
            phase: Phase::Ready,
            progress: 0,
            status: "Prêt à installer Mayhem Lens".into(),
            detail,
            certificate_trusted: trusted,
            ocr_available,
            installed_version: installed
                .as_ref()
                .map(|package| version_text(package.version)),
        })
    }

    pub(super) fn preflight() -> Result<ViewState> {
        let runtime = Runtime::new()?;
        inspect(&runtime)
    }

    fn request_certificate_approval() -> Result<()> {
        let executable =
            std::env::current_exe().context("Chemin de l'installateur indisponible")?;
        // Hold this exact running executable against replacement throughout UAC
        // and helper completion; no external executable or payload path is used.
        let _executable_lock = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&executable)
            .context("Verrouillage de l'installateur pour l'autorisation")?;
        let mut filename: Vec<u16> = executable.as_os_str().encode_wide().collect();
        ensure!(!filename.contains(&0), "Chemin de l'installateur invalide");
        filename.push(0);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(filename.as_ptr()),
            lpParameters: w!("--approve-certificate"),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: all UTF-16 inputs are static or kept alive through the call;
        // runas requests Windows consent, with no additional executable argument.
        if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                anyhow::bail!("Autorisation Windows refusée. Aucun package n'a été installé");
            }
            anyhow::bail!(
                "Impossible de demander l'autorisation Windows (HRESULT {:08X})",
                error.code().0 as u32
            );
        }
        ensure!(
            !info.hProcess.is_invalid(),
            "Confirmation de l'autorisation indisponible"
        );
        let helper = OwnedHandle(info.hProcess);
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            // SAFETY: owned child-process handle remains valid. Short waits run
            // only on the worker; timeout never terminates the helper or any app.
            let outcome = unsafe { WaitForSingleObject(helper.0, 250) };
            if outcome == WAIT_OBJECT_0 {
                break;
            }
            ensure!(
                outcome == WAIT_TIMEOUT,
                "Attente de l'autorisation Windows interrompue"
            );
            ensure!(
                Instant::now() < deadline,
                "Autorisation non confirmée dans le délai prévu. Aucun package n'a été installé"
            );
        }
        let mut exit = 1;
        // SAFETY: the helper has exited and the writable exit-code buffer lives.
        unsafe { GetExitCodeProcess(helper.0, &mut exit) }?;
        ensure!(
            exit == 0,
            "L'approbation du certificat n'a pas abouti. Aucun package n'a été installé"
        );
        ensure!(
            certificate_trusted()?,
            "Le certificat public attendu n'est pas approuvé"
        );
        Ok(())
    }

    pub(super) fn approve_certificate_helper() -> Result<()> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        ensure!(
            args.len() == 1 && args[0] == "--approve-certificate",
            "Arguments du helper de certificat invalides"
        );
        ensure!(
            elevated_process()?,
            "L'approbation du certificat nécessite l'autorisation administrateur Windows"
        );
        let _certificate = Certificate::embedded()?;
        if certificate_trusted()? {
            return Ok(());
        }
        let store = Store::open(false)
            .context("Ouverture de LocalMachine\\TrustedPeople pour approbation")?;
        // SAFETY: only our hash-pinned public X.509 DER is accepted. ADD_NEW does
        // not replace an existing certificate or attach any private key/profile.
        let added = unsafe {
            CertAddEncodedCertificateToStore(
                Some(store.0),
                X509_ASN_ENCODING,
                PUBLIC_CERTIFICATE,
                CERT_STORE_ADD_NEW,
                None,
            )
        };
        // Close/commit before a separate read-only view verifies the exact pin.
        drop(store);
        let trusted = certificate_trusted()?;
        if let Err(error) = added {
            ensure!(
                trusted,
                "Approbation du certificat refusée (HRESULT {:08X})",
                error.code().0 as u32
            );
        }
        ensure!(
            trusted,
            "L'empreinte du certificat approuvé ne correspond pas au certificat embarqué"
        );
        Ok(())
    }

    struct Reporter<P> {
        callback: P,
        state: ViewState,
    }
    impl<P: Fn(ViewState)> Reporter<P> {
        fn emit(&mut self, state: ViewState) {
            self.state = state.clone();
            (self.callback)(state);
        }
        fn deployment_progress(&mut self, percentage: u32) {
            if self.state.phase != Phase::Installing {
                return;
            }
            let progress = 30 + (percentage.min(100) * 65 / 100) as u8;
            if progress <= self.state.progress {
                return;
            }
            let mut state = self.state.clone();
            state.progress = progress;
            self.emit(state);
        }
    }

    fn report<P: Fn(ViewState)>(reporter: &Mutex<Reporter<P>>, state: ViewState) -> Result<()> {
        reporter
            .lock()
            .map_err(|_| anyhow::anyhow!("État de l'installation indisponible"))?
            .emit(state);
        Ok(())
    }

    fn install_inner<P: Fn(ViewState) + Send + 'static>(
        approve: bool,
        reporter: &Arc<Mutex<Reporter<P>>>,
    ) -> Result<ViewState> {
        let runtime = Runtime::new()?;
        let mut state = inspect(&runtime)?;
        state.phase = Phase::Checking;
        state.progress = 15;
        state.status = "Package et prérequis vérifiés".into();
        report(reporter, state.clone())?;
        let before = installed(&runtime.manager)?;
        prevent_downgrade(
            before.as_ref().map(|package| package.version),
            EXPECTED_PACKAGE_VERSION,
        )?;
        if let Some(package) = &before
            && package.version == EXPECTED_PACKAGE_VERSION
            && package.package.Status()?.VerifyIsOK()?
        {
            state.phase = Phase::Complete;
            state.progress = 100;
            state.status = "Mayhem Lens est déjà installé".into();
            state.detail = "La version attendue est inscrite pour cet utilisateur. Vous pouvez lancer l'application.".into();
            return Ok(state);
        }
        if !state.certificate_trusted {
            ensure!(
                approve,
                "Cochez l'accord pour approuver le certificat public de développement avant de continuer"
            );
            state.phase = Phase::Authorizing;
            state.status = "Autorisation Windows pour le certificat…".into();
            state.detail = "Seul le certificat public embarqué sera ajouté à LocalMachine\\TrustedPeople. Le package sera installé avec votre compte habituel.".into();
            report(reporter, state.clone())?;
            request_certificate_approval()?;
            state.certificate_trusted = certificate_trusted()?;
            ensure!(
                state.certificate_trusted,
                "L'approbation du certificat attendu n'est pas confirmée"
            );
        }
        // An app/update may have changed since preflight or while UAC was open.
        ordinary_user()?;
        let current = installed(&runtime.manager)?;
        prevent_downgrade(
            current.as_ref().map(|package| package.version),
            EXPECTED_PACKAGE_VERSION,
        )?;
        let package = ScratchPackage::extract()?;
        state.phase = Phase::Installing;
        state.progress = 30;
        state.status = "Installation de Mayhem Lens…".into();
        state.detail =
            "Windows vérifie la signature. Aucun arrêt forcé de l'overlay ou du jeu n'est demandé."
                .into();
        report(reporter, state.clone())?;
        let options = AddPackageOptions::new()?;
        options.SetAllowUnsigned(false)?;
        options.SetDeferRegistrationWhenPackagesAreInUse(true)?;
        options.SetForceAppShutdown(false)?;
        options.SetForceTargetAppShutdown(false)?;
        options.SetForceUpdateFromAnyVersion(false)?;
        let url = reqwest::Url::from_file_path(&package.path)
            .map_err(|_| anyhow::anyhow!("Chemin du package embarqué invalide"))?;
        let uri = Uri::CreateUri(&HSTRING::from(url.as_str()))?;
        // Source archive remains read-locked until the async operation and its
        // result have completed. No settings, cache or game process are touched.
        let operation = runtime.manager.AddPackageByUriAsync(&uri, &options)?;
        let events = Arc::clone(reporter);
        let handler = AsyncOperationProgressHandler::<DeploymentResult, DeploymentProgress>::new(
            move |_, value| {
                if let Ok(mut reporter) = events.lock() {
                    reporter.deployment_progress(value.percentage);
                }
                Ok(())
            },
        );
        // A missing progress subscription cannot abort an already-started
        // deployment and release its source archive before completion.
        let _ = operation.SetProgress(&handler);
        let result = operation.join().context("Installation native du package")?;
        let code = result.ExtendedErrorCode()?;
        let registered = if code.is_ok() {
            result.IsRegistered()?
        } else {
            false
        };
        // Do not retain untrusted OS ErrorText (paths/profile); expose HRESULT.
        deployment_outcome(code.0, false, None, EXPECTED_PACKAGE_VERSION)?;
        let after = installed(&runtime.manager)?;
        let healthy = match &after {
            Some(package) => package.package.Status()?.VerifyIsOK()?,
            None => false,
        };
        state.installed_version = after.as_ref().map(|package| version_text(package.version));
        state.phase = deployment_outcome(
            code.0,
            registered && healthy,
            after.as_ref().map(|package| package.version),
            EXPECTED_PACKAGE_VERSION,
        )?;
        state.progress = 100;
        if state.phase == Phase::Complete {
            state.status = "Mayhem Lens est installé".into();
            state.detail = "Version inscrite et vérifiée pour votre compte. L'application se lancera uniquement si vous cliquez sur Lancer.".into();
        } else {
            state.status = "Mise à jour préparée".into();
            state.detail = "Windows a préparé le package. Fermez Mayhem Lens lorsque vous avez terminé, puis relancez-le pour appliquer la mise à jour.".into();
        }
        Ok(state)
    }

    pub(super) fn install(
        approve: bool,
        progress: impl Fn(ViewState) + Send + 'static,
    ) -> Result<ViewState> {
        let initial = ViewState {
            status: "Vérification du package embarqué…".into(),
            ..Default::default()
        };
        let reporter = Arc::new(Mutex::new(Reporter {
            callback: progress,
            state: initial.clone(),
        }));
        report(&reporter, initial)?;
        match install_inner(approve, &reporter) {
            Ok(state) => {
                report(&reporter, state.clone())?;
                Ok(state)
            }
            Err(error) => {
                if let Ok(mut reporter) = reporter.lock() {
                    let mut failed = reporter.state.clone();
                    failed.phase = Phase::Failed;
                    failed.status = "Installation interrompue".into();
                    failed.detail = format!("{error:#}");
                    reporter.emit(failed);
                }
                Err(error)
            }
        }
    }

    pub(super) fn launch() -> Result<()> {
        ordinary_user()?;
        let runtime = Runtime::new()?;
        let package = installed(&runtime.manager)?
            .context("Mayhem Lens n'est pas installé pour votre compte")?;
        ensure!(
            package.version >= EXPECTED_PACKAGE_VERSION
                && package.package.Status()?.VerifyIsOK()?,
            "La version installée n'est pas prête à être lancée"
        );
        let family = package.package.Id()?.FamilyName()?.to_string();
        ensure!(
            family.starts_with(&format!("{PACKAGE_NAME}_"))
                && family
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
            "Famille de package inattendue"
        );
        let target = HSTRING::from(format!("shell:AppsFolder\\{family}!Lens"));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: w!("open"),
            lpFile: PCWSTR(target.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: namespace target is built from the exact registered package's
        // validated family and our fixed application ID; no arbitrary arguments.
        unsafe { ShellExecuteExW(&mut info) }
            .context("Lancement de Mayhem Lens depuis AppsFolder")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_payload_rejects_corruption_missing_data_and_ambiguous_pins() {
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(pinned_digest(b"abc", digest, 3).is_ok());
        assert!(pinned_digest(b"abc", &digest.to_ascii_uppercase(), 3).is_ok());
        for (bytes, expected, maximum) in [
            (&b"abd"[..], digest, 3),
            (&b""[..], digest, 3),
            (&b"abcd"[..], digest, 3),
            (&b"abc"[..], "", 3),
            (&b"abc"[..], "sha256:abcd", 3),
            (&b"abc"[..], digest, 2),
        ] {
            assert!(pinned_digest(bytes, expected, maximum).is_err());
        }
        assert!(pinned_digest(b"abc", &"g".repeat(64), 3).is_err());
        assert!(pinned_digest(b"abc", &"é".repeat(32), 3).is_err());
    }

    #[test]
    fn platform_and_version_gates_do_not_allow_downgrades() {
        assert!(supported_platform(22_621, true).is_ok());
        assert!(supported_platform(22_000, true).is_err());
        assert!(supported_platform(26_200, false).is_err());
        let target = [1, 1, 1, 0];
        for installed in [None, Some([1, 1, 0, 0]), Some(target)] {
            assert!(prevent_downgrade(installed, target).is_ok());
        }
        for installed in [[1, 1, 1, 1], [1, 1, 2, 0], [2, 0, 0, 0]] {
            assert!(prevent_downgrade(Some(installed), target).is_err());
        }
        assert_eq!(version_text([1, 2, 3, 4]), "1.2.3.4");
    }

    #[test]
    fn completion_requires_success_registration_and_exact_reinspected_version() {
        let target = [1, 1, 1, 0];
        assert_eq!(
            deployment_outcome(0, true, Some(target), target).unwrap(),
            Phase::Complete
        );
        for (registered, observed) in [
            (false, Some(target)),
            (true, None),
            (true, Some([1, 1, 0, 0])),
            (true, Some([1, 1, 2, 0])),
        ] {
            assert_eq!(
                deployment_outcome(0, registered, observed, target).unwrap(),
                Phase::Deferred
            );
        }
        for code in [0x800B0004_u32 as i32, 0x80073CF0_u32 as i32, -1] {
            assert!(deployment_outcome(code, true, Some(target), target).is_err());
            assert!(deployment_outcome(code, false, None, target).is_err());
        }
    }

    #[test]
    fn only_french_or_english_ocr_satisfies_the_nonblocking_check() {
        for tag in ["fr", "fr-FR", "FR-CA", "en", "en-US", "en-GB"] {
            assert!(ocr_language_supported(tag));
        }
        for tag in ["", "de-DE", "english", "french", "enough", "france"] {
            assert!(!ocr_language_supported(tag));
        }
    }
}
