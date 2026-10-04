//! In-memory geometry feedback. This module never captures pixels or reads a file.

use crate::native::Rect;
use serde::Serialize;

const DISCOVERY_EVERY: u16 = 12;
const REQUIRED_STABLE_SAMPLES: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CalibrationMode {
    Discovery,
    Learned,
    PeriodicDiscovery,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationSnapshot {
    pub mode: CalibrationMode,
    /// Absolute physical screen pixels; includes the header above the titles.
    pub roi: Rect,
    pub stable_samples: u8,
    pub captures_since_discovery: u16,
}

#[derive(Default)]
pub(crate) struct Calibration {
    bounds: Option<Rect>,
    learned: Option<Rect>,
    pending: Option<Rect>,
    pending_titles: Option<[Rect; 3]>,
    stable_samples: u8,
    captures_since_discovery: u16,
}

impl Calibration {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn next(&mut self, bounds: Rect) -> CalibrationSnapshot {
        if self.bounds != Some(bounds) {
            self.reset();
            self.bounds = Some(bounds);
        }
        let (mode, roi) = match self.learned {
            Some(_) if self.captures_since_discovery >= DISCOVERY_EVERY => {
                self.captures_since_discovery = 0;
                (CalibrationMode::PeriodicDiscovery, discovery(bounds))
            }
            Some(roi) => {
                self.captures_since_discovery += 1;
                (CalibrationMode::Learned, roi)
            }
            None => {
                self.captures_since_discovery = 0;
                (CalibrationMode::Discovery, discovery(bounds))
            }
        };
        CalibrationSnapshot {
            mode,
            roi,
            stable_samples: self.stable_samples,
            captures_since_discovery: self.captures_since_discovery,
        }
    }

    /// Fixed card cells, rather than the previous title's narrow OCR box. The
    /// envelope includes space for a longer, wrapped replacement after a reroll.
    /// Index zero is the separate stage header; the other three are title cells.
    pub(crate) fn regions(&self) -> Option<[Rect; 4]> {
        let roi = self.learned?;
        let bounds = self.bounds?;
        let titles = self.pending_titles?;
        let centers = titles.map(|r| i64::from(r.x) + i64::from(r.width) / 2);
        let edges = [
            i64::from(roi.x),
            (centers[0] + centers[1]) / 2,
            (centers[1] + centers[2]) / 2,
            i64::from(roi.x) + i64::from(roi.width),
        ];
        let top = titles.iter().map(|r| r.y).min()?;
        let title_top = (top - fraction(bounds.height, 2)).max(roi.y);
        let bottom = roi.y + roi.height;
        let header = Rect {
            height: top - roi.y,
            ..roi
        };
        if header.height <= 0 {
            return None;
        }
        // Offer association accepts titles up to 40% of the window. Cover it
        // plus the tolerated 1% center drift, even for formerly very short names.
        let half_width = i64::from(fraction(bounds.width, 21));
        let cells: [Rect; 3] = std::array::from_fn(|i| {
            let left = edges[i].min(centers[i] - half_width).max(edges[0]);
            let right = edges[i + 1].max(centers[i] + half_width).min(edges[3]);
            Rect {
                x: left as i32,
                y: title_top,
                width: (right - left) as i32,
                height: bottom - title_top,
            }
        });
        Some([header, cells[0], cells[1], cells[2]])
    }

    /// Broad OCR cells can overlap to preserve long words. Assign their text
    /// back to its nearest learned card center instead of importing a neighbour.
    pub(crate) fn owns_observation(&self, index: usize, rect: Rect) -> bool {
        let Some(regions) = self.regions() else {
            return false;
        };
        if index >= regions.len() || !inside(rect, regions[index]) {
            return false;
        }
        if index == 0 {
            return true;
        }
        let Some(titles) = self.pending_titles else {
            return false;
        };
        let center = i64::from(rect.x) * 2 + i64::from(rect.width);
        let centers = titles.map(|r| i64::from(r.x) * 2 + i64::from(r.width));
        let Some(bounds) = self.bounds else {
            return false;
        };
        // A fragment from the neighbouring, wider rerolled title can cross the
        // midpoint and look closer to this card. Only a centered title remains
        // eligible for regional reuse; moved layouts require broad discovery.
        let tolerance = i64::from((bounds.width / 100).max(2)) * 2;
        if (center - centers[index - 1]).abs() > tolerance {
            return false;
        }
        centers
            .iter()
            .enumerate()
            .min_by_key(|(_, candidate)| (center - **candidate).abs())
            .is_some_and(|(slot, _)| slot + 1 == index)
    }

    /// Called only with the three accepted title bounds, or [] after a miss.
    pub(crate) fn feedback(&mut self, rects: &[Rect]) {
        let Some(bounds) = self.bounds else { return };
        let Some((candidate, titles)) = coherent_region(bounds, rects) else {
            self.learned = None;
            self.pending = None;
            self.pending_titles = None;
            self.stable_samples = 0;
            self.captures_since_discovery = 0;
            return;
        };
        let tolerance = (bounds.height / 100).max(2);
        let same_titles = self.pending_titles.is_some_and(|prior| {
            prior.iter().zip(titles).all(|(before, after)| {
                let before_x = i64::from(before.x) * 2 + i64::from(before.width);
                let after_x = i64::from(after.x) * 2 + i64::from(after.width);
                (before_x - after_x).abs() <= i64::from((bounds.width / 100).max(2)) * 2
                    && (i64::from(before.y) - i64::from(after.y)).abs() <= i64::from(tolerance)
            })
        });
        if same_titles
            && self.pending.is_some_and(|prior| {
                (i64::from(prior.height) - i64::from(candidate.height)).abs()
                    <= i64::from(tolerance)
            })
        {
            self.stable_samples = self
                .stable_samples
                .saturating_add(1)
                .min(REQUIRED_STABLE_SAMPLES);
        } else {
            self.pending = Some(candidate);
            self.stable_samples = 1;
            // A changed row must be rediscovered before trusting a tighter crop.
            self.learned = None;
        }
        self.pending_titles = Some(titles);
        if self.stable_samples >= REQUIRED_STABLE_SAMPLES {
            // Keep the larger envelope when a wrapped title changes by a few pixels.
            let prior_height = self.pending.map_or(candidate.height, |prior| prior.height);
            let roi = Rect {
                height: candidate.height.max(prior_height),
                ..candidate
            };
            self.pending = Some(roi);
            self.learned = Some(roi);
        }
    }
}

fn fraction(value: i32, percent: i32) -> i32 {
    ((i64::from(value) * i64::from(percent) + 50) / 100) as i32
}

pub(crate) fn discovery(bounds: Rect) -> Rect {
    let left = fraction(bounds.width, 6);
    let right = fraction(bounds.width, 94);
    let top = fraction(bounds.height, 10);
    let bottom = fraction(bounds.height, 88);
    Rect {
        x: bounds.x.saturating_add(left),
        y: bounds.y.saturating_add(top),
        width: right - left,
        height: bottom - top,
    }
}

fn inside(rect: Rect, bounds: Rect) -> bool {
    rect.width > 0
        && rect.height > 0
        && i64::from(rect.x) >= i64::from(bounds.x)
        && i64::from(rect.y) >= i64::from(bounds.y)
        && i64::from(rect.x) + i64::from(rect.width)
            <= i64::from(bounds.x) + i64::from(bounds.width)
        && i64::from(rect.y) + i64::from(rect.height)
            <= i64::from(bounds.y) + i64::from(bounds.height)
}

fn coherent_region(bounds: Rect, rects: &[Rect]) -> Option<(Rect, [Rect; 3])> {
    if bounds.width <= 0 || bounds.height <= 0 || rects.len() != 3 {
        return None;
    }
    let broad = discovery(bounds);
    let mut titles = [rects[0], rects[1], rects[2]];
    titles.sort_by_key(|rect| rect.x);
    if titles.iter().any(|rect| {
        !inside(*rect, broad)
            || rect.width > fraction(bounds.width, 30)
            || rect.height > fraction(bounds.height, 10)
    }) {
        return None;
    }
    let centers = titles.map(|rect| i64::from(rect.x) * 2 + i64::from(rect.width));
    let gaps = [centers[1] - centers[0], centers[2] - centers[1]];
    let twice_width = i64::from(bounds.width) * 2;
    if gaps
        .iter()
        .any(|gap| *gap < twice_width * 12 / 100 || *gap > twice_width * 45 / 100)
        || (gaps[0] - gaps[1]).abs() * 4 > gaps[0].max(gaps[1])
        || titles
            .windows(2)
            .any(|pair| i64::from(pair[0].x) + i64::from(pair[0].width) >= i64::from(pair[1].x))
    {
        return None;
    }
    let top = titles.iter().map(|rect| rect.y).min()?;
    let max_top = titles.iter().map(|rect| rect.y).max()?;
    if i64::from(max_top) - i64::from(top) > i64::from(fraction(bounds.height, 5).max(2)) {
        return None;
    }
    let bottom = titles
        .iter()
        .map(|rect| i64::from(rect.y) + i64::from(rect.height))
        .max()?;
    let envelope_bottom = bottom.max(i64::from(max_top) + i64::from(fraction(bounds.height, 10)));
    let padded_bottom = (envelope_bottom + i64::from(fraction(bounds.height, 4).max(2)))
        .min(i64::from(broad.y) + i64::from(broad.height));
    // Preserve broad horizontal bounds and its header area: long replacement
    // titles and the explicit choice header must remain visible after learning.
    Some((
        Rect {
            height: (padded_bottom - i64::from(broad.y)) as i32,
            ..broad
        },
        titles,
    ))
}

/// Converts absolute physical geometry to frame pixels with outward rounding.
/// This keeps title/header coverage when WGC and window dimensions differ.
pub(crate) fn frame_region(bounds: Rect, roi: Rect, width: i32, height: i32) -> Option<Rect> {
    if width <= 0 || height <= 0 || !inside(roi, bounds) {
        return None;
    }
    let scale_x = f64::from(width) / f64::from(bounds.width);
    let scale_y = f64::from(height) / f64::from(bounds.height);
    let local_x = i64::from(roi.x) - i64::from(bounds.x);
    let local_y = i64::from(roi.y) - i64::from(bounds.y);
    let left = (local_x as f64 * scale_x).floor().max(0.0) as i32;
    let top = (local_y as f64 * scale_y).floor().max(0.0) as i32;
    let right = (((local_x + i64::from(roi.width)) as f64 * scale_x).ceil() as i32).min(width);
    let bottom = (((local_y + i64::from(roi.height)) as f64 * scale_y).ceil() as i32).min(height);
    (right > left && bottom > top).then_some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_cells_cover_long_wrapped_rerolls_and_keep_header_separate() {
        for (width, height, x, y) in [
            (1280, 720, 0, 0),
            (2560, 1440, -2560, -1440),
            (3840, 2160, 0, 0),
        ] {
            let game = Rect {
                x,
                y,
                width,
                height,
            };
            let mut calibration = Calibration::default();
            calibration.next(game);
            let short = titles(game);
            assert!(calibration.regions().is_none());
            calibration.feedback(&short);
            calibration.feedback(&short);
            let regions = calibration.regions().unwrap();
            for (slot, before) in short.into_iter().enumerate() {
                let long = Rect {
                    x: before.x + before.width / 2 - fraction(width, 15),
                    width: fraction(width, 30),
                    height: fraction(height, 8),
                    ..before
                };
                assert!(inside(long, regions[slot + 1]));
                assert!(calibration.owns_observation(slot + 1, long));
                assert!(!calibration.owns_observation((slot + 1) % 3 + 1, long));
                assert!(inside(regions[slot + 1], calibration.learned.unwrap()));
            }
            assert_eq!(regions[0].y, discovery(game).y);
            assert_eq!(regions[0].y + regions[0].height, short[0].y);
            let central_wide = Rect {
                x: short[1].x + short[1].width / 2 - fraction(width, 20),
                width: fraction(width, 40),
                height: fraction(height, 8),
                ..short[1]
            };
            assert!(inside(central_wide, regions[2]));
            assert!(calibration.owns_observation(2, central_wide));
            calibration.feedback(&[]);
            assert!(calibration.regions().is_none());
        }
    }

    #[test]
    fn clipped_neighbour_and_moved_title_cannot_become_a_regional_offer() {
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 700,
        };
        let titles = [240, 500, 760].map(|center| Rect {
            x: center - 40,
            y: 322,
            width: 80,
            height: 21,
        });
        let mut calibration = Calibration::default();
        calibration.next(bounds);
        calibration.feedback(&titles);
        calibration.feedback(&titles);
        let fragment = Rect {
            x: 340,
            y: 322,
            width: 70,
            height: 21,
        };
        assert!(inside(fragment, calibration.regions().unwrap()[2]));
        assert!(!calibration.owns_observation(2, fragment));
        assert!(!calibration.owns_observation(
            1,
            Rect {
                x: 110,
                width: 300,
                ..titles[0]
            }
        ));
        assert!(calibration.owns_observation(
            1,
            Rect {
                x: 90,
                width: 300,
                ..titles[0]
            }
        ));
    }

    fn titles(game: Rect) -> [Rect; 3] {
        [20, 46, 72].map(|percent| Rect {
            x: game.x + fraction(game.width, percent),
            y: game.y + fraction(game.height, 46),
            width: fraction(game.width, 8),
            height: fraction(game.height, 3),
        })
    }

    #[test]
    fn learns_bounded_roi_with_header_at_720p_1080p_1440p_and_4k() {
        for (width, height, x, y) in [
            (1280, 720, 0, 0),
            (1920, 1080, -1920, 0),
            (2560, 1440, 0, -1440),
            (3840, 2160, -3840, -2160),
        ] {
            let game = Rect {
                x,
                y,
                width,
                height,
            };
            let mut calibration = Calibration::default();
            let initial = calibration.next(game);
            assert_eq!(initial.mode, CalibrationMode::Discovery);
            let rects = titles(game);
            calibration.feedback(&rects);
            assert_eq!(calibration.next(game).mode, CalibrationMode::Discovery);
            calibration.feedback(&rects);
            let learned = calibration.next(game);
            assert_eq!(learned.mode, CalibrationMode::Learned);
            assert!(inside(learned.roi, game));
            assert!(rects.iter().all(|rect| inside(*rect, learned.roi)));
            assert_eq!(learned.roi.y, initial.roi.y);
            assert_eq!(learned.roi.width, initial.roi.width);
            assert!(learned.roi.height < initial.roi.height);
        }
    }

    #[test]
    fn ambiguous_nonuniform_and_out_of_bounds_layouts_never_train() {
        let game = Rect {
            x: -1920,
            y: -100,
            width: 1920,
            height: 1080,
        };
        let good = titles(game);
        let mut far_row = good;
        far_row[2].y += 150;
        let mut uneven = good;
        uneven[1].x -= 250;
        let mut outside = good;
        outside[0].x = game.x - 1;
        let mut overlapping = good;
        overlapping[1] = overlapping[0];
        let mut zero = good;
        zero[0].height = 0;
        for bad in [
            &good[..2],
            &far_row[..],
            &uneven[..],
            &outside[..],
            &overlapping[..],
            &zero[..],
            &[],
        ] {
            let mut calibration = Calibration::default();
            calibration.next(game);
            calibration.feedback(bad);
            calibration.feedback(bad);
            assert_eq!(calibration.next(game).mode, CalibrationMode::Discovery);
        }
    }

    #[test]
    fn miss_resize_reposition_and_reset_require_new_stable_feedback() {
        let game = Rect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let mut calibration = Calibration::default();
        calibration.next(game);
        calibration.feedback(&titles(game));
        calibration.feedback(&titles(game));
        assert_eq!(calibration.next(game).mode, CalibrationMode::Learned);
        calibration.feedback(&[]);
        assert_eq!(calibration.next(game).mode, CalibrationMode::Discovery);
        calibration.feedback(&titles(game));
        calibration.feedback(&titles(game));
        for changed in [
            Rect {
                width: 1920,
                height: 1080,
                ..game
            },
            Rect { x: -2560, ..game },
        ] {
            assert_eq!(calibration.next(changed).mode, CalibrationMode::Discovery);
            calibration.feedback(&titles(changed));
            calibration.feedback(&titles(changed));
            assert_eq!(calibration.next(changed).mode, CalibrationMode::Learned);
        }
        calibration.reset();
        assert_eq!(calibration.next(game).mode, CalibrationMode::Discovery);
    }

    #[test]
    fn learned_region_periodically_rediscovers_and_rejects_a_moved_title_row() {
        let game = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let mut calibration = Calibration::default();
        calibration.next(game);
        calibration.feedback(&titles(game));
        calibration.feedback(&titles(game));
        for _ in 0..DISCOVERY_EVERY {
            assert_eq!(calibration.next(game).mode, CalibrationMode::Learned);
        }
        let rediscovery = calibration.next(game);
        assert_eq!(rediscovery.mode, CalibrationMode::PeriodicDiscovery);
        assert_eq!(rediscovery.roi, discovery(game));
        let moved = titles(game).map(|rect| Rect {
            y: rect.y + 100,
            ..rect
        });
        calibration.feedback(&moved);
        assert_eq!(calibration.next(game).mode, CalibrationMode::Discovery);
        calibration.feedback(&moved);
        assert_eq!(calibration.next(game).mode, CalibrationMode::Learned);
    }

    #[test]
    fn moving_columns_do_not_count_as_stable_even_with_identical_crop() {
        let game = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let mut calibration = Calibration::default();
        calibration.next(game);
        calibration.feedback(&titles(game));
        let shifted = titles(game).map(|rect| Rect {
            x: rect.x + 60,
            ..rect
        });
        calibration.feedback(&shifted);
        assert_eq!(calibration.next(game).mode, CalibrationMode::Discovery);
        calibration.feedback(&shifted);
        assert_eq!(calibration.next(game).mode, CalibrationMode::Learned);
    }

    #[test]
    fn frame_projection_preserves_coverage_and_negative_screen_origins() {
        for (width, height) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
            let game = Rect {
                x: -width,
                y: -height,
                width,
                height,
            };
            let roi = discovery(game);
            let native = frame_region(game, roi, width, height).unwrap();
            assert_eq!(native.x, roi.x - game.x);
            assert_eq!(native.y, roi.y - game.y);
            assert_eq!(native.width, roi.width);
            let scaled = frame_region(game, roi, width / 2 + 1, height / 2 + 1).unwrap();
            assert!(scaled.x >= 0 && scaled.y >= 0);
            assert!(scaled.x + scaled.width <= width / 2 + 1);
            assert!(scaled.y + scaled.height <= height / 2 + 1);
            assert!(
                frame_region(
                    game,
                    Rect {
                        x: game.x - 1,
                        ..roi
                    },
                    width,
                    height
                )
                .is_none()
            );
        }
    }
}
