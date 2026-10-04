//! Cheap visual hints before OCR; this never identifies an augmentation.
//!
//! The caller supplies its already cropped, tightly packed BGRA region. Sparse
//! sampling keeps the work bounded without copying the image. A plausible row
//! of text in three broad columns requests OCR, while other nonuniform images
//! stay uncertain and must retain the caller's periodic OCR fallback. Color and
//! geometry here are shortcuts, not requirements imposed on the game's UI.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VisualGate {
    Candidate,
    Uncertain,
    Absent,
}

const SAMPLE_COLUMNS: usize = 384;
const SAMPLE_ROWS: usize = 256;
const ROW_BINS: usize = 64;
// A broad vertical neighbourhood accommodates wrapped titles and uneven
// baselines; no exact title position or reroll-button geometry is assumed.
const BAND_BINS: usize = 8;

pub(crate) fn inspect(pixels: &[u8], width: usize, height: usize) -> VisualGate {
    let Some(expected_bytes) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
        return VisualGate::Uncertain;
    };
    if width < 48 || height < 32 || pixels.len() < expected_bytes {
        return VisualGate::Uncertain;
    }

    let x_step = width.div_ceil(SAMPLE_COLUMNS);
    let y_step = height.div_ceil(SAMPLE_ROWS);
    let neighbour_distance = (x_step / 2).max(1);
    let mut edge_counts = [[0_u16; ROW_BINS]; 3];
    let mut edge_columns = [[0_u64; ROW_BINS]; 3];
    let mut minimum = u8::MAX;
    let mut maximum = u8::MIN;

    for y in (0..height).step_by(y_step) {
        let row_bin = y * ROW_BINS / height;
        for x in (0..width).step_by(x_step) {
            let sample = pixel(pixels, width, x, y);
            let value = luminance(sample);
            minimum = minimum.min(value);
            maximum = maximum.max(value);

            // Light, approximately neutral title pixels are only a hint. A
            // dim or strongly colored title falls through to periodic OCR.
            let low = sample[0].min(sample[1]).min(sample[2]);
            let high = sample[0].max(sample[1]).max(sample[2]);
            if value < 128 || high - low > 112 {
                continue;
            }
            let left_x = x.saturating_sub(neighbour_distance);
            let right_x = x.saturating_add(neighbour_distance).min(width - 1);
            let left = luminance(pixel(pixels, width, left_x, y));
            let right = luminance(pixel(pixels, width, right_x, y));
            minimum = minimum.min(left).min(right);
            maximum = maximum.max(left).max(right);
            // Horizontal transitions favor glyph strokes over a wide bright
            // separator. Vertical edges of a plain rectangle occupy too few
            // different horizontal buckets to qualify as text below.
            if value.abs_diff(left).max(value.abs_diff(right)) < 24 {
                continue;
            }
            let column = x * 3 / width;
            let start = width * column / 3;
            let end = width * (column + 1) / 3;
            let bucket = ((x - start) * 64 / (end - start)).min(63);
            edge_counts[column][row_bin] = edge_counts[column][row_bin].saturating_add(1);
            edge_columns[column][row_bin] |= 1_u64 << bucket;
        }
    }

    // Only effectively flat regions warrant an absent hint. A dark but busy
    // game, weak contrast or one unrelated label remains uncertain: it cannot
    // suppress all later attempts at OCR.
    if maximum - minimum <= 3 {
        return VisualGate::Absent;
    }

    for top in 0..=ROW_BINS - BAND_BINS {
        let aligned = (0..3).all(|column| {
            let count: u32 = edge_counts[column][top..top + BAND_BINS]
                .iter()
                .map(|&count| u32::from(count))
                .sum();
            let positions = edge_columns[column][top..top + BAND_BINS]
                .iter()
                .fold(0_u64, |positions, &bucket| positions | bucket);
            count >= 6 && positions.count_ones() >= 3
        });
        if aligned {
            return VisualGate::Candidate;
        }
    }
    VisualGate::Uncertain
}

fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> &[u8] {
    let start = (y * width + x) * 4;
    &pixels[start..start + 3]
}

fn luminance(bgra: &[u8]) -> u8 {
    ((u32::from(bgra[0]) * 29 + u32::from(bgra[1]) * 150 + u32::from(bgra[2]) * 77) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(width: usize, height: usize, gray: u8) -> Vec<u8> {
        [gray, gray, gray, 255].repeat(width * height)
    }

    fn paint(pixels: &mut [u8], width: usize, x: usize, y: usize, gray: u8) {
        let start = (y * width + x) * 4;
        pixels[start..start + 3].fill(gray);
    }

    // Synthetic glyph strokes, not copies of game assets or real screenshots.
    fn title(pixels: &mut [u8], width: usize, x: usize, y: usize, scale: usize, gray: u8) {
        for glyph in 0..8 {
            for row in 0..7 {
                for col in 0..5 {
                    if col == 0 || col == 4 || row == 0 || row == 3 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                paint(
                                    pixels,
                                    width,
                                    x + (glyph * 7 + col) * scale + dx,
                                    y + row * scale + dy,
                                    gray,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_or_tiny_buffers_remain_uncertain() {
        assert_eq!(inspect(&[], 0, 0), VisualGate::Uncertain);
        assert_eq!(inspect(&[0; 64], 100, 100), VisualGate::Uncertain);
        assert_eq!(inspect(&[], usize::MAX, 2), VisualGate::Uncertain);
        assert_eq!(inspect(&canvas(20, 20, 0), 20, 20), VisualGate::Uncertain);
    }

    #[test]
    fn flat_dark_and_bright_regions_are_absent() {
        for gray in [0, 18, 100, 255] {
            let pixels = canvas(640, 360, gray);
            assert_eq!(inspect(&pixels, 640, 360), VisualGate::Absent);
        }
    }

    #[test]
    fn aligned_titles_request_ocr_at_different_sizes_and_heights() {
        for (width, height, scale) in [(640, 360, 1), (1920, 1080, 3), (3840, 2160, 6)] {
            for y in [height / 6, height / 2, height * 3 / 4] {
                let mut pixels = canvas(width, height, 24);
                for center in [20, 46, 72] {
                    title(&mut pixels, width, width * center / 100, y, scale, 224);
                }
                assert_eq!(inspect(&pixels, width, height), VisualGate::Candidate);
            }
        }
    }

    #[test]
    fn wrapped_titles_with_uneven_baselines_remain_candidates() {
        let (width, height) = (960, 540);
        let mut pixels = canvas(width, height, 24);
        for (x, y) in [(170, 180), (420, 188), (680, 182)] {
            title(&mut pixels, width, x, y, 2, 216);
        }
        title(&mut pixels, width, 420, 208, 2, 216);
        assert_eq!(inspect(&pixels, width, height), VisualGate::Candidate);
    }

    #[test]
    fn unrelated_single_title_and_plain_rectangles_do_not_request_ocr() {
        let (width, height) = (960, 540);
        let mut pixels = canvas(width, height, 24);
        title(&mut pixels, width, 420, 180, 2, 224);
        assert_eq!(inspect(&pixels, width, height), VisualGate::Uncertain);

        let mut pixels = canvas(width, height, 24);
        for x in [150, 400, 650] {
            for y in 180..210 {
                for dx in 0..140 {
                    paint(&mut pixels, width, x + dx, y, 224);
                }
            }
        }
        assert_eq!(inspect(&pixels, width, height), VisualGate::Uncertain);
    }

    #[test]
    fn weak_contrast_and_colored_titles_retain_fallback() {
        let (width, height) = (960, 540);
        let mut pixels = canvas(width, height, 80);
        for x in [170, 420, 680] {
            title(&mut pixels, width, x, 180, 2, 96);
        }
        assert_eq!(inspect(&pixels, width, height), VisualGate::Uncertain);

        let mut pixels = canvas(width, height, 24);
        for x in [170, 420, 680] {
            title(&mut pixels, width, x, 180, 2, 224);
        }
        for bgra in pixels.as_chunks_mut::<4>().0 {
            if bgra[0] == 224 {
                bgra[..3].copy_from_slice(&[20, 50, 240]);
            }
        }
        assert_eq!(inspect(&pixels, width, height), VisualGate::Uncertain);
    }

    #[test]
    fn serializes_as_a_visual_hint_not_ocr_confidence() {
        assert_eq!(
            serde_json::to_string(&VisualGate::Candidate).unwrap(),
            "\"candidate\""
        );
        assert_eq!(
            serde_json::to_string(&VisualGate::Uncertain).unwrap(),
            "\"uncertain\""
        );
        assert_eq!(
            serde_json::to_string(&VisualGate::Absent).unwrap(),
            "\"absent\""
        );
    }
}
