//! Pure local decisions: no network, OCR engine, or window access.

use crate::model::{AugmentStat, Catalog, Snapshot, Tier};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub id: u32,
    /// Name similarity, not a calibrated probability of OCR correctness.
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    pub augment_id: u32,
    pub tier: Tier,
    pub explanation: String,
    pub synergies: Vec<String>,
    pub sample_count: Option<u64>,
    pub stage_specific: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildAdvice {
    pub item_ids: Vec<u32>,
    pub label: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynergyRule {
    pub id: String,
    pub requires: Vec<u32>,
    pub offered: u32,
    pub explanation_fr: String,
    pub explanation_en: String,
}

#[derive(Debug, Clone, Default)]
pub struct AugmentMatcher {
    candidates: Vec<PreparedAugment>,
}

#[derive(Debug, Clone)]
struct PreparedAugment {
    id: u32,
    names: Vec<PreparedName>,
}

#[derive(Debug, Clone)]
struct PreparedName {
    text: String,
    characters: Vec<char>,
    significant_length: usize,
}

fn normalize(text: &str) -> String {
    let mut result = String::new();
    for character in text.nfkd().filter(|c| !is_combining_mark(*c)) {
        for lower in character.to_lowercase() {
            match lower {
                'œ' => result.push_str("oe"),
                'æ' => result.push_str("ae"),
                c if c.is_alphanumeric() => result.push(c),
                _ if !result.ends_with(' ') && !result.is_empty() => result.push(' '),
                _ => {}
            }
        }
    }
    result.trim_end().to_owned()
}

/// Match the whole title region, including titles wrapped over multiple lines.
/// Do not search for a name inside arbitrary OCR text or an effect description.
pub fn match_augment(text: &str, catalog: &Catalog) -> Option<Match> {
    AugmentMatcher::new(catalog).match_text(text)
}

impl AugmentMatcher {
    /// Prepare once when the catalog changes; reuse for every OCR title.
    pub fn new(catalog: &Catalog) -> Self {
        let candidates = catalog
            .augments
            .iter()
            .map(|augment| {
                let mut names: Vec<_> = [&augment.name_fr, &augment.name_en]
                    .into_iter()
                    .map(|name| normalize(name))
                    .filter(|name| !name.is_empty())
                    .collect();
                names.dedup();
                PreparedAugment {
                    id: augment.id,
                    names: names
                        .into_iter()
                        .map(|text| PreparedName {
                            significant_length: text
                                .chars()
                                .filter(|c| c.is_alphanumeric())
                                .count(),
                            characters: text.chars().collect(),
                            text,
                        })
                        .collect(),
                }
            })
            .collect();
        Self { candidates }
    }

    pub fn match_text(&self, text: &str) -> Option<Match> {
        if text.chars().take(257).count() > 256 {
            return None;
        }
        let text = normalize(text);
        if text.is_empty() {
            return None;
        }
        let exact_ids: BTreeSet<_> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.names.iter().any(|name| name.text == text))
            .map(|candidate| candidate.id)
            .collect();
        if !exact_ids.is_empty() {
            return (exact_ids.len() == 1).then(|| Match {
                id: *exact_ids.first().expect("one exact match"),
                confidence: 1.0,
            });
        }
        // Very short names do not provide enough information for a typo guess.
        if text.chars().filter(|c| c.is_alphanumeric()).count() < 8 {
            return None;
        }
        let query: Vec<_> = text.chars().collect();
        let mut scored = Vec::new();
        for candidate in &self.candidates {
            let mut best: Option<(f32, usize, usize)> = None;
            for name in &candidate.names {
                if name.significant_length < 8 {
                    continue;
                }
                let length = query.len().max(name.characters.len());
                // Acceptance starts at .90, with a .06 ambiguity margin. Keep
                // near misses down to .84, but skip distant lengths immediately.
                let ambiguity_budget = (length * 16).div_ceil(100).max(2);
                let Some(distance) =
                    bounded_edit_distance(&query, &name.characters, ambiguity_budget)
                else {
                    continue;
                };
                let score = 1.0 - distance as f32 / length as f32;
                if best.is_none_or(|previous| score > previous.0) {
                    best = Some((score, distance, length));
                }
            }
            if let Some((score, distance, length)) = best {
                scored.push((candidate.id, score, distance, length));
            }
        }
        scored.sort_by(|left, right| right.1.total_cmp(&left.1));
        let &(id, confidence, distance, length) = scored.first()?;
        let budget = if length >= 20 { 2 } else { 1 };
        if distance > budget || confidence < 0.90 {
            return None;
        }
        if scored
            .iter()
            .find(|candidate| candidate.0 != id)
            .is_some_and(|candidate| confidence - candidate.1 < 0.06)
        {
            return None;
        }
        Some(Match { id, confidence })
    }
}

/// Banded optimal string alignment distance; a transposition counts as one edit.
fn bounded_edit_distance(left: &[char], right: &[char], limit: usize) -> Option<usize> {
    if left.len().abs_diff(right.len()) > limit {
        return None;
    }
    let infinity = limit + 1;
    let mut previous_two = vec![infinity; right.len() + 1];
    let mut previous: Vec<_> = (0..=right.len()).map(|value| value.min(infinity)).collect();
    let mut current = vec![infinity; right.len() + 1];
    for (i, left_char) in left.iter().enumerate() {
        current.fill(infinity);
        current[0] = (i + 1).min(infinity);
        let start = (i + 1).saturating_sub(limit).max(1);
        let end = (i + 1 + limit).min(right.len());
        for j in start..=end {
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + usize::from(*left_char != right[j - 1]))
                .min(infinity);
            if i > 0 && j > 1 && *left_char == right[j - 2] && left[i - 1] == right[j - 1] {
                current[j] = current[j].min(previous_two[j - 2] + 1);
            }
        }
        std::mem::swap(&mut previous_two, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[right.len()] <= limit).then_some(previous[right.len()])
}

pub fn recommend(
    snapshot: &Snapshot,
    catalog: &Catalog,
    offers: &[u32],
    selected: &[u32],
    stage: Option<u8>,
) -> Vec<Recommendation> {
    recommend_localized(snapshot, catalog, offers, selected, stage, "fr")
}

/// Preserve offer order, source grades, and an explicitly unknown stage grade.
pub fn recommend_localized(
    snapshot: &Snapshot,
    catalog: &Catalog,
    offers: &[u32],
    _selected: &[u32],
    stage: Option<u8>,
    language: &str,
) -> Vec<Recommendation> {
    let english = is_english(language);
    offers
        .iter()
        .map(|&id| {
            let augment = catalog.augments.iter().find(|augment| augment.id == id);
            let stage_stat = stage.and_then(|stage| {
                snapshot
                    .stages
                    .get(&stage)?
                    .iter()
                    .find(|stat| stat.id == id)
            });
            let stat = augment.and_then(|_| {
                stage_stat.or_else(|| snapshot.augments.iter().find(|stat| stat.id == id))
            });
            let stage_specific = stat.is_some() && stage_stat.is_some();
            Recommendation {
                augment_id: id,
                tier: stat.map_or(Tier::Unknown, |stat| stat.tier),
                explanation: explain(snapshot, catalog, stat, stage, stage_specific, english),
                synergies: Vec::new(),
                sample_count: stat.and_then(|stat| stat.sample_count),
                stage_specific,
            }
        })
        .collect()
}

fn explain(
    snapshot: &Snapshot,
    catalog: &Catalog,
    stat: Option<&AugmentStat>,
    stage: Option<u8>,
    stage_specific: bool,
    english: bool,
) -> String {
    let Some(stat) = stat else {
        return choose(english, "Classement indisponible.", "No rating available.").to_owned();
    };
    if stat.tier == Tier::Unknown {
        return choose(
            english,
            "La source ne fournit pas de tier pour ce choix.",
            "The source provides no tier for this choice.",
        )
        .to_owned();
    }
    let champion = catalog
        .champions
        .iter()
        .find(|champion| champion.id == snapshot.champion_id)
        .map(|champion| choose(english, &champion.name_fr, &champion.name_en).to_owned())
        .unwrap_or_else(|| format!("#{}", snapshot.champion_id));
    let context = match (stage, stage_specific, english) {
        (Some(stage), true, false) => format!("étape {stage}"),
        (Some(stage), true, true) => format!("stage {stage}"),
        (Some(stage), false, false) => format!("étape {stage} indisponible ; tier champion"),
        (Some(stage), false, true) => format!("stage {stage} unavailable; champion tier"),
        (None, _, false) => "tier champion ; étape inconnue".to_owned(),
        (None, _, true) => "champion tier; stage unknown".to_owned(),
    };
    format!(
        "{} · {} · {} · {}.",
        snapshot.source,
        champion,
        context,
        sample_label(stat.sample_count, english)
    )
}

/// Apply explicitly configured personal rules, without changing source grades.
/// No rules are bundled: a name pair alone is not verified mechanical evidence.
pub fn apply_rules(
    recommendations: &mut [Recommendation],
    selected: &[u32],
    rules: &[SynergyRule],
    language: &str,
) {
    let english = is_english(language);
    let selected: BTreeSet<_> = selected.iter().copied().collect();
    for recommendation in recommendations {
        for rule in rules {
            let explanation = choose(english, &rule.explanation_fr, &rule.explanation_en).trim();
            if rule.offered != recommendation.augment_id
                || rule.id.trim().is_empty()
                || explanation.is_empty()
                || !rule.requires.iter().all(|id| selected.contains(id))
            {
                continue;
            }
            let note = format!(
                "{} [{}] · {}",
                choose(english, "Règle personnelle", "Personal rule"),
                rule.id,
                explanation
            );
            if !recommendation.synergies.contains(&note) {
                recommendation.synergies.push(note);
            }
        }
    }
}

pub fn build_advice(snapshot: &Snapshot, catalog: &Catalog, selected: &[u32]) -> Vec<BuildAdvice> {
    build_advice_localized(snapshot, catalog, selected, "fr")
}

pub fn build_advice_localized(
    snapshot: &Snapshot,
    catalog: &Catalog,
    selected: &[u32],
    language: &str,
) -> Vec<BuildAdvice> {
    let english = is_english(language);
    let mut advice = Vec::new();
    for (index, route) in snapshot.builds.iter().enumerate() {
        let route_number = index + 1;
        if !route.purchase_order.is_empty() {
            advice.push(BuildAdvice {
                item_ids: route.purchase_order.clone(),
                label: format!(
                    "{} {route_number}",
                    choose(english, "Route Mayhem observée", "Observed Mayhem route")
                ),
                reason: format!(
                    "{} · {} · {}{}.",
                    snapshot.source,
                    choose(english, "Ordre d’achat observé", "Observed purchase order"),
                    sample_label(route.sample_count, english),
                    catalog_warning(&route.purchase_order, catalog, english)
                ),
            });
        }
        // These fields are alternatives attached to a route, not its measured
        // purchase order. Route sample counts cannot be transferred to options.
        for (items, french, english_label) in [
            (&route.starters, "Objets de départ", "Starting items"),
            (&route.boots, "Options de bottes", "Boot options"),
            (
                &route.later_items,
                "Options de fin de build",
                "Later item options",
            ),
        ] {
            if items.is_empty() {
                continue;
            }
            advice.push(BuildAdvice {
                item_ids: unique_ids(items),
                label: format!("{} · {route_number}", choose(english, french, english_label)),
                reason: format!(
                    "{}{}.",
                    choose(
                        english,
                        "Options associées à cette route par la source ; ce n’est pas un ordre d’achat complet observé",
                        "Source options attached to this route; not a complete observed purchase order"
                    ),
                    catalog_warning(items, catalog, english)
                ),
            });
        }
    }
    let mut seen = BTreeSet::new();
    for association in &snapshot.item_synergies {
        if !selected.contains(&association.augment_id)
            || association.sample_count == Some(0)
            || !seen.insert((association.item_id, association.augment_id))
        {
            continue;
        }
        let Some(augment) = catalog
            .augments
            .iter()
            .find(|augment| augment.id == association.augment_id)
        else {
            continue;
        };
        let Some(item) = catalog
            .items
            .iter()
            .find(|item| item.id == association.item_id)
        else {
            continue;
        };
        advice.push(BuildAdvice {
            item_ids: vec![association.item_id],
            label: format!(
                "{} · {} × {}",
                choose(english, "Association observée", "Observed association"),
                choose(english, &item.name_fr, &item.name_en),
                choose(english, &augment.name_fr, &augment.name_en),
            ),
            reason: format!(
                "{} · {} · {}.",
                snapshot.source,
                sample_label(association.sample_count, english),
                choose(
                    english,
                    "Association objet × augmentation ; ne prouve pas un bénéfice causal",
                    "Item × augment association; does not establish a causal benefit"
                )
            ),
        });
    }
    advice
}

fn unique_ids(ids: &[u32]) -> Vec<u32> {
    let mut seen = BTreeSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

fn catalog_warning(ids: &[u32], catalog: &Catalog, english: bool) -> &'static str {
    if ids
        .iter()
        .any(|id| !catalog.items.iter().any(|item| item.id == *id))
    {
        choose(
            english,
            " ; catalogue incomplet",
            "; incomplete item catalog",
        )
    } else {
        ""
    }
}

fn sample_label(count: Option<u64>, english: bool) -> String {
    count.map_or_else(
        || choose(english, "effectif indisponible", "sample count unavailable").to_owned(),
        |count| {
            format!(
                "{count} {}",
                choose(english, "observations", "observations")
            )
        },
    )
}

fn choose<'a>(english: bool, french: &'a str, english_text: &'a str) -> &'a str {
    if english { english_text } else { french }
}

fn is_english(language: &str) -> bool {
    language
        .split(['-', '_'])
        .next()
        .is_some_and(|code| code.eq_ignore_ascii_case("en"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Augment, BuildRoute, Champion, Item, ItemSynergy};
    use std::collections::BTreeMap;

    fn augment(id: u32, name_id: &str, french: &str, english: &str) -> Augment {
        Augment {
            id,
            name_id: name_id.to_owned(),
            name_fr: french.to_owned(),
            name_en: english.to_owned(),
            description_fr: "Des effets synthétiques pour ce test.".to_owned(),
            description_en: "Synthetic effect descriptions for this test.".to_owned(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            champions: vec![Champion {
                id: 63,
                key: "Brand".to_owned(),
                name_fr: "Brand".to_owned(),
                name_en: "Brand".to_owned(),
            }],
            augments: vec![
                augment(
                    1045,
                    "ARAM_InfernalConduit",
                    "Conduit infernal",
                    "Infernal Conduit",
                ),
                augment(
                    1048,
                    "ARAM_JeweledGauntlet",
                    "Gantelet précieux",
                    "Jeweled Gauntlet",
                ),
                augment(1092, "ARAM_Vulnerability", "Vulnérabilité", "Vulnerability"),
                augment(1041, "ARAM_Goliath", "Goliath", "Goliath"),
                augment(
                    1390,
                    "ARAM_PhenomenalEvil",
                    "Pouvoir maléfique phénoménal",
                    "Phenomenal Evil",
                ),
            ],
            items: (1..=6)
                .map(|id| Item {
                    id,
                    name_fr: format!("Objet {id}"),
                    name_en: format!("Item {id}"),
                })
                .collect(),
        }
    }

    fn stat(id: u32, tier: Tier, count: Option<u64>) -> AugmentStat {
        AugmentStat {
            id,
            tier,
            rank: None,
            sample_count: count,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            champion_id: 63,
            source: "Fournisseur test".to_owned(),
            patch: "16.19".to_owned(),
            dataset: "synthetic".to_owned(),
            dataset_date: "2026-10-04".to_owned(),
            fetched_at: "2026-10-04T10:00:00Z".to_owned(),
            augments: vec![stat(1045, Tier::A, Some(100)), stat(1048, Tier::B, None)],
            stages: BTreeMap::from([
                (1, vec![stat(1045, Tier::S, Some(8))]),
                (3, vec![stat(1045, Tier::Unknown, None)]),
            ]),
            builds: Vec::new(),
            item_synergies: Vec::new(),
        }
    }

    #[test]
    fn recognizes_both_languages_and_unicode_titles() {
        let catalog = catalog();
        for title in [
            "  GANTELET PRÉCIEUX  ",
            "Gantelet pre\u{301}cieux",
            "Jeweled Gauntlet",
        ] {
            assert_eq!(
                match_augment(title, &catalog),
                Some(Match {
                    id: 1048,
                    confidence: 1.0
                })
            );
        }
        assert_eq!(
            match_augment("Pouvoir maléfique\nphénoménal", &catalog)
                .unwrap()
                .id,
            1390
        );
    }

    #[test]
    fn tolerates_one_transposition_but_rejects_short_guesses_and_descriptions() {
        let catalog = catalog();
        let found = match_augment("Gantelet preiceux", &catalog).unwrap();
        assert_eq!(found.id, 1048);
        assert!(found.confidence >= 0.9 && found.confidence < 1.0);
        for title in [
            "Goliat",
            "",
            "!!!",
            "Your abilities receive Jeweled Gauntlet effects.",
            "Jeweled Gauntlet\nYour abilities critically strike.",
        ] {
            assert!(match_augment(title, &catalog).is_none(), "{title}");
        }
    }

    #[test]
    fn rejects_fuzzy_ties_and_exact_names_shared_by_distinct_ids() {
        let mut catalog = Catalog {
            augments: vec![
                augment(1, "First", "Amélioration divine", "Divine upgrade"),
                augment(2, "Second", "Amélioration devine", "Different upgrade"),
            ],
            ..Catalog::default()
        };
        assert!(match_augment("Amélioration dovine", &catalog).is_none());
        catalog.augments[1].name_fr = catalog.augments[0].name_fr.clone();
        assert!(match_augment("Amélioration divine", &catalog).is_none());
    }

    #[test]
    fn ligatures_and_punctuation_do_not_change_the_catalog_identity() {
        let mut catalog = Catalog::default();
        catalog
            .augments
            .push(augment(7, "Synthetic", "Puissance d’œuf", "Egg power"));
        assert_eq!(match_augment("PUISSANCE D'OEuf", &catalog).unwrap().id, 7);
    }

    #[test]
    fn uses_stage_only_when_known_and_preserves_explicit_unknown() {
        let snapshot = snapshot();
        let catalog = catalog();
        for (stage, tier, count, specific) in [
            (None, Tier::A, Some(100), false),
            (Some(1), Tier::S, Some(8), true),
            (Some(2), Tier::A, Some(100), false),
            (Some(3), Tier::Unknown, None, true),
        ] {
            let result = recommend(&snapshot, &catalog, &[1045], &[], stage);
            assert_eq!(result[0].tier, tier);
            assert_eq!(result[0].sample_count, count);
            assert_eq!(result[0].stage_specific, specific);
        }
    }

    #[test]
    fn preserves_offer_positions_and_does_not_rate_unknown_augments() {
        let mut snapshot = snapshot();
        snapshot.augments.push(stat(999, Tier::S, Some(1_000)));
        let results = recommend(&snapshot, &catalog(), &[1048, 999, 1045, 1048], &[], None);
        assert_eq!(
            results.iter().map(|r| r.augment_id).collect::<Vec<_>>(),
            [1048, 999, 1045, 1048]
        );
        assert_eq!(results[1].tier, Tier::Unknown);
        assert_eq!(results[1].sample_count, None);
        assert!(results[1].synergies.is_empty());
    }

    #[test]
    fn personal_rules_require_all_conditions_and_never_regrade_the_source() {
        let catalog = catalog();
        let snapshot = snapshot();
        let mut result = recommend(&snapshot, &catalog, &[1045, 1048], &[1092], None);
        assert!(result.iter().all(|entry| entry.synergies.is_empty()));
        let rules = vec![SynergyRule {
            id: "synthetic-demo".to_owned(),
            requires: vec![1092, 1048],
            offered: 1045,
            explanation_fr: "Note synthétique personnalisée, sans preuve statistique.".to_owned(),
            explanation_en: "Synthetic personal note, without statistical evidence.".to_owned(),
        }];
        apply_rules(&mut result, &[1092], &rules, "fr");
        assert!(result[0].synergies.is_empty());
        apply_rules(&mut result, &[1092, 1048, 1048], &rules, "fr");
        assert_eq!(result[0].tier, Tier::A);
        assert_eq!(result[1].tier, Tier::B);
        assert_eq!(result[0].sample_count, Some(100));
        assert!(!result[0].stage_specific);
        assert_eq!(result[0].synergies.len(), 1);
        assert!(result[0].synergies[0].starts_with("Règle personnelle [synthetic-demo]"));
        assert!(result[1].synergies.is_empty());
        apply_rules(&mut result, &[1092, 1048], &rules, "fr");
        assert_eq!(result[0].synergies.len(), 1);
    }

    #[test]
    fn conflicting_personal_notes_remain_identifiable_and_do_not_rate_an_unknown_choice() {
        let mut result = recommend(&snapshot(), &catalog(), &[999], &[], None);
        let rules = vec![
            SynergyRule {
                id: "synthetic-favor".to_owned(),
                requires: vec![],
                offered: 999,
                explanation_fr: "Préférence personnelle synthétique.".to_owned(),
                explanation_en: "Synthetic personal preference.".to_owned(),
            },
            SynergyRule {
                id: "synthetic-avoid".to_owned(),
                requires: vec![],
                offered: 999,
                explanation_fr: "Avis personnel synthétique contraire.".to_owned(),
                explanation_en: "Opposing synthetic personal note.".to_owned(),
            },
        ];
        apply_rules(&mut result, &[], &rules, "fr");
        assert_eq!(result[0].tier, Tier::Unknown);
        assert_eq!(result[0].sample_count, None);
        assert_eq!(result[0].synergies.len(), 2);
        assert!(result[0].synergies[0].contains("synthetic-favor"));
        assert!(result[0].synergies[1].contains("synthetic-avoid"));
    }

    #[test]
    fn warm_matcher_keeps_ambiguity_checks_for_near_misses_beyond_the_acceptance_budget() {
        let catalog = Catalog {
            augments: vec![
                augment(
                    1,
                    "First",
                    "Pouvoir malefiqxe phenomenxl",
                    "First synthetic title",
                ),
                augment(
                    2,
                    "Second",
                    "Pouvoir malefoqxe phenomenxl",
                    "Second synthetic title",
                ),
            ],
            ..Catalog::default()
        };
        let matcher = AugmentMatcher::new(&catalog);
        assert!(matcher.match_text("Pouvoir malefique phenomenal").is_none());
        assert_eq!(matcher.match_text("First synthetic title").unwrap().id, 1);
        assert!(matcher.match_text(&"a".repeat(257)).is_none());
    }

    #[test]
    fn item_options_do_not_become_a_six_item_observed_route() {
        let mut snapshot = snapshot();
        snapshot.builds.push(BuildRoute {
            purchase_order: vec![1, 2, 3],
            sample_count: Some(17),
            starters: vec![1],
            boots: vec![2],
            later_items: vec![4, 5, 6],
        });
        let advice = build_advice(&snapshot, &catalog(), &[]);
        assert_eq!(advice[0].item_ids, [1, 2, 3]);
        assert!(advice[0].reason.contains("17 observations"));
        assert_eq!(advice[3].item_ids, [4, 5, 6]);
        assert!(advice[3].label.contains("fin de build"));
        assert!(!advice[3].reason.contains("17 observations"));
        assert!(!advice.iter().any(|entry| entry.item_ids.len() == 6));
    }

    #[test]
    fn item_associations_require_a_known_selected_augment_and_remain_observational() {
        let mut snapshot = snapshot();
        snapshot.item_synergies = vec![
            ItemSynergy {
                item_id: 4,
                augment_id: 1045,
                sample_count: Some(12),
            },
            ItemSynergy {
                item_id: 4,
                augment_id: 1045,
                sample_count: Some(12),
            },
            ItemSynergy {
                item_id: 5,
                augment_id: 1048,
                sample_count: Some(0),
            },
            ItemSynergy {
                item_id: 6,
                augment_id: 999,
                sample_count: Some(12),
            },
        ];
        assert!(build_advice(&snapshot, &catalog(), &[]).is_empty());
        let advice = build_advice(&snapshot, &catalog(), &[1045, 1048, 999]);
        assert_eq!(advice.len(), 1);
        assert_eq!(advice[0].item_ids, [4]);
        assert!(advice[0].label.starts_with("Association observée"));
        assert!(
            advice[0]
                .reason
                .contains("ne prouve pas un bénéfice causal")
        );
    }

    #[test]
    fn english_recommendations_and_builds_do_not_fall_back_to_french() {
        let mut snapshot = snapshot();
        snapshot.item_synergies.push(ItemSynergy {
            item_id: 4,
            augment_id: 1045,
            sample_count: None,
        });
        let mut result =
            recommend_localized(&snapshot, &catalog(), &[1045], &[1092], None, "en-US");
        apply_rules(
            &mut result,
            &[1092],
            &[SynergyRule {
                id: "synthetic-demo".to_owned(),
                requires: vec![1092],
                offered: 1045,
                explanation_fr: "Note de test.".to_owned(),
                explanation_en: "Test note.".to_owned(),
            }],
            "en-US",
        );
        assert!(result[0].explanation.contains("stage unknown"));
        assert!(result[0].synergies[0].starts_with("Personal rule"));
        let advice = build_advice_localized(&snapshot, &catalog(), &[1045], "EN_gb");
        assert!(advice[0].label.contains("Item 4 × Infernal Conduit"));
        assert!(advice[0].reason.contains("sample count unavailable"));
    }
}
