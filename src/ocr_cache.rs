//! Four independent OCR regions. No capture, clock, file or network access.

use crate::native::{Observation, Rect};

/// Physical region coordinates and complete frame geometry must agree before
/// reusing text. The caller resets the cache on HWND, language or session changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegionOcrKey {
    pub(crate) region: Rect,
    pub(crate) frame_bounds: Rect,
    pub(crate) frame_width: usize,
    pub(crate) frame_height: usize,
    pub(crate) pixels_hash: u64,
}

#[derive(Debug)]
struct CachedRegion {
    key: RegionOcrKey,
    observations: Vec<Observation>,
}

/// Index 0 holds the header; indices 1, 2 and 3 hold the three card titles.
#[derive(Debug, Default)]
pub(crate) struct RegionOcrCache {
    entries: [Option<CachedRegion>; 4],
}

impl RegionOcrCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A changed key destroys the old reading before OCR is attempted, so an
    /// unreadable reroll cannot leave an old title available to the caller.
    pub(crate) fn get(&mut self, index: usize, key: RegionOcrKey) -> Option<&[Observation]> {
        let entry = self.entries.get_mut(index)?;
        if entry.as_ref().is_some_and(|cached| cached.key != key) {
            *entry = None;
        }
        entry.as_ref().map(|cached| cached.observations.as_slice())
    }

    pub(crate) fn put(&mut self, index: usize, key: RegionOcrKey, observations: Vec<Observation>) {
        if let Some(entry) = self.entries.get_mut(index) {
            *entry = Some(CachedRegion { key, observations });
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::{RegionOcrCache, RegionOcrKey};
    use crate::native::{Observation, Rect};

    fn key(index: usize) -> RegionOcrKey {
        RegionOcrKey {
            region: Rect {
                x: -2300 + index as i32 * 400,
                y: if index == 0 { 200 } else { 500 },
                width: if index == 0 { 1000 } else { 240 },
                height: 60,
            },
            frame_bounds: Rect {
                x: -2560,
                y: 0,
                width: 2560,
                height: 1440,
            },
            frame_width: 2560,
            frame_height: 1440,
            pixels_hash: index as u64 + 10,
        }
    }

    fn reading(key: RegionOcrKey, text: &str) -> Vec<Observation> {
        vec![Observation {
            text: text.into(),
            rect: key.region,
        }]
    }

    fn populated_cache() -> RegionOcrCache {
        let mut cache = RegionOcrCache::new();
        for (index, text) in ["Choix 2", "Goliath", "Jeweled Gauntlet", "Scoped Weapons"]
            .into_iter()
            .enumerate()
        {
            cache.put(index, key(index), reading(key(index), text));
        }
        cache
    }

    #[test]
    fn one_card_reroll_discards_its_reading_and_preserves_the_other_regions() {
        let mut cache = populated_cache();
        let old_key = key(1);
        let rerolled_key = RegionOcrKey {
            pixels_hash: 99,
            ..old_key
        };

        assert!(cache.get(1, rerolled_key).is_none());
        assert!(cache.get(1, old_key).is_none());
        for index in [0, 2, 3] {
            assert!(cache.get(index, key(index)).is_some());
        }

        let new_reading = reading(rerolled_key, "Agrandissement");
        cache.put(1, rerolled_key, new_reading.clone());
        assert_eq!(cache.get(1, rerolled_key), Some(new_reading.as_slice()));
    }

    #[test]
    fn a_changed_header_cannot_reuse_its_previous_stage_but_preserves_cards() {
        let mut cache = populated_cache();
        let changed_header = RegionOcrKey {
            pixels_hash: 42,
            ..key(0)
        };

        assert!(cache.get(0, changed_header).is_none());
        assert!(cache.get(0, key(0)).is_none());
        for index in 1..=3 {
            assert!(cache.get(index, key(index)).is_some());
        }
    }

    #[test]
    fn physical_region_and_frame_geometry_are_part_of_the_key() {
        let original = key(2);
        let mut shifted_region = original;
        shifted_region.region.x += 1;
        let mut shifted_frame = original;
        shifted_frame.frame_bounds.x += 2560;
        let mut resized_frame = original;
        resized_frame.frame_bounds.width += 1;
        let mut changed_width = original;
        changed_width.frame_width += 1;
        let mut changed_height = original;
        changed_height.frame_height += 1;

        for changed in [
            shifted_region,
            shifted_frame,
            resized_frame,
            changed_width,
            changed_height,
        ] {
            let mut cache = populated_cache();
            assert!(cache.get(2, changed).is_none());
            assert!(cache.get(2, original).is_none());
        }
    }

    #[test]
    fn reset_discards_header_and_all_cards_even_with_identical_keys() {
        let mut cache = populated_cache();
        cache.reset();
        for index in 0..4 {
            assert!(cache.get(index, key(index)).is_none());
        }
    }
}
