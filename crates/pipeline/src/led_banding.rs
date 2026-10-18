//! Row-profile based correction for horizontal LED flicker/banding in rendered images.
//!
//! Operates on display-encoded RGBA output so it applies to JPEG-ready renders. It estimates a
//! robust per-row luminance and chroma profile, smooths that profile vertically, then attenuates
//! row deviations. This is a conservative single-frame heuristic: real horizontal stage lighting
//! can resemble banding, so strengths are user-adjustable and the feature is off by default.
use lightcraft_develop::LedBanding;
use lightcraft_raster::Rgba8;

/// Correct horizontal row-to-row exposure and colour shifts in-place.
pub fn correct(image: &mut Rgba8, settings: LedBanding) {
    let (w, h) = (image.width, image.height);
    if w < 8 || h < 3 || (settings.luminance <= 0.0 && settings.color <= 0.0) {
        return;
    }
    let lum_strength = (settings.luminance / 100.0).clamp(0.0, 1.0) as f32;
    let color_strength = (settings.color / 100.0).clamp(0.0, 1.0) as f32;
    let radius = (settings.band_width as usize).clamp(1, 64).min((h - 1) / 2).max(1);
    let mut lum = vec![0.0f32; h];
    let mut chroma = vec![[0.0f32; 3]; h];
    for y in 0..h {
        let mut l = 0.0f32;
        let mut c = [0.0f32; 3];
        let mut n = 0.0f32;
        for x in 0..w {
            let p = image.data[y * w + x];
            if p[3] == 0 { continue; }
            let rgb = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
            let yv = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            l += yv;
            for k in 0..3 { c[k] += rgb[k]; }
            n += 1.0;
        }
        if n > 0.0 {
            lum[y] = l / n;
            chroma[y] = c.map(|v| v / n);
        }
    }
    let smooth = |profile: &[f32], y: usize| -> f32 {
        let lo = y.saturating_sub(radius);
        let hi = (y + radius + 1).min(h);
        let mut sum = 0.0;
        let mut n = 0.0f32;
        for &v in &profile[lo..hi] { sum += v; n += 1.0; }
        sum / n.max(1.0)
    };
    let smooth_lum: Vec<f32> = (0..h).map(|y| smooth(&lum, y)).collect();
    let smooth_ch: [Vec<f32>; 3] = std::array::from_fn(|k| {
        let profile: Vec<f32> = chroma.iter().map(|v| v[k]).collect();
        (0..h).map(|y| smooth(&profile, y)).collect()
    });
    for y in 0..h {
        let source_l = lum[y];
        let target_l = smooth_lum[y];
        let l_gain = if source_l > 1.0 / 255.0 && target_l.is_finite() {
            (1.0 + ((target_l / source_l).clamp(0.5, 2.0) - 1.0) * lum_strength).clamp(0.5, 2.0)
        } else { 1.0 };
        let mut gains = [1.0f32; 3];
        if color_strength > 0.0 {
            for k in 0..3 {
                let src = chroma[y][k];
                let target = smooth_ch[k][y];
                if src > 2.0 / 255.0 && target.is_finite() {
                    gains[k] = (1.0 + ((target / src).clamp(0.7, 1.4) - 1.0) * color_strength).clamp(0.7, 1.4);
                }
            }
            // Colour correction should not double-apply luminance correction.
            let mean = (gains[0] + gains[1] + gains[2]) / 3.0;
            if mean > 0.0 { for g in &mut gains { *g /= mean; } }
        }
        for x in 0..w {
            let p = &mut image.data[y * w + x];
            for k in 0..3 {
                let v = p[k] as f32 * l_gain * gains[k];
                p[k] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_raster::Rgba8;

    #[test]
    fn corrects_synthetic_horizontal_luminance_bands() {
        let mut img = Rgba8::from_fn(64, 48, |_, y| {
            let v = if y % 8 < 4 { 70 } else { 130 };
            [v, v, v, 255]
        });
        correct(&mut img, LedBanding { enabled: true, luminance: 100.0, color: 0.0, band_width: 8 });
        let row_mean = |y: usize| img.row(y).iter().map(|p| p[0] as f32).sum::<f32>() / 64.0;
        assert!((row_mean(2) - row_mean(6)).abs() < 45.0);
    }

    #[test]
    fn neutral_settings_leave_pixels_untouched() {
        let mut img = Rgba8::from_fn(8, 8, |x, y| [(x * 20) as u8, (y * 20) as u8, 80, 255]);
        let before = img.clone();
        correct(&mut img, LedBanding { luminance: 0.0, color: 0.0, ..LedBanding::default() });
        assert_eq!(img.data, before.data);
    }
}
