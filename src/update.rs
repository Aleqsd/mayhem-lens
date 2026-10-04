//! Native MSIX updates. Loading this module performs no network or deployment.
//! The controller runs both launch and manual requests on one background worker.

use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const RELEASES_URL: &str = "https://github.com/Aleqsd/mayhem-lens/releases/latest";
const RELEASE_API_URL: &str = "https://api.github.com/repos/Aleqsd/mayhem-lens/releases/latest";
const PACKAGE_NAME: &str = "Aleqsd.MayhemLens";
const PACKAGE_PUBLISHER: &str = "CN=Alexandre DO-O ALMEIDA";
const MINIMUM_DEFERRED_BUILD: u32 = 22_621;
const MAX_METADATA_SIZE: u64 = 1024 * 1024;
const MAX_PACKAGE_SIZE: u64 = 128 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RECEIPT_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Checking,
    UpToDate,
    Available,
    Downloading,
    Verifying,
    Preparing,
    ReadyOnRestart,
    Registered,
    Updated,
    Unassociated,
    Unsupported,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub phase: UpdatePhase,
    /// Observed package identity; never substitute the offered target version.
    pub installed_version: Option<String>,
    #[serde(default)]
    pub active_version: Option<String>,
    #[serde(default)]
    pub target_version: Option<String>,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub confirmed_version: Option<String>,
    #[serde(default)]
    pub confirmed_at_unix: Option<u64>,
    #[serde(default)]
    pub check_error: Option<String>,
    pub checked_at_unix: u64,
    pub detail: String,
}

impl UpdateStatus {
    fn new(phase: UpdatePhase, version: Option<String>, detail: impl Into<String>) -> Self {
        Self {
            phase,
            active_version: version.clone(),
            installed_version: version,
            target_version: None,
            downloaded_bytes: 0,
            total_bytes: None,
            confirmed_version: None,
            confirmed_at_unix: None,
            check_error: None,
            checked_at_unix: unix_seconds(),
            detail: detail.into(),
        }
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking
                | UpdatePhase::Downloading
                | UpdatePhase::Verifying
                | UpdatePhase::Preparing
        )
    }

    pub fn summary(&self, language: &str) -> &'static str {
        match (self.phase, language == "en") {
            (UpdatePhase::Checking, false) => "Recherche de mise à jour…",
            (UpdatePhase::Checking, true) => "Checking for updates…",
            (UpdatePhase::UpToDate, false) => "Application à jour",
            (UpdatePhase::UpToDate, true) => "Application is up to date",
            (UpdatePhase::Available, false) => "Mise à jour disponible",
            (UpdatePhase::Available, true) => "Update available",
            (UpdatePhase::Downloading, false) => "Téléchargement de la mise à jour…",
            (UpdatePhase::Downloading, true) => "Downloading update…",
            (UpdatePhase::Verifying, false) => "Vérification du package…",
            (UpdatePhase::Verifying, true) => "Verifying package…",
            (UpdatePhase::Preparing, false) => "Préparation de la mise à jour…",
            (UpdatePhase::Preparing, true) => "Preparing update…",
            (UpdatePhase::ReadyOnRestart, false) => "Mise à jour préparée — prochain lancement",
            (UpdatePhase::ReadyOnRestart, true) => "Update prepared — next launch",
            (UpdatePhase::Registered, false) => "Mise à jour enregistrée — relancer l’overlay",
            (UpdatePhase::Registered, true) => "Update registered — relaunch the overlay",
            (UpdatePhase::Updated, false) => "Mise à jour appliquée — version active confirmée",
            (UpdatePhase::Updated, true) => "Update applied — active version confirmed",
            (UpdatePhase::Unassociated, false) => "Canal de mise à jour non associé",
            (UpdatePhase::Unassociated, true) => "Update channel is not associated",
            (UpdatePhase::Unsupported, false) => "Mise à jour différée : Windows 11 22H2 requis",
            (UpdatePhase::Unsupported, true) => "Deferred updates require Windows 11 22H2",
            (UpdatePhase::Error, false) => "Mise à jour indisponible — réessayer",
            (UpdatePhase::Error, true) => "Update unavailable — retry",
        }
    }

    /// Local presentation only: no network, filesystem or Windows API calls.
    pub fn display_lines(&self, language: &str) -> Vec<String> {
        let english = language == "en";
        let mut lines = vec![self.summary(language).into()];
        if let Some(version) = self
            .active_version
            .as_ref()
            .or(self.installed_version.as_ref())
        {
            lines.push(format!(
                "{} : {version}",
                if english {
                    "Active version"
                } else {
                    "Version active"
                }
            ));
        }
        if let Some(version) = &self.installed_version
            && self
                .active_version
                .as_ref()
                .is_some_and(|active| active != version)
        {
            lines.push(format!(
                "{} : {version}",
                if english {
                    "Installed version"
                } else {
                    "Version installée"
                }
            ));
        }
        if let Some(version) = &self.target_version {
            lines.push(format!(
                "{} : {version}",
                if english {
                    "Target version"
                } else {
                    "Version cible"
                }
            ));
        }
        if let Some(total) = self.total_bytes.filter(|total| *total > 0) {
            let downloaded = self.downloaded_bytes.min(total);
            let percent = u128::from(downloaded) * 100 / u128::from(total);
            lines.push(format!(
                "{} : {:.1} / {:.1} {} ({percent} %)",
                if english {
                    "Download"
                } else {
                    "Téléchargement"
                },
                downloaded as f64 / 1_048_576.0,
                total as f64 / 1_048_576.0,
                if english { "MiB" } else { "Mio" }
            ));
        }
        if let Some(version) = &self.confirmed_version {
            lines.push(format!(
                "{} : {version}",
                if english {
                    "Update applied at launch"
                } else {
                    "Mise à jour confirmée au lancement"
                }
            ));
        }
        if let Some(error) = &self.check_error {
            lines.push(format!(
                "{} : {error}",
                if english {
                    "Last check unavailable"
                } else {
                    "Dernière vérification indisponible"
                }
            ));
        } else if self.phase == UpdatePhase::Error && !self.detail.is_empty() {
            lines.push(self.detail.clone());
        }
        lines
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Clones share one bounded request queue and one state. Dropping the last
/// controller disconnects the worker; shutdown never joins a network operation.
#[derive(Clone)]
pub struct UpdateController {
    request: SyncSender<UpdateRequest>,
    state: Arc<Mutex<UpdateStatus>>,
}

#[derive(Clone, Copy)]
enum UpdateRequest {
    Check,
    Prepare,
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
                // an execution alias, with no App Installer association required.
                run_request(&worker_state, UpdateRequest::Prepare);
                while let Ok(request) = requests.recv() {
                    run_request(&worker_state, request);
                }
            })
            .context("Création du worker de mise à jour")?;
        Ok(Self { request, state })
    }

    pub fn request_update(&self) -> bool {
        !self.status().busy() && self.request.try_send(UpdateRequest::Prepare).is_ok()
    }

    /// Check release metadata on the existing worker, without downloading MSIX.
    pub fn request_check(&self) -> bool {
        !self.status().busy() && self.request.try_send(UpdateRequest::Check).is_ok()
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingReceipt {
    package_name: String,
    publisher: String,
    from_version: [u16; 4],
    target_version: [u16; 4],
    prepared_phase: UpdatePhase,
    prepared_at_unix: u64,
    #[serde(default)]
    confirmed_at_unix: Option<u64>,
}

impl PendingReceipt {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.package_name == PACKAGE_NAME && self.publisher == PACKAGE_PUBLISHER,
            "Identité du reçu de mise à jour invalide"
        );
        ensure!(
            self.target_version > self.from_version
                && matches!(
                    self.prepared_phase,
                    UpdatePhase::ReadyOnRestart | UpdatePhase::Registered
                ),
            "Reçu de mise à jour invalide"
        );
        Ok(())
    }
}

fn receipt_path() -> PathBuf {
    crate::config::app_directory().join("update-pending.json")
}

fn load_receipt() -> Result<Option<PendingReceipt>> {
    let mut file = match File::open(receipt_path()) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Lecture du reçu de mise à jour"),
    };
    let mut data = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_RECEIPT_SIZE + 1)
        .read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= MAX_RECEIPT_SIZE,
        "Reçu de mise à jour trop volumineux"
    );
    let receipt: PendingReceipt =
        serde_json::from_slice(&data).context("Reçu de mise à jour invalide")?;
    receipt.validate()?;
    Ok(Some(receipt))
}

fn save_receipt(receipt: &PendingReceipt) -> Result<()> {
    receipt.validate()?;
    let path = receipt_path();
    fs::create_dir_all(path.parent().context("Dossier du reçu absent")?)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(receipt)?)?;
    file.sync_all()
        .context("Persistance du reçu de mise à jour")?;
    drop(file);
    fs::rename(temporary, path).context("Enregistrement du reçu de mise à jour")
}

fn reconcile_receipt(active: [u16; 4], receipt: Option<&PendingReceipt>, now: u64) -> UpdateStatus {
    let mut status = UpdateStatus::new(UpdatePhase::Checking, Some(version_string(active)), "");
    if let Some(receipt) = receipt {
        if active == receipt.target_version {
            status.phase = UpdatePhase::Updated;
            status.target_version = Some(version_string(receipt.target_version));
            status.confirmed_version = status.target_version.clone();
            status.confirmed_at_unix = Some(receipt.confirmed_at_unix.unwrap_or(now));
        } else if active < receipt.target_version {
            // A completed Windows operation is still pending for this process.
            status.phase = receipt.prepared_phase;
            status.target_version = Some(version_string(receipt.target_version));
        }
    }
    status
}

fn persist_confirmation(
    mut status: UpdateStatus,
    receipt: Option<&PendingReceipt>,
    persist: impl FnOnce(&PendingReceipt) -> Result<()>,
) -> UpdateStatus {
    if status.phase == UpdatePhase::Updated
        && let Some(receipt) = receipt
        && receipt.confirmed_at_unix.is_none()
    {
        let confirmed = PendingReceipt {
            confirmed_at_unix: status.confirmed_at_unix,
            ..receipt.clone()
        };
        if let Err(error) = persist(&confirmed) {
            // The durable pending receipt and active identity already prove the
            // update. Failure to persist its timestamp cannot invalidate that.
            let detail = format!("Persistance du reçu de confirmation : {error:#}");
            status.detail.clone_from(&detail);
            status.check_error = Some(detail);
        }
    }
    status
}

fn failed_status(mut status: UpdateStatus, error: String) -> UpdateStatus {
    // A failed release check cannot erase a prior preparation or a confirmation
    // established from the current process identity and durable receipt.
    if !(matches!(
        status.phase,
        UpdatePhase::Updated | UpdatePhase::ReadyOnRestart | UpdatePhase::Registered
    ) || status.phase == UpdatePhase::Checking && status.confirmed_version.is_some())
    {
        status.phase = UpdatePhase::Error;
    } else if status.phase == UpdatePhase::Checking {
        status.phase = UpdatePhase::Updated;
    }
    status.detail.clone_from(&error);
    status.check_error = Some(error);
    status.checked_at_unix = unix_seconds();
    status
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

fn run_request(state: &Mutex<UpdateStatus>, request: UpdateRequest) {
    let mut checking = state
        .lock()
        .map(|status| status.clone())
        .unwrap_or_else(|_| UpdateStatus::new(UpdatePhase::Checking, None, ""));
    checking.phase = UpdatePhase::Checking;
    checking.check_error = None;
    checking.detail.clear();
    checking.downloaded_bytes = 0;
    checking.total_bytes = None;
    publish(state, checking);
    let status = match request {
        UpdateRequest::Prepare => check_and_stage_with(|status| publish(state, status)),
        UpdateRequest::Check => check(),
    };
    match status {
        Ok(status) => publish(state, status),
        Err(error) => {
            let previous = state
                .lock()
                .map(|status| status.clone())
                .unwrap_or_else(|_| UpdateStatus::new(UpdatePhase::Error, None, ""));
            publish(state, failed_status(previous, format!("{error:#}")));
        }
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

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    size: u64,
    state: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Clone, Debug)]
struct Candidate {
    version: [u16; 4],
    url: reqwest::Url,
    size: u64,
    digest: [u8; 32],
}

fn release_version(tag: &str) -> Result<[u16; 4]> {
    let parts: Vec<_> = tag
        .strip_prefix('v')
        .context("Tag de release non pris en charge")?
        .split('.')
        .collect();
    ensure!(parts.len() == 3, "Tag de release non pris en charge");
    let mut version = [0; 4];
    for (index, part) in parts.iter().enumerate() {
        ensure!(
            !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()),
            "Tag de release non pris en charge"
        );
        version[index] = part
            .parse()
            .context("Version de release hors limites MSIX")?;
    }
    ensure!(
        tag == format!("v{}.{}.{}", version[0], version[1], version[2]),
        "Tag de release non canonique"
    );
    Ok(version)
}

fn version_string(version: [u16; 4]) -> String {
    format!(
        "{}.{}.{}.{}",
        version[0], version[1], version[2], version[3]
    )
}

fn digest_bytes(digest: &str) -> Result<[u8; 32]> {
    let hex = digest
        .strip_prefix("sha256:")
        .context("Digest SHA256 absent")?;
    ensure!(
        hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Digest SHA256 invalide"
    );
    let mut value = [0; 32];
    for (index, byte) in value.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)?;
    }
    Ok(value)
}

fn parse_candidate(metadata: &[u8]) -> Result<Candidate> {
    ensure!(
        metadata.len() as u64 <= MAX_METADATA_SIZE,
        "Métadonnées de release trop volumineuses"
    );
    let release: Release =
        serde_json::from_slice(metadata).context("Métadonnées de release invalides")?;
    ensure!(
        !release.draft
            && !release.prerelease
            && release
                .published_at
                .as_ref()
                .is_some_and(|date| !date.is_empty()),
        "Aucune release stable publiée"
    );
    let version = release_version(&release.tag_name)?;
    let expected_name = format!("MayhemLens_{}_x64.msix", version_string(version));
    let mut matches = release
        .assets
        .iter()
        .filter(|asset| asset.name == expected_name);
    let asset = matches
        .next()
        .context("MSIX attendu absent de la release")?;
    ensure!(
        matches.next().is_none(),
        "Plusieurs MSIX correspondent à la release"
    );
    ensure!(asset.state == "uploaded", "MSIX non publié complètement");
    ensure!(
        (1..=MAX_PACKAGE_SIZE).contains(&asset.size),
        "Taille MSIX hors limites"
    );
    let expected_url = format!(
        "https://github.com/Aleqsd/mayhem-lens/releases/download/{}/{}",
        release.tag_name, expected_name
    );
    let url = reqwest::Url::parse(&asset.browser_download_url).context("URL MSIX invalide")?;
    ensure!(
        url.as_str() == expected_url,
        "URL MSIX hors du dépôt ou du tag attendu"
    );
    let digest = digest_bytes(asset.digest.as_deref().context("Digest SHA256 absent")?)?;
    Ok(Candidate {
        version,
        url,
        size: asset.size,
        digest,
    })
}

fn allowed_download_redirect(url: &reqwest::Url, previous_count: usize) -> bool {
    previous_count <= 5
        && url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
        )
}

fn http_client(download: bool) -> Result<Client> {
    let redirects = if download {
        reqwest::redirect::Policy::custom(|attempt| {
            if allowed_download_redirect(attempt.url(), attempt.previous().len()) {
                attempt.follow()
            } else {
                attempt.error("Redirection MSIX hors des CDN GitHub autorisés")
            }
        })
    } else {
        reqwest::redirect::Policy::none()
    };
    Client::builder()
        .https_only(true)
        .user_agent(format!("MayhemLens/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(if download { 120 } else { 20 }))
        .redirect(redirects)
        .build()
        .context("Création du client de mise à jour")
}

fn request_error(stage: &'static str, error: reqwest::Error) -> anyhow::Error {
    // Do not log signed CDN query strings or arbitrary HTTP response bodies.
    if error.is_timeout() {
        anyhow::anyhow!("{stage} : délai HTTP dépassé")
    } else if let Some(status) = error.status() {
        anyhow::anyhow!("{stage} : HTTP {}", status.as_u16())
    } else {
        anyhow::anyhow!("{stage} : requête HTTPS refusée")
    }
}

fn fetch_candidate() -> Result<Candidate> {
    let response = http_client(false)?
        .get(RELEASE_API_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|error| request_error("Lecture de release", error))?;
    let response = response
        .error_for_status()
        .map_err(|error| request_error("Lecture de release", error))?;
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= MAX_METADATA_SIZE),
        "Métadonnées de release trop volumineuses"
    );
    let mut metadata = Vec::new();
    response
        .take(MAX_METADATA_SIZE + 1)
        .read_to_end(&mut metadata)
        .context("Lecture des métadonnées de release")?;
    parse_candidate(&metadata)
}

fn availability_phase(installed: [u16; 4], target: [u16; 4]) -> UpdatePhase {
    if target > installed {
        UpdatePhase::Available
    } else {
        UpdatePhase::UpToDate
    }
}

fn staging_allowed(phase: UpdatePhase) -> bool {
    phase == UpdatePhase::Available
}

fn deployment_phase(error_code: i32, is_registered: bool) -> Result<UpdatePhase> {
    ensure!(
        error_code >= 0,
        "Déploiement MSIX refusé (HRESULT {:08X})",
        error_code as u32
    );
    Ok(if is_registered {
        UpdatePhase::Registered
    } else {
        UpdatePhase::ReadyOnRestart
    })
}

fn transfer_verified(
    reader: &mut impl Read,
    writer: &mut impl Write,
    candidate: &Candidate,
) -> Result<()> {
    transfer_verified_with(reader, writer, candidate, |_| {})
}

#[derive(Clone, Copy)]
enum TransferEvent {
    Bytes(u64),
    Verifying,
}

struct DownloadProgress {
    status: UpdateStatus,
    last_publish: Instant,
}

impl DownloadProgress {
    fn new(mut status: UpdateStatus, candidate: &Candidate, now: Instant) -> Self {
        status.phase = UpdatePhase::Downloading;
        status.target_version = Some(version_string(candidate.version));
        status.downloaded_bytes = 0;
        status.total_bytes = Some(candidate.size);
        status.check_error = None;
        Self {
            status,
            last_publish: now,
        }
    }

    fn event(&mut self, event: TransferEvent, now: Instant) -> Option<UpdateStatus> {
        match event {
            TransferEvent::Bytes(bytes) => {
                let total = self.status.total_bytes.unwrap_or(0);
                self.status.downloaded_bytes = self.status.downloaded_bytes.max(bytes.min(total));
                if now.saturating_duration_since(self.last_publish) < PROGRESS_INTERVAL {
                    return None;
                }
            }
            TransferEvent::Verifying => {
                self.status.phase = UpdatePhase::Verifying;
                self.status.downloaded_bytes = self.status.total_bytes.unwrap_or(0);
            }
        }
        self.last_publish = now;
        Some(self.status.clone())
    }
}

fn transfer_verified_with(
    reader: &mut impl Read,
    writer: &mut impl Write,
    candidate: &Candidate,
    mut progress: impl FnMut(TransferEvent),
) -> Result<()> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    loop {
        let count = reader.read(&mut buffer).context("Lecture du MSIX")?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .context("Taille MSIX invalide")?;
        ensure!(
            total <= candidate.size && total <= MAX_PACKAGE_SIZE,
            "Taille MSIX téléchargée excessive"
        );
        hash.update(&buffer[..count]);
        writer
            .write_all(&buffer[..count])
            .context("Écriture du MSIX temporaire")?;
        progress(TransferEvent::Bytes(total));
    }
    ensure!(
        total == candidate.size,
        "MSIX incomplet ou de taille incorrecte"
    );
    progress(TransferEvent::Verifying);
    let actual: [u8; 32] = hash.finalize().into();
    ensure!(
        actual == candidate.digest,
        "Le digest SHA256 du MSIX ne correspond pas à la release"
    );
    writer.flush().context("Finalisation du MSIX temporaire")?;
    Ok(())
}

/// Owns only the unique file successfully created by this request. A crash may
/// leave it behind; no partial or previous file is ever treated as a cache.
struct TemporaryPackage {
    path: PathBuf,
    handle: Option<File>,
}
impl Drop for TemporaryPackage {
    fn drop(&mut self) {
        // Close our deny-write/delete handle before removing our own file.
        drop(self.handle.take());
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
fn download_candidate(
    candidate: &Candidate,
    progress: impl FnMut(TransferEvent),
) -> Result<TemporaryPackage> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::System::Com::CoCreateGuid;
    let directory = crate::config::app_directory().join("updates");
    fs::create_dir_all(&directory).context("Création du dossier des mises à jour")?;
    // SAFETY: requests a fresh OS-generated UUID, no COM interface or GUI.
    let unique = unsafe { CoCreateGuid() }?;
    let path = directory.join(format!(
        "MayhemLens-{}-{:032x}.msix",
        version_string(candidate.version),
        unique.to_u128()
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(&path)
        .context("Création exclusive du MSIX temporaire")?;
    let mut package = TemporaryPackage {
        path,
        handle: Some(file),
    };
    let mut response = http_client(true)?
        .get(candidate.url.clone())
        .send()
        .map_err(|error| request_error("Téléchargement MSIX", error))?
        .error_for_status()
        .map_err(|error| request_error("Téléchargement MSIX", error))?;
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size == candidate.size),
        "Taille HTTP du MSIX différente de la release"
    );
    transfer_verified_with(
        &mut response,
        package.handle.as_mut().context("Fichier MSIX absent")?,
        candidate,
        progress,
    )?;
    drop(package.handle.take());
    // FILE_SHARE_READ = 1: manifest validation and Windows can read the source,
    // but another process cannot replace/write/delete it while it is staged.
    package.handle = Some(
        OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&package.path)
            .context("Verrouillage en lecture du MSIX vérifié")?,
    );
    // Recheck under the immutable read handle to close the write/reopen race.
    transfer_verified(
        package.handle.as_mut().context("Fichier MSIX absent")?,
        &mut std::io::sink(),
        candidate,
    )?;
    Ok(package)
}

#[cfg(windows)]
mod windows_update {
    use super::*;
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
            // SAFETY: this caller balances WinRT after its package objects drop.
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
        manager: PackageManager,
        version: [u16; 4],
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
            Ok(Self {
                manager: PackageManager::new()?,
                version: [value.Major, value.Minor, value.Build, value.Revision],
                build,
                _apartment: apartment,
            })
        }

        fn status(&self, phase: UpdatePhase, detail: impl Into<String>) -> UpdateStatus {
            UpdateStatus::new(phase, Some(version_string(self.version)), detail)
        }

        fn baseline(&self, confirm: bool) -> Result<UpdateStatus> {
            let receipt = load_receipt()?;
            let status = reconcile_receipt(self.version, receipt.as_ref(), unix_seconds());
            if confirm {
                // Keep the receipt after confirmation: a future offline check
                // must retain the evidence rather than erase it preemptively.
                return Ok(persist_confirmation(status, receipt.as_ref(), save_receipt));
            }
            Ok(status)
        }

        fn candidate(&self, mut status: UpdateStatus) -> Result<(UpdateStatus, Option<Candidate>)> {
            if self.build < MINIMUM_DEFERRED_BUILD {
                return Ok((
                    self.status(
                        UpdatePhase::Unsupported,
                        format!("Windows build {}", self.build),
                    ),
                    None,
                ));
            }
            let candidate = fetch_candidate()?;
            let phase = availability_phase(self.version, candidate.version);
            // A previously staged version remains pending even if the channel
            // now serves an equal/older release. Do not download it repeatedly.
            let receipt = load_receipt()?;
            let already_prepared = receipt.as_ref().is_some_and(|receipt| {
                self.version < receipt.target_version && candidate.version <= receipt.target_version
            });
            if !already_prepared {
                if phase == UpdatePhase::Available {
                    status.phase = phase;
                    status.target_version = Some(version_string(candidate.version));
                } else if status.phase != UpdatePhase::Updated {
                    status.phase = phase;
                    status.target_version = None;
                }
            }
            if status.check_error.is_none() {
                status.detail = format!(
                    "Dernière release stable : {}",
                    version_string(candidate.version)
                );
            }
            status.checked_at_unix = unix_seconds();
            Ok((status, Some(candidate)))
        }
    }

    pub fn check() -> Result<UpdateStatus> {
        // Only release metadata is requested. No MSIX download or deployment.
        let context = PackageContext::new()?;
        let baseline = context.baseline(false)?;
        match context.candidate(baseline.clone()) {
            Ok((status, _)) => Ok(status),
            Err(error) => Ok(failed_status(baseline, format!("{error:#}"))),
        }
    }

    pub fn check_and_stage(progress: impl Fn(UpdateStatus)) -> Result<UpdateStatus> {
        let context = PackageContext::new()?;
        let baseline = context.baseline(true)?;
        let mut checking = baseline.clone();
        checking.phase = UpdatePhase::Checking;
        progress(checking);
        let (checked, candidate) = match context.candidate(baseline.clone()) {
            Ok(value) => value,
            Err(error) => {
                // Restore the receipt-backed proof before the controller adds
                // the failed-check detail; checking remains busy during HTTP.
                progress(baseline);
                return Err(error);
            }
        };
        if !staging_allowed(checked.phase) {
            return Ok(checked);
        }
        let candidate = candidate.context("Release disponible sans package")?;
        let mut download = DownloadProgress::new(checked, &candidate, Instant::now());
        progress(download.status.clone());
        let package = download_candidate(&candidate, |event| {
            if let Some(status) = download.event(event, Instant::now()) {
                progress(status);
            }
        })?;
        crate::update_package::validate(
            &package.path,
            PACKAGE_NAME,
            PACKAGE_PUBLISHER,
            candidate.version,
        )?;
        download.status.phase = UpdatePhase::Preparing;
        progress(download.status.clone());
        let options = AddPackageOptions::new()?;
        options.SetDeferRegistrationWhenPackagesAreInUse(true)?;
        options.SetForceAppShutdown(false)?;
        options.SetForceTargetAppShutdown(false)?;
        options.SetAllowUnsigned(false)?;
        options.SetForceUpdateFromAnyVersion(false)?;
        let url = reqwest::Url::from_file_path(&package.path)
            .map_err(|_| anyhow::anyhow!("Chemin MSIX local invalide"))?;
        let uri = Uri::CreateUri(&HSTRING::from(url.as_str()))?;
        // Windows verifies signature/trust and stages the validated local MSIX.
        // The source remains read-locked until the async operation is finished.
        let result = context
            .manager
            .AddPackageByUriAsync(&uri, &options)?
            .join()?;
        let code = result.ExtendedErrorCode()?;
        let registered = if code.is_ok() {
            result.IsRegistered()?
        } else {
            false
        };
        let phase = deployment_phase(code.0, registered)?;
        save_receipt(&PendingReceipt {
            package_name: PACKAGE_NAME.into(),
            publisher: PACKAGE_PUBLISHER.into(),
            from_version: context.version,
            target_version: candidate.version,
            prepared_phase: phase,
            prepared_at_unix: unix_seconds(),
            confirmed_at_unix: None,
        })?;
        // The completed operation has extracted/staged the payload in Windows'
        // PackageVolume. Dropping package now removes only our source archive.
        download.status.phase = phase;
        download.status.detail = format!("Version cible : {}", version_string(candidate.version));
        Ok(download.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        json!({
            "tag_name": "v1.0.2", "draft": false, "prerelease": false,
            "published_at": "2026-10-04T12:00:00Z",
            "assets": [{
                "name": "MayhemLens_1.0.2.0_x64.msix", "size": 3, "state": "uploaded",
                "browser_download_url": "https://github.com/Aleqsd/mayhem-lens/releases/download/v1.0.2/MayhemLens_1.0.2.0_x64.msix",
                "digest": format!("sha256:{:x}", Sha256::digest(b"abc")),
            }]
        })
    }

    fn parse(value: &Value) -> Result<Candidate> {
        parse_candidate(&serde_json::to_vec(value)?)
    }

    fn prepared_receipt() -> PendingReceipt {
        PendingReceipt {
            package_name: PACKAGE_NAME.into(),
            publisher: PACKAGE_PUBLISHER.into(),
            from_version: [1, 0, 1, 0],
            target_version: [1, 0, 2, 0],
            prepared_phase: UpdatePhase::Registered,
            prepared_at_unix: 20,
            confirmed_at_unix: None,
        }
    }

    #[test]
    fn old_status_json_and_localized_display_remain_compatible() {
        let old = json!({
            "phase":"ready_on_restart", "installed_version":"1.0.1.0",
            "checked_at_unix":12, "detail":"Version cible : 1.0.2.0"
        });
        let status: UpdateStatus = serde_json::from_value(old).unwrap();
        assert!(status.active_version.is_none());
        assert!(status.target_version.is_none());
        assert_eq!(status.downloaded_bytes, 0);
        assert!(status.total_bytes.is_none());
        assert!(
            status
                .display_lines("fr")
                .contains(&"Version active : 1.0.1.0".into())
        );
        assert!(
            status
                .display_lines("en")
                .contains(&"Active version : 1.0.1.0".into())
        );
        for phase in [
            UpdatePhase::Checking,
            UpdatePhase::Downloading,
            UpdatePhase::Verifying,
            UpdatePhase::Preparing,
        ] {
            assert!(UpdateStatus::new(phase, None, "").busy());
        }
        assert!(!UpdateStatus::new(UpdatePhase::Updated, None, "").busy());
    }

    #[test]
    fn download_progress_is_bounded_monotonic_and_throttled() {
        let candidate = Candidate {
            size: 10_000,
            ..parse(&fixture()).unwrap()
        };
        let start = Instant::now();
        let mut progress = DownloadProgress::new(
            UpdateStatus::new(UpdatePhase::Available, Some("1.0.1.0".into()), ""),
            &candidate,
            start,
        );
        let mut published = Vec::new();
        for tick in 1..=10_000_u64 {
            if let Some(status) = progress.event(
                TransferEvent::Bytes(tick),
                start + Duration::from_millis(tick),
            ) {
                published.push(status);
            }
        }
        assert_eq!(published.len(), 10);
        assert!(
            published
                .iter()
                .all(|status| status.downloaded_bytes <= candidate.size)
        );
        assert!(
            published
                .windows(2)
                .all(|pair| pair[0].downloaded_bytes <= pair[1].downloaded_bytes)
        );
        assert_eq!(progress.status.downloaded_bytes, 10_000);
        assert!(
            progress
                .event(
                    TransferEvent::Bytes(20_000),
                    start + Duration::from_millis(10_001)
                )
                .is_none()
        );
        assert!(
            progress
                .event(
                    TransferEvent::Bytes(1),
                    start + Duration::from_millis(10_001)
                )
                .is_none()
        );
        assert_eq!(progress.status.downloaded_bytes, 10_000);
        let verifying = progress
            .event(
                TransferEvent::Verifying,
                start + Duration::from_millis(10_002),
            )
            .unwrap();
        assert_eq!(verifying.phase, UpdatePhase::Verifying);
        assert!(verifying.busy());
        assert_eq!(verifying.downloaded_bytes, candidate.size);
        assert_eq!(verifying.target_version.as_deref(), Some("1.0.2.0"));
        assert!(
            verifying
                .display_lines("en")
                .iter()
                .any(|line| line.contains("100 %"))
        );
    }

    #[test]
    fn corruption_stops_after_verification_and_never_becomes_ready() {
        let candidate = parse(&fixture()).unwrap();
        let start = Instant::now();
        let mut progress = DownloadProgress::new(
            UpdateStatus::new(UpdatePhase::Available, Some("1.0.1.0".into()), ""),
            &candidate,
            start,
        );
        let result = transfer_verified_with(
            &mut std::io::Cursor::new(b"abd"),
            &mut Vec::new(),
            &candidate,
            |event| {
                let _ = progress.event(event, start + PROGRESS_INTERVAL);
            },
        );
        assert!(result.is_err());
        assert_eq!(progress.status.phase, UpdatePhase::Verifying);
        let failed = failed_status(progress.status, result.unwrap_err().to_string());
        assert_eq!(failed.phase, UpdatePhase::Error);
        assert!(!failed.busy());
        assert_eq!(failed.active_version.as_deref(), Some("1.0.1.0"));
        assert_eq!(failed.target_version.as_deref(), Some("1.0.2.0"));
    }

    #[test]
    fn receipt_confirms_only_the_active_target_and_survives_offline_check() {
        let mut receipt = prepared_receipt();
        receipt.validate().unwrap();
        let old = reconcile_receipt([1, 0, 1, 0], Some(&receipt), 30);
        assert_eq!(old.phase, UpdatePhase::Registered);
        assert_eq!(old.active_version.as_deref(), Some("1.0.1.0"));
        assert_eq!(old.target_version.as_deref(), Some("1.0.2.0"));
        assert!(old.confirmed_version.is_none());
        let pending_offline = failed_status(old, "Réseau indisponible".into());
        assert_eq!(pending_offline.phase, UpdatePhase::Registered);
        assert_eq!(pending_offline.target_version.as_deref(), Some("1.0.2.0"));
        assert!(pending_offline.confirmed_version.is_none());
        let relaunched = reconcile_receipt([1, 0, 2, 0], Some(&receipt), 40);
        assert_eq!(relaunched.phase, UpdatePhase::Updated);
        assert_eq!(relaunched.confirmed_version.as_deref(), Some("1.0.2.0"));
        assert_eq!(relaunched.confirmed_at_unix, Some(40));
        receipt.confirmed_at_unix = relaunched.confirmed_at_unix;
        let persisted: PendingReceipt =
            serde_json::from_slice(&serde_json::to_vec(&receipt).unwrap()).unwrap();
        let later = reconcile_receipt([1, 0, 2, 0], Some(&persisted), 90);
        assert_eq!(later.confirmed_at_unix, Some(40));
        let offline = failed_status(later, "Délai HTTP dépassé".into());
        assert_eq!(offline.phase, UpdatePhase::Updated);
        assert_eq!(offline.confirmed_at_unix, Some(40));
        assert!(offline.check_error.is_some());
        assert!(
            offline
                .display_lines("fr")
                .iter()
                .any(|line| line.contains("confirmée au lancement"))
        );
        // A greater active version is not proof that this exact target ran.
        assert_eq!(
            reconcile_receipt([1, 0, 3, 0], Some(&receipt), 90).phase,
            UpdatePhase::Checking
        );
        receipt.prepared_phase = UpdatePhase::Verifying;
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn confirmation_write_failure_preserves_proof_and_the_pending_receipt() {
        let receipt = prepared_receipt();
        let active = reconcile_receipt([1, 0, 2, 0], Some(&receipt), 40);
        let mut attempted = None;
        let confirmed = persist_confirmation(active, Some(&receipt), |value| {
            attempted = Some(value.clone());
            Err(anyhow::anyhow!("synthetic write failure"))
        });
        assert_eq!(confirmed.phase, UpdatePhase::Updated);
        assert_eq!(confirmed.active_version.as_deref(), Some("1.0.2.0"));
        assert_eq!(confirmed.confirmed_version.as_deref(), Some("1.0.2.0"));
        assert_eq!(confirmed.confirmed_at_unix, Some(40));
        assert!(confirmed.detail.contains("synthetic write failure"));
        assert_eq!(
            confirmed.check_error.as_deref(),
            Some(confirmed.detail.as_str())
        );
        assert_eq!(attempted.unwrap().confirmed_at_unix, Some(40));
        // The durable input is unchanged and can prove the update next launch.
        assert!(receipt.confirmed_at_unix.is_none());
        assert_eq!(receipt.prepared_phase, UpdatePhase::Registered);
        let relaunched = reconcile_receipt([1, 0, 2, 0], Some(&receipt), 60);
        assert_eq!(relaunched.phase, UpdatePhase::Updated);
        let persisted = persist_confirmation(relaunched, Some(&receipt), |_| Ok(()));
        assert!(persisted.check_error.is_none());

        let pending = reconcile_receipt([1, 0, 1, 0], Some(&receipt), 70);
        let pending = persist_confirmation(pending, Some(&receipt), |_| {
            panic!("an inactive target must never be confirmed")
        });
        assert_eq!(pending.phase, UpdatePhase::Registered);
        assert!(pending.confirmed_version.is_none());
    }

    #[test]
    fn only_exact_stable_versioned_msix_is_selected() {
        let candidate = parse(&fixture()).unwrap();
        assert_eq!(candidate.version, [1, 0, 2, 0]);
        for (key, value) in [
            ("draft", json!(true)),
            ("prerelease", json!(true)),
            ("published_at", Value::Null),
            ("tag_name", json!("v1.0.2-beta")),
            ("tag_name", json!("v01.0.2")),
            ("tag_name", json!("v65536.0.2")),
            ("tag_name", json!("v1.0.2.0")),
        ] {
            let mut invalid = fixture();
            invalid[key] = value;
            assert!(parse(&invalid).is_err());
        }
        let mut missing = fixture();
        missing["assets"][0]["name"] = json!("MayhemLens_1.0.1.0_x64.msix");
        assert!(parse(&missing).is_err());
        let mut duplicate = fixture();
        let asset = duplicate["assets"][0].clone();
        duplicate["assets"].as_array_mut().unwrap().push(asset);
        assert!(parse(&duplicate).is_err());
    }

    #[test]
    fn asset_size_digest_and_pinned_url_are_mandatory() {
        for (key, value) in [
            ("size", json!(0)),
            ("size", json!(MAX_PACKAGE_SIZE + 1)),
            ("digest", Value::Null),
            ("digest", json!("sha256:abcd")),
            ("digest", json!(format!("sha256:{}", "g".repeat(64)))),
            ("state", json!("new")),
            (
                "browser_download_url",
                json!(
                    "https://github.com/Aleqsd/mayhem-lens/releases/latest/download/MayhemLens_1.0.2.0_x64.msix"
                ),
            ),
            (
                "browser_download_url",
                json!(
                    "https://github.com/Other/mayhem-lens/releases/download/v1.0.2/MayhemLens_1.0.2.0_x64.msix"
                ),
            ),
            (
                "browser_download_url",
                json!(
                    "https://github.com.evil.test/Aleqsd/mayhem-lens/releases/download/v1.0.2/MayhemLens_1.0.2.0_x64.msix"
                ),
            ),
        ] {
            let mut invalid = fixture();
            invalid["assets"][0][key] = value;
            assert!(parse(&invalid).is_err());
        }
        assert!(parse_candidate(&vec![b' '; MAX_METADATA_SIZE as usize + 1]).is_err());
        assert!(parse_candidate(b"not json").is_err());
    }

    #[test]
    fn redirects_are_restricted_to_https_github_cdns() {
        for url in [
            "https://release-assets.githubusercontent.com/github-production-release-asset/test?sig=value",
            "https://objects.githubusercontent.com/github-production-release-asset/test",
        ] {
            assert!(allowed_download_redirect(
                &reqwest::Url::parse(url).unwrap(),
                1
            ));
        }
        for url in [
            "http://release-assets.githubusercontent.com/test",
            "https://evil.test/test",
            "https://github.com/Aleqsd/mayhem-lens/test",
            "https://release-assets.githubusercontent.com.evil.test/test",
            "https://user:password@objects.githubusercontent.com/test",
            "https://objects.githubusercontent.com:444/test",
            "https://objects.githubusercontent.com/test#fragment",
        ] {
            assert!(!allowed_download_redirect(
                &reqwest::Url::parse(url).unwrap(),
                1
            ));
        }
        let cdn = reqwest::Url::parse("https://release-assets.githubusercontent.com/test").unwrap();
        assert!(allowed_download_redirect(&cdn, 5));
        assert!(!allowed_download_redirect(&cdn, 6));
    }

    #[test]
    fn only_newer_versions_can_be_staged() {
        for (installed, target, expected) in [
            ([1, 0, 1, 0], [1, 0, 2, 0], UpdatePhase::Available),
            ([1, 0, 1, 0], [1, 0, 1, 0], UpdatePhase::UpToDate),
            ([1, 0, 2, 0], [1, 0, 1, 0], UpdatePhase::UpToDate),
            ([1, 0, 1, 1], [1, 0, 1, 0], UpdatePhase::UpToDate),
            ([1, 9, 99, 0], [2, 0, 0, 0], UpdatePhase::Available),
        ] {
            assert_eq!(availability_phase(installed, target), expected);
            assert_eq!(
                staging_allowed(expected),
                expected == UpdatePhase::Available
            );
        }
        for phase in [
            UpdatePhase::Error,
            UpdatePhase::Unassociated,
            UpdatePhase::Unsupported,
            UpdatePhase::Registered,
            UpdatePhase::ReadyOnRestart,
            UpdatePhase::Checking,
        ] {
            assert!(!staging_allowed(phase));
        }
    }

    #[test]
    fn stream_rejects_truncation_extra_bytes_and_corruption() {
        struct ShortReader<'a> {
            bytes: &'a [u8],
            offset: usize,
            max_read: usize,
            fail_at: Option<usize>,
        }
        impl Read for ShortReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.fail_at.is_some_and(|limit| self.offset >= limit) {
                    return Err(std::io::Error::other("synthetic read failure"));
                }
                let remaining = self.bytes.len() - self.offset;
                let until_failure = self.fail_at.map_or(remaining, |limit| limit - self.offset);
                let count = buffer
                    .len()
                    .min(self.max_read)
                    .min(remaining)
                    .min(until_failure);
                buffer[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
                self.offset += count;
                Ok(count)
            }
        }
        struct FaultyWriter {
            written: usize,
            fail_at: usize,
            fail_flush: bool,
        }
        impl Write for FaultyWriter {
            fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
                if self.written >= self.fail_at {
                    return Err(std::io::Error::other("synthetic write failure"));
                }
                let count = buffer.len().min(self.fail_at - self.written);
                self.written += count;
                Ok(count)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                if self.fail_flush {
                    Err(std::io::Error::other("synthetic flush failure"))
                } else {
                    Ok(())
                }
            }
        }

        let candidate = parse(&fixture()).unwrap();
        let mut output = Vec::new();
        transfer_verified(&mut std::io::Cursor::new(b"abc"), &mut output, &candidate).unwrap();
        assert_eq!(output, b"abc");
        for bytes in [&b"ab"[..], &b"abcd"[..], &b"abd"[..]] {
            assert!(
                transfer_verified(
                    &mut std::io::Cursor::new(bytes),
                    &mut Vec::new(),
                    &candidate
                )
                .is_err()
            );
        }

        // More than three 64-KiB buffers; expected digest independently computed
        // with .NET SHA256 over the byte pattern, not with this streaming helper.
        let payload: Vec<u8> = (0..3 * 64 * 1024 + 17)
            .map(|index| (index % 251) as u8)
            .collect();
        let large = Candidate {
            size: payload.len() as u64,
            digest: digest_bytes(
                "sha256:4bb7439bc39bc2d0e3d6a915d7c81e38250a9c5bb320a94a19a95bba0d5fe40a",
            )
            .unwrap(),
            ..candidate
        };
        let mut multi_chunk_output = Vec::new();
        transfer_verified(
            &mut std::io::Cursor::new(&payload),
            &mut multi_chunk_output,
            &large,
        )
        .unwrap();
        assert_eq!(multi_chunk_output, payload);
        for max_read in [17, 997, 65_537] {
            let mut reader = ShortReader {
                bytes: &payload,
                offset: 0,
                max_read,
                fail_at: None,
            };
            let mut output = Vec::new();
            transfer_verified(&mut reader, &mut output, &large).unwrap();
            assert_eq!(output, payload);
        }
        let mut corrupted = payload.clone();
        corrupted[2 * 64 * 1024 + 7] ^= 1;
        assert!(
            transfer_verified(
                &mut std::io::Cursor::new(corrupted),
                &mut Vec::new(),
                &large
            )
            .is_err()
        );
        let mut failed_reader = ShortReader {
            bytes: &payload,
            offset: 0,
            max_read: 4096,
            fail_at: Some(64 * 1024 + 11),
        };
        assert!(transfer_verified(&mut failed_reader, &mut Vec::new(), &large).is_err());
        for (fail_at, fail_flush) in [(64 * 1024 + 13, false), (usize::MAX, true)] {
            let mut writer = FaultyWriter {
                written: 0,
                fail_at,
                fail_flush,
            };
            assert!(
                transfer_verified(&mut std::io::Cursor::new(&payload), &mut writer, &large)
                    .is_err()
            );
        }
    }

    #[test]
    fn signature_failures_cannot_be_reported_as_ready_or_registered() {
        // Windows trust/invalid-package failures, simulated without invoking APIs.
        for code in [0x800B0004_u32 as i32, 0x80073CF0_u32 as i32, -1] {
            assert!(deployment_phase(code, false).is_err());
            assert!(deployment_phase(code, true).is_err());
        }
        assert_eq!(
            deployment_phase(0, false).unwrap(),
            UpdatePhase::ReadyOnRestart
        );
        assert_eq!(deployment_phase(0, true).unwrap(), UpdatePhase::Registered);
    }

    #[test]
    fn packed_windows_version_distinguishes_supported_deferred_install() {
        let packed = |build: u64| ((10_u64 << 48) | (build << 16) | 1).to_string();
        assert_eq!(windows_build(&packed(22_000)).unwrap(), 22_000);
        assert!(windows_build(&packed(22_000)).unwrap() < MINIMUM_DEFERRED_BUILD);
        assert!(windows_build(&packed(22_621)).unwrap() >= MINIMUM_DEFERRED_BUILD);
        assert!(windows_build("invalid").is_err());
    }
}
