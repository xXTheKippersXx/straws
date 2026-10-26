//! The window icon, drawn in code: a radiation pattern lobe over a dipole.

use eframe::egui::IconData;

use crate::colormap::viridis;

pub fn icon() -> IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 - c, c - y as f32);
            // Rounded dark square.
            let edge = (fx.abs().max(fy.abs()) - 26.0).max(0.0);
            let corner =
                ((fx.abs() - 22.0).max(0.0).powi(2) + (fy.abs() - 22.0).max(0.0).powi(2)).sqrt();
            if edge > 5.5 || corner > 9.5 {
                continue;
            }
            let mut px = [24u8, 28, 36, 255];
            // A cardioid lobe pointing up-right, coloured by "gain".
            let r = (fx * fx + fy * fy).sqrt();
            let a = fy.atan2(fx) - 0.6;
            let lobe = 24.0 * (0.5 + 0.5 * a.cos()).powf(1.5);
            if r < lobe {
                let col = viridis(
                    0.35 + 0.65 * (1.0 - r / lobe.max(1.0)) * 0.3 + 0.65 * (lobe / 24.0) * 0.7,
                );
                px = [col.r(), col.g(), col.b(), 255];
            }
            // The dipole: a light bar through the middle, feed in orange.
            if fy.abs() < 1.6 && fx.abs() < 22.0 {
                px = if fx.abs() < 2.5 {
                    [255, 140, 60, 255]
                } else {
                    [215, 222, 235, 255]
                };
            }
            rgba[(y * N + x) * 4..(y * N + x) * 4 + 4].copy_from_slice(&px);
        }
    }
    IconData {
        rgba,
        width: N as u32,
        height: N as u32,
    }
}
