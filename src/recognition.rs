//! Conservative reading decisions, independent of capture, windows and network.
use crate::native::{Observation, Rect};
use serde::Serialize;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

#[derive(Clone, Debug)]
pub struct Offer {
    pub id: u32,
    pub rect: Rect,
    /// Catalog-name similarity, never an OCR success probability.
    pub confidence: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadingQuality {
    Exact,
    Approximate,
    Uncertain,
}

impl ReadingQuality {
    pub fn label(self, language: &str) -> &'static str {
        match (self, language == "en") {
            (Self::Exact, false) => "Nom reconnu exactement",
            (Self::Exact, true) => "Exact name match",
            (Self::Approximate, false) => "Nom approché, lecture stable",
            (Self::Approximate, true) => "Approximate name, stable reading",
            (Self::Uncertain, false) => "Lecture incertaine — relire",
            (Self::Uncertain, true) => "Uncertain reading — scan again",
        }
    }
}

/// A changed group cannot inherit the stability of previous cards. An uncertain
/// member withholds the entire recommendation group and its item advice.
#[derive(Default)]
pub struct ReadingGate {
    previous: Vec<(u32, Rect)>,
    stable_reads: u8,
}

impl ReadingGate {
    pub fn reset(&mut self) {
        self.previous.clear();
        self.stable_reads = 0;
    }

    pub fn assess(&mut self, offers: &[Offer], minimum: f32) -> ReadingQuality {
        if offers.len() != 3
            || offers
                .iter()
                .any(|offer| !offer.confidence.is_finite() || offer.confidence < minimum)
        {
            self.reset();
            return ReadingQuality::Uncertain;
        }
        let same = self.previous.len() == 3
            && self.previous.iter().zip(offers).all(|((id, rect), offer)| {
                *id == offer.id
                    && (i64::from(rect.x) - i64::from(offer.rect.x)).abs()
                        <= i64::from(rect.height.max(offer.rect.height).max(4))
                    && (i64::from(rect.y) - i64::from(offer.rect.y)).abs()
                        <= i64::from(rect.height.max(offer.rect.height).max(4))
            });
        self.stable_reads = if same {
            self.stable_reads.saturating_add(1)
        } else {
            1
        };
        self.previous = offers.iter().map(|offer| (offer.id, offer.rect)).collect();
        if offers.iter().all(|offer| offer.confidence == 1.0) {
            ReadingQuality::Exact
        } else if self.stable_reads >= 2 {
            ReadingQuality::Approximate
        } else {
            ReadingQuality::Uncertain
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StageSource {
    Manual,
    Screen,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageReading {
    pub stage: Option<u8>,
    pub source: StageSource,
}

impl StageReading {
    pub fn resolve(manual: Option<u8>, automatic: bool, screen: Option<u8>) -> Self {
        if let Some(stage) = manual.filter(|stage| (1..=4).contains(stage)) {
            Self {
                stage: Some(stage),
                source: StageSource::Manual,
            }
        } else if automatic && screen.is_some_and(|stage| (1..=4).contains(&stage)) {
            Self {
                stage: screen,
                source: StageSource::Screen,
            }
        } else {
            Self {
                stage: None,
                source: StageSource::Unknown,
            }
        }
    }
}

#[derive(Default)]
pub struct StageTracker {
    previous: Option<u8>,
    group: Vec<u32>,
    stable_reads: u8,
}

impl StageTracker {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// No retained stage when the marker is missing, conflicting or a reroll has
    /// replaced the group. Manual overrides are resolved separately.
    pub fn observe(&mut self, candidate: Option<u8>, offers: &[Offer]) -> Option<u8> {
        let Some(stage) = candidate.filter(|stage| (1..=4).contains(stage)) else {
            self.reset();
            return None;
        };
        if offers.len() != 3 {
            self.reset();
            return None;
        }
        let group: Vec<_> = offers.iter().map(|offer| offer.id).collect();
        self.stable_reads = if self.previous == Some(stage) && self.group == group {
            self.stable_reads.saturating_add(1)
        } else {
            1
        };
        self.previous = Some(stage);
        self.group = group;
        (self.stable_reads >= 2).then_some(stage)
    }
}

/// Only a complete ordinal label is accepted. Bare numbers, level thresholds,
/// elapsed time and number of manually recorded choices cannot prove the stage.
pub fn parse_stage_label(text: &str) -> Option<u8> {
    if text.chars().take(97).count() > 96 {
        return None;
    }
    let mut normalized = String::new();
    for c in text.nfkd().filter(|c| !is_combining_mark(*c)) {
        for c in c.to_lowercase() {
            if c.is_ascii_alphanumeric() || c == '/' {
                normalized.push(c);
            } else if !normalized.ends_with(' ') {
                normalized.push(' ');
            }
        }
    }
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let prefixes = [
        "augment choice ",
        "augmentation choice ",
        "augment stage ",
        "choix d augmentation ",
        "choix d optimisation ",
        "choix d optimisations ",
        "augmentation ",
        "optimisation ",
        "choix ",
    ];
    for prefix in prefixes {
        let Some(suffix) = normalized.strip_prefix(prefix) else {
            continue;
        };
        for stage in 1..=4_u8 {
            if [
                stage.to_string(),
                format!("{stage}/4"),
                format!("{stage} / 4"),
                format!("{stage} sur 4"),
                format!("{stage} of 4"),
            ]
            .contains(&suffix.to_owned())
            {
                return Some(stage);
            }
        }
    }
    None
}

pub fn screen_stage(observations: &[Observation], offers: &[Offer], bounds: Rect) -> Option<u8> {
    if offers.len() != 3 || bounds.width <= 0 || bounds.height <= 0 {
        return None;
    }
    let top = offers.iter().map(|offer| offer.rect.y).min()?;
    let left_center = offers.first()?.rect.x + offers.first()?.rect.width / 2;
    let right_center = offers.last()?.rect.x + offers.last()?.rect.width / 2;
    let mut found = std::collections::BTreeSet::new();
    for observation in observations {
        let rect = observation.rect;
        let center = i64::from(rect.x) + i64::from(rect.width) / 2;
        if rect.width <= 0
            || rect.height <= 0
            || i64::from(rect.y) < i64::from(bounds.y)
            || i64::from(rect.y) + i64::from(rect.height) > i64::from(top)
            || i64::from(top) - i64::from(rect.y) > i64::from(bounds.height) * 35 / 100
            || center < i64::from(left_center)
            || center > i64::from(right_center)
            || i64::from(rect.width) > i64::from(bounds.width) * 3 / 4
            || i64::from(rect.height) > i64::from(bounds.height) / 12
        {
            continue;
        }
        if let Some(stage) = parse_stage_label(&observation.text) {
            found.insert(stage);
        }
    }
    (found.len() == 1).then(|| *found.first().expect("one stage"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn offers(confidence: f32) -> Vec<Offer> {
        (0..3)
            .map(|i| Offer {
                id: i + 1,
                rect: Rect {
                    x: 200 + i as i32 * 400,
                    y: 500,
                    width: 180,
                    height: 24,
                },
                confidence,
            })
            .collect()
    }
    #[test]
    fn fuzzy_names_wait_and_changed_or_uncertain_cards_withhold_tiers() {
        let mut gate = ReadingGate::default();
        let mut cards = offers(0.97);
        assert_eq!(gate.assess(&cards, 0.95), ReadingQuality::Uncertain);
        assert_eq!(gate.assess(&cards, 0.95), ReadingQuality::Approximate);
        cards[1].id = 42;
        assert_eq!(gate.assess(&cards, 0.95), ReadingQuality::Uncertain);
        assert_eq!(gate.assess(&cards, 0.95), ReadingQuality::Approximate);
        cards[2].confidence = 0.94;
        assert_eq!(gate.assess(&cards, 0.95), ReadingQuality::Uncertain);
        cards[2].confidence = 0.97;
        assert_eq!(gate.assess(&cards, 0.95), ReadingQuality::Uncertain);
        assert_eq!(gate.assess(&offers(1.0), 1.0), ReadingQuality::Exact);
        assert_eq!(
            gate.assess(&offers(f32::NAN), 0.95),
            ReadingQuality::Uncertain
        );
        assert_eq!(gate.assess(&cards[..2], 0.95), ReadingQuality::Uncertain);
    }
    #[test]
    fn stage_labels_are_explicit_not_level_or_body_text() {
        for label in [
            "Augment choice 2 of 4",
            "Choix d’optimisation 2 sur 4",
            "Augmentation 2/4",
            "CHOIX 2 / 4",
        ] {
            assert_eq!(parse_stage_label(label), Some(2), "{label}");
        }
        for label in [
            "Choose an Augment",
            "7",
            "Niveau 7",
            "Level 15",
            "Augment choice 5",
            "Choix 2/5",
            "Choix 2 sur 4 dégâts",
            "Augmentation 2 réduit les dégâts",
            "2/4",
            "Round 2",
        ] {
            assert_eq!(parse_stage_label(label), None, "{label}");
        }
    }
    #[test]
    fn automatic_stage_requires_two_readings_and_never_survives_loss_or_reroll() {
        let cards = offers(1.0);
        let mut tracker = StageTracker::default();
        assert_eq!(tracker.observe(Some(2), &cards), None);
        assert_eq!(tracker.observe(Some(2), &cards), Some(2));
        assert_eq!(tracker.observe(None, &cards), None);
        assert_eq!(tracker.observe(Some(2), &cards), None);
        assert_eq!(tracker.observe(Some(2), &cards), Some(2));
        let mut reroll = cards.clone();
        reroll[1].id = 44;
        assert_eq!(tracker.observe(Some(2), &reroll), None);
        assert_eq!(
            StageReading::resolve(Some(4), true, Some(2)).source,
            StageSource::Manual
        );
        assert_eq!(StageReading::resolve(None, false, Some(2)).stage, None);
        assert_eq!(
            StageReading::resolve(None, true, Some(2)).source,
            StageSource::Screen
        );
    }
    #[test]
    fn stage_is_accepted_only_above_cards_and_conflicting_markers_fail_closed() {
        let cards = offers(1.0);
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 1440,
            height: 900,
        };
        let marker = Observation {
            text: "Choix 3 sur 4".into(),
            rect: Rect {
                x: 600,
                y: 400,
                width: 180,
                height: 25,
            },
        };
        assert_eq!(
            screen_stage(std::slice::from_ref(&marker), &cards, bounds),
            Some(3)
        );
        let mut body = marker.clone();
        body.rect.y = 550;
        assert_eq!(screen_stage(&[body], &cards, bounds), None);
        let mut outside = marker.clone();
        outside.rect.x = 0;
        assert_eq!(screen_stage(&[outside], &cards, bounds), None);
        let mut conflict = marker.clone();
        conflict.text = "Choix 2".into();
        assert_eq!(screen_stage(&[marker, conflict], &cards, bounds), None);
    }
}
