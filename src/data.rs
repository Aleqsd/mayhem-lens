//! Versioned Mayhem data. Network access only occurs during catalog preparation or sync.
use crate::model::{
    Augment, AugmentStat, BuildRoute, Catalog, Champion, Item, ItemSynergy, Snapshot, Tier,
};
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SOURCE: &str = "ARAMKit / Mayhem / all brackets";
const ORIGIN: &str = "https://data.aramkit.com";
const MANIFEST_URL: &str = "https://data.aramkit.com/data/versions.json";
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;
const CACHE_SCHEMA: u32 = 1;

pub struct DataStore {
    cache: PathBuf,
    client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Release {
    patch: String,
    dataset: String,
    date: String,
}

#[derive(Serialize, Deserialize)]
struct CatalogCache {
    schema: u32,
    patch: String,
    catalog: Catalog,
}

#[derive(Serialize, Deserialize)]
struct SnapshotCache {
    schema: u32,
    snapshot: Snapshot,
}

impl DataStore {
    pub fn new(cache: PathBuf) -> Result<Self> {
        fs::create_dir_all(&cache).context("Créer le cache de données")?;
        let client = Client::builder()
            .user_agent(format!(
                "MayhemLens/{} (Windows desktop companion)",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { cache, client })
    }

    /// Loads the catalog for the last manifest, fetching metadata only when missing.
    pub fn catalog(&self) -> Result<Catalog> {
        let release = match read_json::<Release>(&self.cache.join("release.json")) {
            Ok(release) => {
                validate_release(&release)?;
                release
            }
            Err(_) => self.refresh_release()?,
        };
        self.prepare_catalog(&release.patch)
    }

    /// Refreshes the manifest and its catalog when a cached list cannot identify
    /// the current champion. Only call this from the data loading worker.
    pub fn refresh_catalog(&self) -> Result<Catalog> {
        let release = self.refresh_release()?;
        self.prepare_catalog(&release.patch)
    }

    /// Uses only the cached manifest and catalog; suitable for recognition paths.
    pub fn cached_catalog(&self) -> Result<Catalog> {
        let release: Release = read_json(&self.cache.join("release.json"))?;
        validate_release(&release)?;
        self.read_catalog(&release.patch)
    }

    /// One manifest request per sync; the champion payload is reused for an unchanged dataset.
    pub fn sync_champion(&self, id: u32) -> Result<Snapshot> {
        ensure!(id > 0, "Identifiant de champion invalide");
        let release = self.refresh_release()?;
        let catalog = self.prepare_catalog(&release.patch)?;
        ensure!(
            catalog.champions.iter().any(|c| c.id == id),
            "Champion absent du catalogue du patch"
        );
        if let Ok(snapshot) = self.cached_champion(id)
            && snapshot.dataset == release.dataset
            && snapshot.patch == release.patch
            && snapshot.dataset_date == release.date
        {
            return Ok(snapshot);
        }
        let url = format!(
            "{ORIGIN}/{}/stats/all/champion-details/{id}.json",
            release.dataset
        );
        let raw = self.get_json(&url)?;
        let snapshot = parse_snapshot(&raw, id, &release, &catalog, now_rfc3339()?)?;
        write_json(
            &self.champion_path(id),
            &SnapshotCache {
                schema: CACHE_SCHEMA,
                snapshot: snapshot.clone(),
            },
        )?;
        Ok(snapshot)
    }

    /// Strictly offline. Returns provenance and dates unchanged, including for an old cache.
    pub fn cached_champion(&self, id: u32) -> Result<Snapshot> {
        let entry: SnapshotCache = read_json(&self.champion_path(id))?;
        ensure!(
            entry.schema == CACHE_SCHEMA,
            "Version de cache incompatible"
        );
        let snapshot = entry.snapshot;
        ensure!(
            snapshot.champion_id == id,
            "Le cache appartient à un autre champion"
        );
        let release: Release = read_json(&self.cache.join("release.json"))?;
        validate_release(&release)?;
        validate_cached_patch(&snapshot, &release)?;
        let catalog = self.read_catalog(&snapshot.patch)?;
        validate_snapshot(&snapshot, &catalog)?;
        Ok(snapshot)
    }

    fn champion_path(&self, id: u32) -> PathBuf {
        self.cache.join("champions").join(format!("{id}.json"))
    }

    fn catalog_path(&self, patch: &str) -> Result<PathBuf> {
        validate_patch(patch)?;
        Ok(self.cache.join("catalogs").join(format!("{patch}.json")))
    }

    fn read_catalog(&self, patch: &str) -> Result<Catalog> {
        let entry: CatalogCache = read_json(&self.catalog_path(patch)?)?;
        ensure!(
            entry.schema == CACHE_SCHEMA && entry.patch == patch,
            "Catalogue de cache incompatible"
        );
        validate_catalog(&entry.catalog)?;
        Ok(entry.catalog)
    }

    fn refresh_release(&self) -> Result<Release> {
        let release = parse_release(&self.get_json(MANIFEST_URL)?)?;
        write_json(&self.cache.join("release.json"), &release)?;
        Ok(release)
    }

    fn prepare_catalog(&self, patch: &str) -> Result<Catalog> {
        if let Ok(catalog) = self.read_catalog(patch) {
            return Ok(catalog);
        }
        let base =
            format!("https://raw.communitydragon.org/{patch}/plugins/rcp-be-lol-game-data/global");
        let fr_champions = self.get_json(&format!("{base}/fr_fr/v1/champion-summary.json"))?;
        // CommunityDragon's default locale is English; en_us is not a published path.
        let en_champions = self.get_json(&format!("{base}/default/v1/champion-summary.json"))?;
        let fr_augments = self.get_json(&format!("{base}/fr_fr/v1/cherry-augments.json"))?;
        let en_augments = self.get_json(&format!("{base}/default/v1/cherry-augments.json"))?;
        let pools = self.get_json(&format!("{base}/fr_fr/v1/augment-lists.json"))?;
        let fr_items = self.get_json(&format!("{base}/fr_fr/v1/items.json"))?;
        let en_items = self.get_json(&format!("{base}/default/v1/items.json"))?;
        let catalog = parse_catalog(
            &fr_champions,
            &en_champions,
            &fr_augments,
            &en_augments,
            &pools,
            &fr_items,
            &en_items,
        )?;
        write_json(
            &self.catalog_path(patch)?,
            &CatalogCache {
                schema: CACHE_SCHEMA,
                patch: patch.to_owned(),
                catalog: catalog.clone(),
            },
        )?;
        Ok(catalog)
    }

    fn get_json(&self, url: &str) -> Result<Value> {
        let mut response = self
            .client
            .get(url)
            .send()
            .context("Télécharger les données publiques")?
            .error_for_status()
            .context("Réponse du fournisseur de données")?;
        if let Some(size) = response.content_length() {
            ensure!(
                size <= MAX_JSON_BYTES,
                "Réponse de données trop volumineuse"
            );
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(MAX_JSON_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_JSON_BYTES,
            "Réponse de données trop volumineuse"
        );
        serde_json::from_slice(&bytes).context("JSON du fournisseur invalide")
    }
}

fn parse_release(raw: &Value) -> Result<Release> {
    let latest = text(raw, "latest")?;
    let releases = rows(raw.get("versions"), "versions")?;
    let matching: Vec<_> = releases
        .iter()
        .filter(|v| v.get("version").and_then(Value::as_str) == Some(latest))
        .collect();
    ensure!(
        matching.len() == 1,
        "Version courante absente ou ambiguë dans le manifest"
    );
    let release = Release {
        patch: latest.to_owned(),
        dataset: text(matching[0], "dataPath")?.to_owned(),
        date: text(matching[0], "dataDate")?.to_owned(),
    };
    validate_release(&release)?;
    Ok(release)
}

fn validate_patch(patch: &str) -> Result<()> {
    let parts: Vec<_> = patch.split('.').collect();
    ensure!(
        parts.len() == 2
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit())),
        "Patch invalide"
    );
    Ok(())
}

fn validate_release(release: &Release) -> Result<()> {
    validate_patch(&release.patch)?;
    let suffix = release
        .dataset
        .strip_prefix(&format!("data/{}-", release.patch))
        .context("Dataset incompatible avec le patch")?;
    ensure!(
        !suffix.is_empty()
            && suffix.len() < 100
            && suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Chemin de dataset invalide"
    );
    let date = release.date.as_bytes();
    ensure!(
        date.len() == 10
            && date[4] == b'-'
            && date[7] == b'-'
            && date
                .iter()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()),
        "Date de dataset invalide"
    );
    Ok(())
}

fn parse_catalog(
    fr_champions: &Value,
    en_champions: &Value,
    fr_augments: &Value,
    en_augments: &Value,
    pools: &Value,
    fr_items: &Value,
    en_items: &Value,
) -> Result<Catalog> {
    let english_champions = id_index(en_champions)?;
    let english_augments = id_index(en_augments)?;
    let english_items = id_index(en_items)?;
    let kiwi: Vec<_> = rows(Some(pools), "pools")?
        .iter()
        .filter(|p| p.get("modeName").and_then(Value::as_str) == Some("KIWI"))
        .collect();
    ensure!(kiwi.len() == 1, "Pool KIWI/Mayhem absent ou ambigu");
    let pool_names: BTreeSet<_> = rows(kiwi[0].get("augmentList"), "augmentList")?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| s.strip_prefix("Maps/ModeSpecificData/Augments/"))
                .filter(|s| !s.is_empty())
                .context("Identifiant du pool Mayhem invalide")
        })
        .collect::<Result<_>>()?;
    ensure!(!pool_names.is_empty(), "Pool Mayhem vide");
    let mut champions = Vec::new();
    for fr in rows(Some(fr_champions), "champions")? {
        if fr
            .get("id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id <= 0)
        {
            continue;
        }
        let id = required_id(fr, "id")?;
        let en = english_champions
            .get(&id)
            .context("Traduction anglaise du champion absente")?;
        champions.push(Champion {
            id,
            key: text(en, "alias")?.to_owned(),
            name_fr: text(fr, "name")?.to_owned(),
            name_en: text(en, "name")?.to_owned(),
        });
    }
    let mut augments = Vec::new();
    for fr in rows(Some(fr_augments), "augments")? {
        let name_id = text(fr, "augmentNameId")?;
        if !pool_names.contains(name_id) {
            continue;
        }
        let id = required_id(fr, "id")?;
        let en = english_augments
            .get(&id)
            .context("Traduction anglaise de l'augmentation absente")?;
        ensure!(
            text(en, "augmentNameId")? == name_id,
            "Identités d'augmentation incompatibles entre langues"
        );
        augments.push(Augment {
            id,
            name_id: name_id.to_owned(),
            name_fr: text(fr, "nameTRA")?.to_owned(),
            name_en: text(en, "nameTRA")?.to_owned(),
            description_fr: optional_description(fr),
            description_en: optional_description(en),
        });
    }
    ensure!(
        augments.len() == pool_names.len(),
        "Catalogue Mayhem incomplet pour ce patch"
    );
    let mut items = Vec::new();
    for fr in rows(Some(fr_items), "items")? {
        let id = required_id(fr, "id")?;
        let en = english_items
            .get(&id)
            .context("Traduction anglaise de l'objet absente")?;
        // The static item table includes unused placeholders (e.g. an empty name).
        // They cannot produce a meaningful build label; a recommendation using
        // one will be rejected by the snapshot's catalog validation.
        if fr
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(str::is_empty)
            && en
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(str::is_empty)
        {
            continue;
        }
        items.push(Item {
            id,
            name_fr: text(fr, "name")?.to_owned(),
            name_en: text(en, "name")?.to_owned(),
        });
    }
    let catalog = Catalog {
        champions,
        augments,
        items,
    };
    validate_catalog(&catalog)?;
    Ok(catalog)
}

fn optional_description(row: &Value) -> String {
    row.get("descriptionTRA")
        .or_else(|| row.get("description"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn id_index(raw: &Value) -> Result<BTreeMap<u32, &Value>> {
    let mut index = BTreeMap::new();
    for row in rows(Some(raw), "catalogue")? {
        if row
            .get("id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id <= 0)
        {
            continue;
        }
        let id = required_id(row, "id")?;
        ensure!(
            index.insert(id, row).is_none(),
            "Identifiant dupliqué dans le catalogue"
        );
    }
    Ok(index)
}

fn validate_catalog(catalog: &Catalog) -> Result<()> {
    ensure!(
        !catalog.champions.is_empty() && !catalog.augments.is_empty() && !catalog.items.is_empty(),
        "Catalogue vide"
    );
    unique_ids(catalog.champions.iter().map(|c| c.id))?;
    unique_ids(catalog.augments.iter().map(|a| a.id))?;
    unique_ids(catalog.items.iter().map(|i| i.id))?;
    ensure!(
        catalog
            .champions
            .iter()
            .all(|c| !c.key.is_empty() && !c.name_fr.is_empty() && !c.name_en.is_empty()),
        "Nom de champion absent"
    );
    ensure!(
        catalog
            .augments
            .iter()
            .all(|a| !a.name_id.is_empty() && !a.name_fr.is_empty() && !a.name_en.is_empty()),
        "Nom d'augmentation absent"
    );
    ensure!(
        catalog
            .items
            .iter()
            .all(|i| !i.name_fr.is_empty() && !i.name_en.is_empty()),
        "Nom d'objet absent"
    );
    Ok(())
}

fn unique_ids(ids: impl IntoIterator<Item = u32>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        ensure!(
            id > 0 && seen.insert(id),
            "Identifiant invalide ou dupliqué"
        );
    }
    Ok(())
}

fn parse_snapshot(
    raw: &Value,
    champion_id: u32,
    release: &Release,
    catalog: &Catalog,
    fetched_at: String,
) -> Result<Snapshot> {
    ensure!(
        raw.pointer("/champion/id").and_then(Value::as_u64) == Some(u64::from(champion_id)),
        "Réponse associée à un autre champion"
    );
    let all = rows(raw.pointer("/augments/all"), "augmentations du champion")?;
    ensure!(
        !all.is_empty(),
        "Aucune recommandation Mayhem pour ce champion"
    );
    let augments = all
        .iter()
        .map(parse_augment_stat)
        .collect::<Result<Vec<_>>>()?;
    let mut stages = BTreeMap::new();
    if let Some(value) = raw.pointer("/augments/stages").filter(|v| !v.is_null()) {
        let stage_object = value.as_object().context("Splits Mayhem invalides")?;
        for (key, value) in stage_object {
            let stage = key.parse::<u8>().context("Stade fournisseur invalide")?;
            ensure!(
                (1..=4).contains(&stage),
                "Stade fournisseur hors du contrat Mayhem"
            );
            stages.insert(
                stage,
                rows(Some(value), "stade")?
                    .iter()
                    .map(parse_augment_stat)
                    .collect::<Result<Vec<_>>>()?,
            );
        }
    }
    let mut builds = Vec::new();
    let mut synergies: BTreeMap<(u32, u32), ItemSynergy> = BTreeMap::new();
    // Use the actual Mayhem champion payload, never a generic ARAM/Arena build endpoint.
    if let Some(archetypes) = raw.pointer("/builds/unfiltered/archetypes") {
        for archetype in rows(Some(archetypes), "archétypes de builds")? {
            for profile in rows(archetype.get("profiles"), "profils de builds")? {
                collect_synergies(profile.get("itemSet"), &mut synergies)?;
                for route in rows(profile.get("routes"), "routes de builds")? {
                    let purchase = rows(route.get("purchaseOrder"), "ordre d'achat")?;
                    let purchase_order = item_ids(purchase, false)?;
                    ensure!(
                        !purchase_order.is_empty() && purchase_order.len() <= 6,
                        "Ordre d'achat invalide"
                    );
                    for item in purchase {
                        collect_item_synergies(item, &mut synergies)?;
                    }
                    let starter = optional_rows(route.get("starters"), "objets de départ")?
                        .first()
                        .map(|v| item_ids(rows(v.get("items"), "objets de départ")?, false))
                        .transpose()?
                        .unwrap_or_default();
                    let boots = collect_options(route.get("boots"), &mut synergies)?;
                    let later_items = collect_options(route.get("laterItems"), &mut synergies)?;
                    builds.push(BuildRoute {
                        purchase_order,
                        sample_count: optional_u64(route, "sampleCount")?,
                        starters: starter,
                        boots,
                        later_items,
                    });
                }
            }
        }
    }
    let snapshot = Snapshot {
        champion_id,
        source: SOURCE.to_owned(),
        patch: release.patch.clone(),
        dataset: release.dataset.clone(),
        dataset_date: release.date.clone(),
        fetched_at,
        augments,
        stages,
        builds,
        item_synergies: synergies.into_values().collect(),
    };
    validate_snapshot(&snapshot, catalog)?;
    Ok(snapshot)
}

fn parse_augment_stat(raw: &Value) -> Result<AugmentStat> {
    // Deliberately never fall back to augmentTier: that is the GLOBAL grade.
    let tier = match raw.get("tier").and_then(Value::as_str) {
        Some("S") => Tier::S,
        Some("A") => Tier::A,
        Some("B") => Tier::B,
        Some("C") => Tier::C,
        Some("D") => Tier::D,
        Some("F") => Tier::F,
        _ => Tier::Unknown,
    };
    let rank = optional_u64(raw, "rank")?.map(u32::try_from).transpose()?;
    ensure!(rank != Some(0), "Rang d'augmentation invalide");
    Ok(AugmentStat {
        id: required_id(raw, "id")?,
        tier,
        rank,
        sample_count: optional_u64(raw, "sampleCount")?,
    })
}

fn collect_options(
    raw: Option<&Value>,
    synergies: &mut BTreeMap<(u32, u32), ItemSynergy>,
) -> Result<Vec<u32>> {
    let mut options = Vec::new();
    for row in optional_rows(raw, "options de builds")? {
        let item = row.get("item").context("Objet d'option absent")?;
        let id = optional_u64(item, "id")?.context("Identifiant d'option absent")?;
        // Provider's item 0 means no boots, not a real purchasable item.
        if id == 0 {
            continue;
        }
        let id = u32::try_from(id)?;
        if !options.contains(&id) {
            options.push(id);
        }
        collect_item_synergies(item, synergies)?;
    }
    Ok(options)
}

fn collect_synergies(
    raw: Option<&Value>,
    synergies: &mut BTreeMap<(u32, u32), ItemSynergy>,
) -> Result<()> {
    for item in optional_rows(raw, "objets avec synergies")? {
        collect_item_synergies(item, synergies)?;
    }
    Ok(())
}

fn collect_item_synergies(
    item: &Value,
    synergies: &mut BTreeMap<(u32, u32), ItemSynergy>,
) -> Result<()> {
    let item_id = required_id(item, "id")?;
    for raw in optional_rows(
        item.get("augmentSpecific"),
        "associations objet/augmentation",
    )? {
        let augment_id = required_id(raw, "id")?;
        let sample_count = optional_u64(raw, "sampleCount")?;
        let key = (item_id, augment_id);
        // Repeated copies describe the same association, not independent samples to sum.
        if synergies
            .get(&key)
            .is_none_or(|previous| previous.sample_count < sample_count)
        {
            synergies.insert(
                key,
                ItemSynergy {
                    item_id,
                    augment_id,
                    sample_count,
                },
            );
        }
    }
    Ok(())
}

fn item_ids(raw: &[Value], allow_zero: bool) -> Result<Vec<u32>> {
    raw.iter()
        .map(|item| {
            let id = optional_u64(item, "id")?.context("Identifiant d'objet absent")?;
            ensure!(allow_zero || id > 0, "Identifiant d'objet nul");
            Ok(u32::try_from(id)?)
        })
        .collect()
}

fn validate_snapshot(snapshot: &Snapshot, catalog: &Catalog) -> Result<()> {
    ensure!(snapshot.source == SOURCE, "Source du cache incompatible");
    validate_release(&Release {
        patch: snapshot.patch.clone(),
        dataset: snapshot.dataset.clone(),
        date: snapshot.dataset_date.clone(),
    })?;
    ensure!(
        catalog
            .champions
            .iter()
            .any(|c| c.id == snapshot.champion_id),
        "Champion absent du catalogue"
    );
    ensure!(
        !snapshot.fetched_at.is_empty() && !snapshot.augments.is_empty(),
        "Snapshot incomplet"
    );
    let allowed_augments: BTreeSet<_> = catalog.augments.iter().map(|a| a.id).collect();
    let allowed_items: BTreeSet<_> = catalog.items.iter().map(|i| i.id).collect();
    for stats in std::iter::once(&snapshot.augments).chain(snapshot.stages.values()) {
        unique_ids(stats.iter().map(|a| a.id))?;
        ensure!(
            stats
                .iter()
                .all(|a| allowed_augments.contains(&a.id) && a.rank != Some(0)),
            "Augmentation hors du pool KIWI ou rang invalide"
        );
    }
    ensure!(
        snapshot.stages.keys().all(|s| (1..=4).contains(s)),
        "Stade invalide dans le cache"
    );
    for build in &snapshot.builds {
        ensure!(
            !build.purchase_order.is_empty() && build.purchase_order.len() <= 6,
            "Build incomplet dans le cache"
        );
        ensure!(
            build
                .purchase_order
                .iter()
                .chain(&build.starters)
                .chain(&build.boots)
                .chain(&build.later_items)
                .all(|id| allowed_items.contains(id)),
            "Objet de build absent du catalogue du patch"
        );
    }
    ensure!(snapshot.item_synergies.iter().all(|s| allowed_items.contains(&s.item_id) && allowed_augments.contains(&s.augment_id)), "Association objet/augmentation hors du catalogue Mayhem");
    Ok(())
}

fn validate_cached_patch(snapshot: &Snapshot, release: &Release) -> Result<()> {
    ensure!(
        snapshot.patch == release.patch,
        "Le cache du champion appartient à un ancien patch"
    );
    Ok(())
}

fn rows<'a>(value: Option<&'a Value>, label: &str) -> Result<&'a [Value]> {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .with_context(|| format!("Tableau {label} absent ou invalide"))
}

fn optional_rows<'a>(value: Option<&'a Value>, label: &str) -> Result<&'a [Value]> {
    match value {
        None | Some(Value::Null) => Ok(&[]),
        Some(value) => rows(Some(value), label),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .with_context(|| format!("Champ texte {key} absent ou invalide"))
}

fn optional_u64(value: &Value, key: &str) -> Result<Option<u64>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(
            v.as_u64()
                .with_context(|| format!("Champ entier {key} invalide"))?,
        )),
    }
}

fn required_id(value: &Value, key: &str) -> Result<u32> {
    let id = optional_u64(value, key)?.context("Identifiant absent")?;
    ensure!(id > 0, "Identifiant nul");
    Ok(u32::try_from(id)?)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::metadata(path).context("Cache indisponible")?;
    ensure!(
        metadata.len() <= MAX_JSON_BYTES,
        "Fichier de cache trop volumineux"
    );
    serde_json::from_slice(&fs::read(path)?).context("Cache JSON invalide")
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("Dossier de cache absent")?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() as u64 <= MAX_JSON_BYTES,
        "Cache trop volumineux"
    );
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).context("Activer le cache complet")
}

fn now_rfc3339() -> Result<String> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(timestamp_rfc3339(seconds))
}

fn timestamp_rfc3339(seconds: u64) -> String {
    // Gregorian civil date conversion, avoiding a runtime/timezone dependency.
    let days = (seconds / 86400) as i64 + 719468;
    let era = days / 146097;
    let day_of_era = days - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let clock = seconds % 86400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release() -> Release {
        Release {
            patch: "16.19".into(),
            dataset: "data/16.19-fixture".into(),
            date: "2026-10-04".into(),
        }
    }
    fn catalog() -> Catalog {
        Catalog {
            champions: vec![Champion {
                id: 1,
                key: "Example".into(),
                name_fr: "Exemple".into(),
                name_en: "Example".into(),
            }],
            augments: vec![Augment {
                id: 20,
                name_id: "ExampleAugment".into(),
                name_fr: "Exemple".into(),
                name_en: "Example".into(),
                description_fr: String::new(),
                description_en: String::new(),
            }],
            items: vec![10, 11]
                .into_iter()
                .map(|id| Item {
                    id,
                    name_fr: "Objet".into(),
                    name_en: "Item".into(),
                })
                .collect(),
        }
    }
    fn payload() -> Value {
        json!({"champion":{"id":1},"augments":{"all":[{"id":20,"tier":"B","augmentTier":"S","rank":2,"sampleCount":500}],"stages":{"2":[{"id":20,"tier":"C","rank":4,"sampleCount":100}]}},"builds":{"unfiltered":{"archetypes":[{"profiles":[{"routes":[{"purchaseOrder":[{"id":10}],"sampleCount":90,"starters":[{"items":[{"id":11}]}],"boots":[{"item":{"id":0}}],"laterItems":[{"item":{"id":11,"augmentSpecific":[{"id":20,"sampleCount":30}]}}]}]}]}]}}})
    }

    #[test]
    fn keeps_champion_and_stage_grades_separate_from_global_grade() {
        let snapshot = parse_snapshot(
            &payload(),
            1,
            &release(),
            &catalog(),
            "2026-10-04T00:00:00Z".into(),
        )
        .unwrap();
        assert_eq!(snapshot.augments[0].tier, Tier::B);
        assert_eq!(snapshot.stages[&2][0].tier, Tier::C);
        assert!(!snapshot.stages.contains_key(&1));
        assert!(snapshot.builds[0].boots.is_empty());
        assert_eq!(snapshot.item_synergies[0].augment_id, 20);
    }

    #[test]
    fn missing_champion_tier_stays_unknown() {
        let stat =
            parse_augment_stat(&json!({"id":20,"tier":null,"augmentTier":"S","sampleCount":null}))
                .unwrap();
        assert_eq!(stat.tier, Tier::Unknown);
        assert_eq!(stat.sample_count, None);
    }

    #[test]
    fn shared_catalog_is_filtered_by_the_kiwi_pool_not_by_name_prefix() {
        let champions = json!([{"id":1,"alias":"Example","name":"Example"}]);
        let augments = json!([
            {"id":20,"augmentNameId":"SharedExample","nameTRA":"Mayhem example"},
            {"id":30,"augmentNameId":"ARAM_ArenaExample","nameTRA":"Arena example"}
        ]);
        let pools = json!([
            {"modeName":"KIWI","augmentList":["Maps/ModeSpecificData/Augments/SharedExample"]},
            {"modeName":"CHERRY","augmentList":["Maps/ModeSpecificData/Augments/ARAM_ArenaExample"]}
        ]);
        let items = json!([{"id":10,"name":"Example item"},{"id":999,"name":""}]);
        let parsed = parse_catalog(
            &champions, &champions, &augments, &augments, &pools, &items, &items,
        )
        .unwrap();
        assert_eq!(parsed.augments.len(), 1);
        assert_eq!(parsed.augments[0].id, 20);
        assert!(parsed.augments[0].description_fr.is_empty());
        assert_eq!(parsed.items.len(), 1);
    }

    #[test]
    fn absent_stage_stats_do_not_fabricate_stage_grades() {
        let mut raw = payload();
        raw["augments"].as_object_mut().unwrap().remove("stages");
        let snapshot = parse_snapshot(
            &raw,
            1,
            &release(),
            &catalog(),
            "2026-10-04T00:00:00Z".into(),
        )
        .unwrap();
        assert!(snapshot.stages.is_empty());
        assert_eq!(snapshot.augments[0].tier, Tier::B);
    }

    #[test]
    fn rejects_other_champions_and_non_mayhem_augments() {
        assert!(parse_snapshot(&payload(), 2, &release(), &catalog(), "now".into()).is_err());
        let mut value = payload();
        value["augments"]["all"][0]["id"] = json!(999);
        assert!(parse_snapshot(&value, 1, &release(), &catalog(), "now".into()).is_err());
    }

    #[test]
    fn rejects_unsafe_or_cross_patch_dataset_paths() {
        assert!(
            validate_release(&Release {
                dataset: "data/16.18-fixture".into(),
                ..release()
            })
            .is_err()
        );
        assert!(
            validate_release(&Release {
                dataset: "data/16.19-../../secret".into(),
                ..release()
            })
            .is_err()
        );
    }

    #[test]
    fn old_champion_cache_cannot_be_used_with_a_new_patch_catalog() {
        let snapshot = parse_snapshot(
            &payload(),
            1,
            &release(),
            &catalog(),
            "2026-10-04T00:00:00Z".into(),
        )
        .unwrap();
        assert!(validate_cached_patch(&snapshot, &release()).is_ok());
        let newer = Release {
            patch: "16.20".into(),
            dataset: "data/16.20-fixture".into(),
            date: "2026-10-05".into(),
        };
        assert!(validate_cached_patch(&snapshot, &newer).is_err());
    }

    #[test]
    fn timestamps_are_utc() {
        assert_eq!(timestamp_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp_rfc3339(1791072000), "2026-10-04T00:00:00Z");
    }
}
