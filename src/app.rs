//! Runtime coordination. Network/game polling and OCR run on separate workers.
use crate::{
    config::{Config, app_directory},
    data::DataStore,
    domain, game,
    model::{Catalog, Snapshot, Tier},
    native::{self, Badge, Observation, Rect, UserAction},
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Session {
    catalog: Catalog,
    snapshot: Snapshot,
    generation: u64,
    offline: bool,
    matcher: domain::AugmentMatcher,
}

#[derive(Clone)]
struct Offer {
    id: u32,
    rect: Rect,
    confidence: f32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecognizedTitle {
    id: u32,
    // Catalog names only: never retain the OCR input or a player's identity.
    name_fr: String,
    name_en: String,
    confidence: f32,
    tier: Tier,
    stage_specific: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanError {
    stage: &'static str,
    message: &'static str,
    windows_error_code: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanStatus {
    timestamp_unix_ms: u64,
    process_id: u32,
    phase: &'static str,
    champion_id: Option<u32>,
    patch: Option<String>,
    game_foreground: bool,
    last_scan_at_unix_ms: Option<u64>,
    observation_count: usize,
    recognized_title_count: usize,
    offer_count: usize,
    badges_requested_count: usize,
    recognized_titles: Vec<RecognizedTitle>,
    recognized_titles_truncated: bool,
    offers: Vec<RecognizedTitle>,
    capture_ocr_ms: Option<f64>,
    association_ms: Option<f64>,
    total_scan_ms: Option<f64>,
    error: Option<ScanError>,
}

impl ScanStatus {
    fn new() -> Self {
        Self {
            timestamp_unix_ms: unix_ms(),
            process_id: std::process::id(),
            phase: "waitingForSession",
            champion_id: None,
            patch: None,
            game_foreground: false,
            last_scan_at_unix_ms: None,
            observation_count: 0,
            recognized_title_count: 0,
            offer_count: 0,
            badges_requested_count: 0,
            recognized_titles: Vec::new(),
            recognized_titles_truncated: false,
            offers: Vec::new(),
            capture_ocr_ms: None,
            association_ms: None,
            total_scan_ms: None,
            error: None,
        }
    }

    fn clear_reading(&mut self, phase: &'static str) {
        self.phase = phase;
        self.last_scan_at_unix_ms = None;
        self.observation_count = 0;
        self.recognized_title_count = 0;
        self.offer_count = 0;
        self.badges_requested_count = 0;
        self.recognized_titles.clear();
        self.recognized_titles_truncated = false;
        self.offers.clear();
        self.capture_ocr_ms = None;
        self.association_ms = None;
        self.total_scan_ms = None;
        self.error = None;
    }

    fn record_titles(
        &mut self,
        active: &Session,
        config: &Config,
        titles: &[Offer],
        offers: &[Offer],
    ) {
        self.recognized_title_count = titles.len();
        // Keep the snapshot small even if an OCR engine returns many title aliases.
        self.recognized_titles_truncated = titles.len() > 16;
        self.recognized_titles = diagnostic_titles(active, config, &titles[..titles.len().min(16)]);
        self.offer_count = offers.len();
        self.offers = diagnostic_titles(active, config, offers);
        self.phase = if offers.len() == 3 {
            "threeOffers"
        } else {
            "noOfferGroup"
        };
        self.error = None;
    }
}

/// One overwritten local snapshot, with serialization/disk I/O off the OCR worker.
/// A failed or busy diagnostic writer never changes runtime decisions.
struct ScanStatusWriter {
    sender: Option<mpsc::SyncSender<ScanStatus>>,
    last_attempt: Option<Instant>,
}

impl ScanStatusWriter {
    fn new(path: PathBuf) -> Self {
        let (sender, snapshots) = mpsc::sync_channel::<ScanStatus>(1);
        let worker = thread::Builder::new()
            .name("mayhem-scan-status".into())
            .spawn(move || {
                let mut last_write: Option<Instant> = None;
                while let Ok(mut snapshot) = snapshots.recv() {
                    if let Some(previous) = last_write {
                        thread::sleep(Duration::from_secs(1).saturating_sub(previous.elapsed()));
                    }
                    while let Ok(newer) = snapshots.try_recv() {
                        snapshot = newer;
                    }
                    let _ = write_scan_snapshot(&path, &snapshot);
                    last_write = Some(Instant::now());
                }
            });
        Self {
            sender: worker.ok().map(|_| sender),
            last_attempt: None,
        }
    }

    fn publish_if_due(&mut self, status: &ScanStatus, now: Instant) {
        if self
            .last_attempt
            .is_some_and(|previous| now.duration_since(previous) < Duration::from_secs(1))
        {
            return;
        }
        self.last_attempt = Some(now);
        if let Some(sender) = &self.sender {
            let mut snapshot = status.clone();
            snapshot.timestamp_unix_ms = unix_ms();
            let _ = sender.try_send(snapshot);
        }
    }
}

fn write_scan_snapshot(path: &std::path::Path, status: &ScanStatus) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(status)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn diagnostic_titles(active: &Session, config: &Config, offers: &[Offer]) -> Vec<RecognizedTitle> {
    let ids: Vec<_> = offers.iter().map(|offer| offer.id).collect();
    let grades = domain::recommend(
        &active.snapshot,
        &active.catalog,
        &ids,
        &config.selected_augments,
        config.offer_stage,
    );
    offers
        .iter()
        .zip(grades)
        .filter_map(|(offer, grade)| {
            let augment = active
                .catalog
                .augments
                .iter()
                .find(|augment| augment.id == offer.id)?;
            Some(RecognizedTitle {
                id: offer.id,
                name_fr: augment.name_fr.clone(),
                name_en: augment.name_en.clone(),
                confidence: offer.confidence,
                tier: grade.tier,
                stage_specific: grade.stage_specific,
            })
        })
        .collect()
}

fn safe_scan_error(error: &anyhow::Error) -> ScanError {
    // Persist only fixed context and a numeric HRESULT, never arbitrary error
    // strings that might contain paths, captured text or source response data.
    ScanError {
        stage: "captureOcr",
        message: "Capture ou OCR indisponible",
        windows_error_code: error.chain().find_map(|cause| {
            cause
                .downcast_ref::<windows::core::Error>()
                .map(|value| format!("{:08X}", value.code().0 as u32))
        }),
    }
}

pub fn run(cache: PathBuf, config_path: PathBuf) -> Result<()> {
    // Require package/OCR support before starting the game workers.
    if !config_path.exists() {
        Config::default().save(&config_path)?;
    }
    let _instance = native::acquire_single_instance()?;
    let updates = crate::update::UpdateController::start()?;
    native::ensure_ready(&Config::load(&config_path)?.language)?;
    let stop = Arc::new(AtomicBool::new(false));
    let session: Arc<RwLock<Option<Arc<Session>>>> = Arc::new(RwLock::new(None));
    let status = Arc::new(Mutex::new(String::new()));
    let (badges_tx, badges_rx) = mpsc::channel();
    let (actions_tx, actions_rx) = mpsc::channel();
    let data_stop = Arc::clone(&stop);
    let data_session = Arc::clone(&session);
    let data_status = Arc::clone(&status);
    let data_worker = thread::Builder::new()
        .name("mayhem-data".into())
        .spawn(move || {
            if let Err(error) = data_loop(cache, &data_stop, &data_session, &data_status) {
                write_status(&data_status, "data-error", &format!("{error:#}"));
            }
        })?;
    let scan_stop = Arc::clone(&stop);
    let scan_status = Arc::clone(&status);
    let scan_config_path = config_path.clone();
    let scan_worker = thread::Builder::new()
        .name("mayhem-ocr".into())
        .spawn(move || {
            if let Err(error) = scan_loop(
                &scan_stop,
                &session,
                &scan_status,
                badges_tx,
                actions_rx,
                scan_config_path,
            ) {
                write_status(&scan_status, "ocr-error", &format!("{error:#}"));
                scan_stop.store(true, Ordering::Relaxed);
            }
        })?;
    let result = native::run_overlay(
        badges_rx,
        actions_tx,
        Arc::clone(&stop),
        &config_path,
        updates,
    );
    stop.store(true, Ordering::Relaxed);
    // A slow provider or OCR operation must not keep the application alive after
    // Quit. The executable exits after run returns; OS cleanup releases workers.
    let deadline = Instant::now() + Duration::from_secs(2);
    while (!scan_worker.is_finished() || !data_worker.is_finished()) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(30));
    }
    if scan_worker.is_finished() {
        let _ = scan_worker.join();
    }
    if data_worker.is_finished() {
        let _ = data_worker.join();
    }
    write_status(&status, "stopped", "Overlay arrêté");
    result
}

fn data_loop(
    cache: PathBuf,
    stop: &AtomicBool,
    session: &RwLock<Option<Arc<Session>>>,
    status: &Mutex<String>,
) -> Result<()> {
    let store = DataStore::new(cache)?;
    let mut champion = String::new();
    let mut last_time = 0.0;
    let mut generation = 0;
    let mut prepared: Option<Arc<Session>> = None;
    let mut retry_after = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match game::read_game() {
            Ok(Some(game)) => {
                let changed = game_context_changed(&champion, last_time, &game);
                if changed {
                    generation += 1;
                    champion.clone_from(&game.champion_key);
                    *session
                        .write()
                        .map_err(|_| anyhow::anyhow!("Verrou session"))? = None;
                    retry_after = Instant::now();
                    prepared = None;
                }
                last_time = game.game_time;
                if prepared.is_some() {
                    *session
                        .write()
                        .map_err(|_| anyhow::anyhow!("Verrou session"))? = prepared.clone();
                }
                if prepared.is_none() && Instant::now() >= retry_after {
                    write_status(
                        status,
                        "loading",
                        "Chargement des données Mayhem du champion",
                    );
                    let load = (|| -> Result<Session> {
                        let catalog = store.catalog()?;
                        let id = resolve_champion_id(&game, &catalog, || store.refresh_catalog())?;
                        let (snapshot, offline) = match store.sync_champion(id) {
                            Ok(snapshot) => (snapshot, false),
                            Err(sync_error) => (
                                store.cached_champion(id).with_context(|| {
                                    format!(
                                        "Sync indisponible et aucun cache compatible : {sync_error}"
                                    )
                                })?,
                                true,
                            ),
                        };
                        // Sync can select a newer patch than the initial ID catalog.
                        let catalog = store.cached_catalog()?;
                        let matcher = domain::AugmentMatcher::new(&catalog);
                        Ok(Session {
                            catalog,
                            snapshot,
                            generation,
                            offline,
                            matcher,
                        })
                    })();
                    match load {
                        Ok(loaded) => {
                            // Loading can outlive the game context that requested it.
                            // Errors/non-KIWI/absence are all insufficient to publish.
                            let latest_game = game::read_game().ok().flatten();
                            if prepared_session_is_current(&game, latest_game.as_ref()) {
                                write_status(
                                    status,
                                    if loaded.offline {
                                        "ready-offline"
                                    } else {
                                        "ready"
                                    },
                                    &format!(
                                        "ARAMKit Mayhem / champion {} / patch {} / dataset {}",
                                        loaded.snapshot.champion_id,
                                        loaded.snapshot.patch,
                                        loaded.snapshot.dataset_date
                                    ),
                                );
                                prepared = Some(Arc::new(loaded));
                                *session
                                    .write()
                                    .map_err(|_| anyhow::anyhow!("Verrou session"))? =
                                    prepared.clone();
                            } else {
                                // Preserve champion/last_time: the next poll must
                                // still notice the new context and advance generation.
                                // If the API is flaky in the same game, do not start
                                // provider requests every two seconds.
                                retry_after = Instant::now() + Duration::from_secs(30);
                                write_status(
                                    status,
                                    "session-changed",
                                    "Chargement périmé : contexte de partie à confirmer",
                                );
                            }
                        }
                        Err(error) => {
                            write_status(status, "data-unavailable", &format!("{error:#}"));
                            retry_after = Instant::now() + Duration::from_secs(30);
                        }
                    }
                }
            }
            Ok(None) | Err(_) => {
                *session
                    .write()
                    .map_err(|_| anyhow::anyhow!("Verrou session"))? = None;
                // An API timeout is not proof of a new game. Keep identity/time;
                // a changed champion or backwards game clock starts a new session.
                write_status(status, "waiting", "En attente d'une partie KIWI / Mayhem");
            }
        }
        sleep_stoppable(stop, Duration::from_secs(2));
    }
    Ok(())
}

fn game_context_changed(
    champion_key: &str,
    previous_game_time: f64,
    latest: &game::GameState,
) -> bool {
    champion_key != latest.champion_key || latest.game_time + 10.0 < previous_game_time
}

fn prepared_session_is_current(
    before_load: &game::GameState,
    after_load: Option<&game::GameState>,
) -> bool {
    after_load.is_some_and(|latest| {
        !game_context_changed(&before_load.champion_key, before_load.game_time, latest)
    })
}

fn resolve_champion_id(
    game: &game::GameState,
    cached_catalog: &Catalog,
    refresh_catalog: impl FnOnce() -> Result<Catalog>,
) -> Result<u32> {
    let find = |catalog: &Catalog| {
        catalog
            .champions
            .iter()
            .find(|champion| {
                champion.key.eq_ignore_ascii_case(&game.champion_key)
                    || champion.name_en == game.champion_name
                    || champion.name_fr == game.champion_name
            })
            .map(|champion| champion.id)
    };
    if let Some(id) = find(cached_catalog) {
        return Ok(id);
    }
    // A valid cached manifest can predate the champion's release. Refresh once;
    // failures return to data_loop's existing 30-second retry, never the OCR path.
    let current_catalog = refresh_catalog().context("Rafraîchir le catalogue des champions")?;
    find(&current_catalog).context("Identifiant du champion absent du catalogue courant")
}

fn scan_loop(
    stop: &AtomicBool,
    session: &RwLock<Option<Arc<Session>>>,
    status: &Mutex<String>,
    sender: mpsc::Sender<Vec<Badge>>,
    actions: mpsc::Receiver<UserAction>,
    config_path: PathBuf,
) -> Result<()> {
    let mut config = Config::load(&config_path)?;
    let mut generation = None;
    let mut previous: Vec<Offer> = Vec::new();
    let mut confirmed_cards: Option<Vec<u32>> = None;
    let mut last_read = Instant::now() - Duration::from_secs(10);
    let mut last_valid = Instant::now() - Duration::from_secs(10);
    let mut last_scan = Instant::now() - Duration::from_secs(10);
    let mut diagnostic = ScanStatus::new();
    let mut diagnostic_writer = ScanStatusWriter::new(app_directory().join("scan-status.json"));
    while !stop.load(Ordering::Relaxed) {
        if last_read.elapsed() >= Duration::from_secs(2) {
            match Config::load(&config_path) {
                Ok(settings) => config = settings,
                Err(error) => write_status(status, "config-error", &error.to_string()),
            }
            last_read = Instant::now();
        }
        let mut force = false;
        while let Ok(action) = actions.try_recv() {
            match action {
                UserAction::ForceScan => {
                    native::invalidate_observations();
                    force = true;
                }
                UserAction::SetStage(stage) => {
                    config.offer_stage = stage;
                    config.save(&config_path)?;
                    force = true;
                }
                UserAction::SelectSlot(slot) => {
                    // Only current visible three-card offers can be confirmed once.
                    let ids: Vec<_> = previous.iter().map(|o| o.id).collect();
                    if last_valid.elapsed() < Duration::from_secs(2)
                        && native::game_window_visible()
                        && session
                            .read()
                            .ok()
                            .and_then(|s| s.as_ref().map(|s| s.generation))
                            == generation
                        && confirmed_cards.as_ref() != Some(&ids)
                        && let Some(offer) = previous.get(slot.saturating_sub(1) as usize)
                        && config.selected_augments.len() < 5
                    {
                        config.selected_augments.push(offer.id);
                        config.offer_stage = None;
                        config.save(&config_path)?;
                        confirmed_cards = Some(ids);
                        // Selected count alone doesn't prove the offer stage.
                        write_status(
                            status,
                            "choice-recorded",
                            &format!("Augmentation {} confirmée manuellement", offer.id),
                        );
                        force = true;
                    }
                }
            }
        }
        let active = session
            .read()
            .map_err(|_| anyhow::anyhow!("Verrou session"))?
            .clone();
        if let Some(active) = active {
            diagnostic.champion_id = Some(active.snapshot.champion_id);
            if diagnostic.patch.as_deref() != Some(active.snapshot.patch.as_str()) {
                diagnostic.patch = Some(active.snapshot.patch.clone());
            }
            if generation != Some(active.generation) {
                diagnostic.clear_reading("newSession");
                generation = Some(active.generation);
                previous.clear();
                confirmed_cards = None;
                config.selected_augments.clear();
                config.offer_stage = None;
                config.save(&config_path)?;
            }
            let game_foreground = native::game_window_visible();
            diagnostic.game_foreground = game_foreground;
            if game_foreground
                && (force || last_scan.elapsed() >= Duration::from_millis(config.scan_interval_ms))
            {
                last_scan = Instant::now();
                diagnostic.phase = "scanning";
                diagnostic_writer.publish_if_due(&diagnostic, Instant::now());
                let scan_started = Instant::now();
                match native::observe_game(&config.language) {
                    Ok(observations) => {
                        let capture_ocr = scan_started.elapsed();
                        let association_started = Instant::now();
                        let (recognized, offers) = detect_offers(&observations, &active.matcher);
                        let association = association_started.elapsed();
                        diagnostic.last_scan_at_unix_ms = Some(unix_ms());
                        diagnostic.observation_count = observations.len();
                        diagnostic.capture_ocr_ms = Some(capture_ocr.as_secs_f64() * 1_000.0);
                        diagnostic.association_ms = Some(association.as_secs_f64() * 1_000.0);
                        diagnostic.record_titles(&active, &config, &recognized, &offers);
                        diagnostic.badges_requested_count = 0;
                        let current_generation = session
                            .read()
                            .ok()
                            .and_then(|s| s.as_ref().map(|s| s.generation));
                        if offers.len() == 3 && current_generation == Some(active.generation) {
                            last_valid = Instant::now();
                            previous = offers;
                            let badges = present(&active, &previous, &config);
                            let badges_requested_count = badges.len();
                            if sender.send(badges).is_err() {
                                diagnostic.phase = "displayChannelClosed";
                                diagnostic_writer.publish_if_due(&diagnostic, Instant::now());
                                break;
                            }
                            diagnostic.badges_requested_count = badges_requested_count;
                        } else {
                            if current_generation != Some(active.generation) {
                                diagnostic.phase = "sessionChanged";
                            }
                            previous.clear();
                            confirmed_cards = None;
                            let _ = sender.send(Vec::new());
                        }
                        diagnostic.total_scan_ms =
                            Some(scan_started.elapsed().as_secs_f64() * 1_000.0);
                    }
                    Err(error) => {
                        diagnostic.clear_reading("captureOcrError");
                        diagnostic.last_scan_at_unix_ms = Some(unix_ms());
                        diagnostic.capture_ocr_ms =
                            Some(scan_started.elapsed().as_secs_f64() * 1_000.0);
                        diagnostic.total_scan_ms = diagnostic.capture_ocr_ms;
                        diagnostic.error = Some(safe_scan_error(&error));
                        previous.clear();
                        let _ = sender.send(Vec::new());
                        write_status(status, "ocr-error", &format!("{error:#}"));
                    }
                }
            } else if !native::game_window_visible() {
                diagnostic.game_foreground = false;
                diagnostic.clear_reading("pausedForeground");
                previous.clear();
                let _ = sender.send(Vec::new());
            }
        } else {
            diagnostic.champion_id = None;
            diagnostic.patch = None;
            diagnostic.clear_reading("waitingForSession");
            diagnostic.game_foreground = native::game_window_visible();
            previous.clear();
            let _ = sender.send(Vec::new());
        }
        diagnostic_writer.publish_if_due(&diagnostic, Instant::now());
        sleep_stoppable(stop, Duration::from_millis(80));
    }
    diagnostic.clear_reading("stopped");
    diagnostic.game_foreground = false;
    diagnostic_writer.publish_if_due(&diagnostic, Instant::now());
    let _ = sender.send(Vec::new());
    Ok(())
}

fn detect_offers(
    observations: &[Observation],
    matcher: &domain::AugmentMatcher,
) -> (Vec<Offer>, Vec<Offer>) {
    let mut candidates: Vec<_> = observations
        .iter()
        .filter_map(|o| {
            matcher.match_text(&o.text).map(|m| Offer {
                id: m.id,
                rect: o.rect,
                confidence: m.confidence,
            })
        })
        .collect();
    candidates.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    for candidate in &candidates {
        let tolerance = (candidate.rect.height * 2).max(25);
        let mut row: Vec<_> = candidates
            .iter()
            .filter(|o| (o.rect.y - candidate.rect.y).abs() < tolerance)
            .cloned()
            .collect();
        row.sort_by_key(|o| o.rect.x);
        row.dedup_by_key(|o| o.id);
        if row.len() != 3
            || row
                .iter()
                .map(|o| o.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != 3
        {
            continue;
        }
        let centers: Vec<_> = row.iter().map(|o| o.rect.x + o.rect.width / 2).collect();
        let gap_a = centers[1] - centers[0];
        let gap_b = centers[2] - centers[1];
        // Prevent a list of tooltips/body text from masquerading as three cards.
        if gap_a >= 100 && gap_b >= 100 && (gap_a - gap_b).abs() < gap_a.max(gap_b) / 2 {
            return (candidates, row);
        }
    }
    (candidates, Vec::new())
}

fn present(session: &Session, offers: &[Offer], config: &Config) -> Vec<Badge> {
    let ids: Vec<_> = offers.iter().map(|o| o.id).collect();
    let mut recommended = domain::recommend_localized(
        &session.snapshot,
        &session.catalog,
        &ids,
        &config.selected_augments,
        config.offer_stage,
        &config.language,
    );
    domain::apply_rules(
        &mut recommended,
        &config.selected_augments,
        &config.synergy_rules,
        &config.language,
    );
    let mut badges = Vec::new();
    for (offer, recommendation) in offers.iter().zip(recommended) {
        let title = session
            .catalog
            .augments
            .iter()
            .find(|a| a.id == offer.id)
            .map(|a| {
                if config.language == "en" {
                    a.name_en.as_str()
                } else {
                    a.name_fr.as_str()
                }
            })
            .unwrap_or("?");
        let champion = session
            .catalog
            .champions
            .iter()
            .find(|c| c.id == session.snapshot.champion_id)
            .map(|c| {
                if config.language == "en" {
                    c.name_en.as_str()
                } else {
                    c.name_fr.as_str()
                }
            })
            .unwrap_or("?");
        let context = match (
            config.offer_stage,
            recommendation.stage_specific,
            config.language.as_str(),
        ) {
            (Some(stage), true, "en") => format!("stage {stage}"),
            (Some(stage), true, _) => format!("choix {stage}"),
            (Some(_), false, "en") => "stage unavailable; champion tier".into(),
            (Some(_), false, _) => "choix indisponible ; tier champion".into(),
            (None, _, "en") => "champion tier; stage unknown".into(),
            (None, _, _) => "tier champion ; choix inconnu".into(),
        };
        let mut detail = format!(
            "ARAMKit · {champion} · {context}\n{} · {}{}",
            session.snapshot.patch,
            session.snapshot.dataset_date,
            if session.offline { " · cache" } else { "" }
        );
        if let Some(synergy) = recommendation.synergies.first() {
            detail.push('\n');
            detail.push_str(synergy);
        }
        // Position above titles, keeping the game card available for pointer input.
        badges.push(Badge {
            rect: Rect {
                x: offer.rect.x - 8,
                y: offer.rect.y - 116,
                width: 300,
                height: 110,
            },
            title: format!("{}  {}", recommendation.tier, title),
            detail,
        });
    }
    if config.show_builds {
        let advice = domain::build_advice_localized(
            &session.snapshot,
            &session.catalog,
            &config.selected_augments,
            &config.language,
        );
        if let (Some(build), Some(first)) = (advice.first(), offers.first()) {
            let names: Vec<_> = build
                .item_ids
                .iter()
                .take(3)
                .map(|id| {
                    session
                        .catalog
                        .items
                        .iter()
                        .find(|item| item.id == *id)
                        .map(|item| {
                            if config.language == "en" {
                                item.name_en.clone()
                            } else {
                                item.name_fr.clone()
                            }
                        })
                        .unwrap_or_else(|| format!("#{id}"))
                })
                .collect();
            badges.push(Badge {
                rect: Rect {
                    x: first.rect.x - 8,
                    y: first.rect.y - 192,
                    width: 660,
                    height: 84,
                },
                title: format!("{} : {}", build.label, names.join(" → ")),
                detail: format!(
                    "{} · {} · données {}{}",
                    build.reason,
                    session.snapshot.patch,
                    session.snapshot.dataset_date,
                    if session.offline {
                        " · cache hors ligne"
                    } else {
                        ""
                    }
                ),
            });
        }
    }
    badges
}

fn sleep_stoppable(stop: &AtomicBool, duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(80).min(duration.saturating_sub(start.elapsed())));
    }
}

fn write_status(previous: &Mutex<String>, state: &str, detail: &str) {
    let value = serde_json::json!({"state": state, "detail": detail}).to_string();
    if let Ok(mut previous) = previous.lock()
        && *previous != value
    {
        let _ = std::fs::create_dir_all(app_directory());
        let _ = std::fs::write(app_directory().join("runtime-status.json"), &value);
        *previous = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Augment, Champion};

    #[test]
    fn prepared_session_requires_the_same_game_after_loading() {
        let before = game::GameState {
            champion_key: "Example".into(),
            champion_name: "Exemple".into(),
            level: 5,
            game_time: 100.0,
        };
        for (game_time, expected) in [
            (150.0, true),
            (100.0, true),
            (90.0, true),
            (89.0, false),
            (1.0, false),
        ] {
            let after = game::GameState {
                game_time,
                ..before.clone()
            };
            assert_eq!(
                prepared_session_is_current(&before, Some(&after)),
                expected,
                "game time {game_time}"
            );
        }
        assert!(!prepared_session_is_current(&before, None));
        let different_champion = game::GameState {
            champion_key: "NextExample".into(),
            champion_name: "Autre exemple".into(),
            ..before.clone()
        };
        assert!(!prepared_session_is_current(
            &before,
            Some(&different_champion)
        ));
        // Rejecting the result doesn't consume the next poll's session change.
        assert!(game_context_changed(
            &before.champion_key,
            before.game_time,
            &different_champion
        ));
    }

    #[test]
    fn missing_champion_refreshes_an_older_catalog_once_without_network() {
        let cached = Catalog {
            champions: vec![Champion {
                id: 123,
                key: "OldExample".into(),
                name_fr: "Ancien exemple".into(),
                name_en: "Old example".into(),
            }],
            ..Catalog::default()
        };
        let current = Catalog {
            champions: vec![Champion {
                id: 321,
                key: "NewExample".into(),
                name_fr: "Nouvel exemple".into(),
                name_en: "New example".into(),
            }],
            ..Catalog::default()
        };
        let game = game::GameState {
            champion_key: "newexample".into(),
            champion_name: "Nouvel exemple".into(),
            level: 1,
            game_time: 1.0,
        };
        let refresh_count = std::cell::Cell::new(0);
        let id = resolve_champion_id(&game, &cached, || {
            refresh_count.set(refresh_count.get() + 1);
            Ok(current.clone())
        })
        .unwrap();
        assert_eq!(id, 321);
        assert_eq!(refresh_count.get(), 1);
        assert_eq!(
            resolve_champion_id(&game, &current, || panic!(
                "cached champion needs no refresh"
            ))
            .unwrap(),
            321
        );
    }

    #[test]
    fn champion_still_missing_after_refresh_returns_to_the_bounded_retry() {
        let game = game::GameState {
            champion_key: "UnknownExample".into(),
            champion_name: "Unknown example".into(),
            level: 1,
            game_time: 1.0,
        };
        let refresh_count = std::cell::Cell::new(0);
        let missing = Catalog::default();
        let result = resolve_champion_id(&game, &missing, || {
            refresh_count.set(refresh_count.get() + 1);
            Ok(Catalog::default())
        });
        assert!(result.is_err());
        assert_eq!(refresh_count.get(), 1);
    }

    fn catalog() -> Catalog {
        Catalog {
            augments: ["Spark", "River", "Cloud"]
                .into_iter()
                .enumerate()
                .map(|(i, name)| Augment {
                    id: i as u32 + 1,
                    name_id: name.into(),
                    name_fr: name.into(),
                    name_en: name.into(),
                    description_fr: String::new(),
                    description_en: String::new(),
                })
                .collect(),
            ..Catalog::default()
        }
    }
    fn observation(text: &str, x: i32, y: i32) -> Observation {
        Observation {
            text: text.into(),
            rect: Rect {
                x,
                y,
                width: 80,
                height: 20,
            },
        }
    }
    #[test]
    fn requires_three_distinct_spaced_titles_in_one_row() {
        let offers = [
            observation("Spark", 100, 500),
            observation("River", 400, 503),
            observation("Cloud", 700, 500),
        ];
        let matcher = domain::AugmentMatcher::new(&catalog());
        assert_eq!(detect_offers(&offers, &matcher).1.len(), 3);
        assert!(detect_offers(&offers[..2], &matcher).1.is_empty());
        let list = [
            observation("Spark", 100, 200),
            observation("River", 100, 400),
            observation("Cloud", 100, 600),
        ];
        assert!(detect_offers(&list, &matcher).1.is_empty());
    }
    #[test]
    fn duplicate_or_uneven_layout_is_rejected() {
        let offers = [
            observation("Spark", 100, 500),
            observation("Spark", 400, 500),
            observation("Cloud", 700, 500),
        ];
        let matcher = domain::AugmentMatcher::new(&catalog());
        assert!(detect_offers(&offers, &matcher).1.is_empty());
        let offers = [
            observation("Spark", 100, 500),
            observation("River", 400, 500),
            observation("Spark", 700, 500),
        ];
        assert!(detect_offers(&offers, &matcher).1.is_empty());
        let offers = [
            observation("Spark", 100, 500),
            observation("River", 220, 500),
            observation("Cloud", 1500, 500),
        ];
        assert!(detect_offers(&offers, &matcher).1.is_empty());
    }

    #[test]
    fn diagnostics_record_catalog_grades_without_raw_ocr_or_render_claims() {
        let catalog = catalog();
        let active = Session {
            matcher: domain::AugmentMatcher::new(&catalog),
            catalog,
            snapshot: Snapshot {
                champion_id: 222,
                patch: "26.19".into(),
                source: "Synthetic test".into(),
                dataset: "synthetic".into(),
                dataset_date: String::new(),
                fetched_at: String::new(),
                augments: vec![crate::model::AugmentStat {
                    id: 1,
                    tier: Tier::S,
                    rank: None,
                    sample_count: None,
                }],
                stages: std::collections::BTreeMap::new(),
                builds: Vec::new(),
                item_synergies: Vec::new(),
            },
            generation: 1,
            offline: false,
        };
        let observations = [
            observation("Spark", 100, 500),
            observation("River", 400, 500),
            observation("Cloud", 700, 500),
            observation("PrivatePlayer#1234", 900, 800),
        ];
        let (titles, offers) = detect_offers(&observations, &active.matcher);
        let mut status = ScanStatus::new();
        status.observation_count = observations.len();
        status.record_titles(&active, &Config::default(), &titles, &offers);
        status.badges_requested_count = 4; // Three letters plus a build panel request.
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["observationCount"], 4);
        assert_eq!(value["recognizedTitleCount"], 3);
        assert_eq!(value["offerCount"], 3);
        assert_eq!(value["offers"][0]["tier"], "S");
        assert_eq!(value["badgesRequestedCount"], 4);
        assert!(!value.to_string().contains("PrivatePlayer"));
        assert!(value.get("visibleWindowCount").is_none());
        assert!(value.get("screenshots").is_none());
        status.clear_reading("pausedForeground");
        assert_eq!(status.offer_count, 0);
        assert_eq!(status.badges_requested_count, 0);
        assert!(status.offers.is_empty());
        assert!(status.capture_ocr_ms.is_none());
    }

    #[test]
    fn diagnostic_publication_is_throttled_without_blocking() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut writer = ScanStatusWriter {
            sender: Some(sender),
            last_attempt: None,
        };
        let mut status = ScanStatus::new();
        let start = Instant::now();
        writer.publish_if_due(&status, start);
        assert_eq!(receiver.try_recv().unwrap().phase, "waitingForSession");
        status.phase = "scanning";
        writer.publish_if_due(&status, start + Duration::from_millis(999));
        assert!(receiver.try_recv().is_err());
        writer.publish_if_due(&status, start + Duration::from_secs(1));
        assert_eq!(receiver.try_recv().unwrap().phase, "scanning");
        // A full queue drops an update instead of holding the OCR loop.
        writer.publish_if_due(&status, start + Duration::from_secs(2));
        writer.publish_if_due(&status, start + Duration::from_secs(3));
        assert_eq!(receiver.try_recv().unwrap().phase, "scanning");
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn snapshot_replaces_one_file_and_error_text_remains_private() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("mayhem-scan-test-{}-{suffix}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("scan-status.json");
        let mut status = ScanStatus::new();
        write_scan_snapshot(&path, &status).unwrap();
        status.phase = "captureOcrError";
        status.error = Some(safe_scan_error(&anyhow::anyhow!(
            "PrivatePlayer#1234 and screenshot.png"
        )));
        write_scan_snapshot(&path, &status).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["phase"], "captureOcrError");
        assert_eq!(value["error"]["stage"], "captureOcr");
        assert!(!contents.contains("PrivatePlayer"));
        assert!(!contents.contains("screenshot.png"));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
