//! Colour maps that stay readable for colour-blind users.

use eframe::egui::Color32;

/// Viridis, sampled at 16 points (from matplotlib's definition) and
/// interpolated in between.
const VIRIDIS: [[u8; 3]; 16] = [
    [68, 1, 84],
    [72, 26, 108],
    [71, 47, 125],
    [65, 68, 135],
    [57, 86, 140],
    [49, 104, 142],
    [42, 120, 142],
    [35, 136, 142],
    [31, 152, 139],
    [34, 168, 132],
    [53, 183, 121],
    [84, 197, 104],
    [122, 209, 81],
    [165, 219, 54],
    [210, 226, 27],
    [253, 231, 37],
];

/// Viridis colour for `t` in 0..=1 (dark purple to yellow).
pub fn viridis(t: f32) -> Color32 {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let x = t * (VIRIDIS.len() - 1) as f32;
    let i = (x.floor() as usize).min(VIRIDIS.len() - 2);
    let f = x - i as f32;
    let a = VIRIDIS[i];
    let b = VIRIDIS[i + 1];
    let mix = |k: usize| (f32::from(a[k]) + (f32::from(b[k]) - f32::from(a[k])) * f).round() as u8;
    Color32::from_rgb(mix(0), mix(1), mix(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_and_clamping() {
        assert_eq!(viridis(0.0), Color32::from_rgb(68, 1, 84));
        assert_eq!(viridis(1.0), Color32::from_rgb(253, 231, 37));
        assert_eq!(viridis(-3.0), viridis(0.0));
        assert_eq!(viridis(f32::NAN), viridis(0.0));
    }
}
