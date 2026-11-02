//! Commentary passage colors, using Pi's OKHSL lightness and 256-color selection.

use palette::{FromColor, Okhsl, Srgb};
use ratatui::style::Color;
use crate::terminal_palette::{StdoutColorLevel, indexed_color, rgb_color};

fn rgb(hue: f64, light: bool) -> (u8, u8, u8) {
    let value: Srgb<f64> = Srgb::from_color(Okhsl::new(hue.rem_euclid(360.0), 0.75, if light { 0.85 } else { 0.32 }));
    let channel = |n: f64| (n.clamp(0.0, 1.0) * 255.0).round() as u8;
    (channel(value.red), channel(value.green), channel(value.blue))
}

fn ansi256((r, g, b): (u8, u8, u8)) -> u8 {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let closest = |value: u8| CUBE.iter().enumerate().min_by_key(|(_, candidate)| value.abs_diff(**candidate)).map(|(index, _)| index).unwrap_or_default();
    let (ri, gi, bi) = (closest(r), closest(g), closest(b));
    let cube = (CUBE[ri], CUBE[gi], CUBE[bi]);
    let gray = (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)).round() as u8;
    let gray_index = (0..24_u8).min_by_key(|index| gray.abs_diff(8 + index * 10)).unwrap_or_default();
    let gray_value = 8 + gray_index * 10;
    let distance = |(target_r, target_g, target_b): (u8, u8, u8)| {
        let (dr, dg, db) = (f64::from(r) - f64::from(target_r), f64::from(g) - f64::from(target_g), f64::from(b) - f64::from(target_b));
        0.299 * dr * dr + 0.587 * dg * dg + 0.114 * db * db
    };
    if r.max(g).max(b) - r.min(g).min(b) < 10 && distance((gray_value, gray_value, gray_value)) < distance(cube) {
        232 + gray_index
    } else {
        16 + (36 * ri + 6 * gi + bi) as u8
    }
}

pub(crate) fn passage_color(hue: f64, light: bool, level: StdoutColorLevel) -> Color {
    let color = rgb(hue, light);
    if level == StdoutColorLevel::TrueColor { rgb_color(color) } else { indexed_color(ansi256(color)) }
}

/// Prefer unused, terminal-distinct hues separated by at least 24 degrees.
pub(crate) fn choose_hue(used: &[f64], level: StdoutColorLevel, random: f64) -> f64 {
    let unused: Vec<f64> = (0..360).map(f64::from).filter(|hue| !used.contains(hue)).collect();
    let distinct: Vec<f64> = unused.iter().copied().filter(|hue| used.iter().all(|other| {
        passage_color(*hue, false, level) != passage_color(*other, false, level)
            && passage_color(*hue, true, level) != passage_color(*other, true, level)
    })).collect();
    let available = if distinct.is_empty() { &unused } else { &distinct };
    let separated: Vec<f64> = available.iter().copied().filter(|hue| used.iter().all(|other| {
        let distance = (hue - other).abs(); distance.min(360.0 - distance) >= 24.0
    })).collect();
    let choices = if separated.is_empty() { available } else { &separated };
    if choices.is_empty() { return random.clamp(0.0, 1.0) * 360.0; }
    choices[((random.clamp(0.0, 1.0) * choices.len() as f64) as usize).min(choices.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_match_pi_reference_for_representative_hues() {
        for (hue, dark, light, dark256, light256) in [
            (0.0, (128, 32, 72), (249, 196, 211), 89, 224),
            (60.0, (107, 62, 20), (248, 202, 166), 58, 223),
            (120.0, (69, 81, 24), (200, 226, 111), 58, 185),
            (180.0, (26, 86, 76), (119, 237, 215), 23, 122),
            (240.0, (23, 80, 114), (178, 219, 249), 23, 153),
            (300.0, (90, 45, 145), (218, 204, 248), 54, 189),
        ] {
            assert_eq!(rgb(hue, false), dark);
            assert_eq!(rgb(hue, true), light);
            assert_eq!(ansi256(dark), dark256);
            assert_eq!(ansi256(light), light256);
        }
    }

    #[test]
    fn chooses_a_separated_hue_and_avoids_duplicate_terminal_colors() {
        let used = [0.0, 120.0, 240.0];
        let hue = choose_hue(&used, StdoutColorLevel::Ansi256, 0.5);
        assert!(used.iter().all(|other| {
            let delta = (hue - other).abs();
            delta.min(360.0 - delta) >= 24.0
                && passage_color(hue, false, StdoutColorLevel::Ansi256) != passage_color(*other, false, StdoutColorLevel::Ansi256)
                && passage_color(hue, true, StdoutColorLevel::Ansi256) != passage_color(*other, true, StdoutColorLevel::Ansi256)
        }));
    }
}
