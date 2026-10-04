//! Runtime coordination. Network/game polling and OCR run on separate workers.
use crate::{
    config::{Config, app_directory},
    data::DataStore,
    domain, game,
    model::{Catalog, Snapshot},
    native::{self, Badge, Observation, Rect, UserAction},
};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
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

pub fn run(cache: PathBuf, config_path: PathBuf) -> Result<()> {
    // Fail before starting workers if package identity/OCR support is unavailable.
    if !config_path.exists() {
        Config::default().save(&config_path)?;
    }
    native::ensure_ready(&Config::load(&config_path)?.language)?;
    let _instance = native::acquire_single_instance()?;
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
    let result = native::run_overlay(badges_rx, actions_tx, Arc::clone(&stop), &config_path);
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
                let changed = champion != game.champion_key || game.game_time + 10.0 < last_time;
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
                        let id = catalog
                            .champions
                            .iter()
                            .find(|c| {
                                c.key.eq_ignore_ascii_case(&game.champion_key)
                                    || c.name_en == game.champion_name
                                    || c.name_fr == game.champion_name
                            })
                            .map(|c| c.id)
                            .context("Identifiant du champion absent du catalogue")?;
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
                                .map_err(|_| anyhow::anyhow!("Verrou session"))? = prepared.clone();
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
            if generation != Some(active.generation) {
                generation = Some(active.generation);
                previous.clear();
                confirmed_cards = None;
                config.selected_augments.clear();
                config.offer_stage = None;
                config.save(&config_path)?;
            }
            if native::game_window_visible()
                && (force || last_scan.elapsed() >= Duration::from_millis(config.scan_interval_ms))
            {
                last_scan = Instant::now();
                match native::observe_game(&config.language) {
                    Ok(observations) => {
                        let offers = detect_offers(&observations, &active.matcher);
                        let current_generation = session
                            .read()
                            .ok()
                            .and_then(|s| s.as_ref().map(|s| s.generation));
                        if offers.len() == 3 && current_generation == Some(active.generation) {
                            last_valid = Instant::now();
                            previous = offers;
                            let badges = present(&active, &previous, &config);
                            if sender.send(badges).is_err() {
                                break;
                            }
                        } else {
                            previous.clear();
                            confirmed_cards = None;
                            let _ = sender.send(Vec::new());
                        }
                    }
                    Err(error) => {
                        previous.clear();
                        let _ = sender.send(Vec::new());
                        write_status(status, "ocr-error", &format!("{error:#}"));
                    }
                }
            } else if !native::game_window_visible() {
                previous.clear();
                let _ = sender.send(Vec::new());
            }
        } else {
            previous.clear();
            let _ = sender.send(Vec::new());
        }
        sleep_stoppable(stop, Duration::from_millis(80));
    }
    let _ = sender.send(Vec::new());
    Ok(())
}

fn detect_offers(observations: &[Observation], matcher: &domain::AugmentMatcher) -> Vec<Offer> {
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
            return row;
        }
    }
    Vec::new()
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
    use crate::model::Augment;
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
        assert_eq!(detect_offers(&offers, &matcher).len(), 3);
        assert!(detect_offers(&offers[..2], &matcher).is_empty());
        let list = [
            observation("Spark", 100, 200),
            observation("River", 100, 400),
            observation("Cloud", 100, 600),
        ];
        assert!(detect_offers(&list, &matcher).is_empty());
    }
    #[test]
    fn duplicate_or_uneven_layout_is_rejected() {
        let offers = [
            observation("Spark", 100, 500),
            observation("Spark", 400, 500),
            observation("Cloud", 700, 500),
        ];
        let matcher = domain::AugmentMatcher::new(&catalog());
        assert!(detect_offers(&offers, &matcher).is_empty());
        let offers = [
            observation("Spark", 100, 500),
            observation("River", 400, 500),
            observation("Spark", 700, 500),
        ];
        assert!(detect_offers(&offers, &matcher).is_empty());
        let offers = [
            observation("Spark", 100, 500),
            observation("River", 220, 500),
            observation("Cloud", 1500, 500),
        ];
        assert!(detect_offers(&offers, &matcher).is_empty());
    }
}
