//! Display transform: a camera-like tone curve that keeps hue.
//!
//! The curve (Lottes 2016 form, `x^a / (x^(a*d) * b + c)`) is fitted to
//! the default JPEG response of a stills camera around a metered exposure
//! (see [`crate::exposure`]), with the slightly deeper toe of a "standard
//! plus" picture style. In 8-bit sRGB, mid grey 0.18 shows at ~128, -3
//! stops at ~28, -5 stops at ~4 (real blacks, no lifted toe), +1 stop at
//! ~176, +2 stops at ~218, and white arrives ~+3.4 stops over mid grey, so
//! sunlit white walls and the sky near the sun can reach the top.
//!
//! The curve runs on `max(r, g, b)` and the colour keeps its RGB ratios, so
//! hue and saturation survive the tone mapping (AgX desaturated sunlit
//! ochre from 0.47 to 0.27). Colours only fade to white as the sensor
//! "saturates": the desaturation grows with how much the curve compressed
//! the peak. The shader in `post.wgsl` mirrors [`ToneCurve::map`].

use forja_math::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct ToneCurve {
    /// Log-log slope of the toe (shadow contrast).
    pub contrast: f32,
    /// Shoulder shape (1 = no shoulder).
    pub shoulder: f32,
    /// Scene value (after exposure) that maps to display white.
    pub white: f32,
    /// Scene mid grey and the display-linear value it maps to.
    pub mid_in: f32,
    pub mid_out: f32,
    /// Chroma scale around luma, like a camera's colour profile.
    pub saturation: f32,
    /// How fast colours whiten as the curve compresses them.
    pub highlight_desaturation: f32,
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self { contrast: 1.55, shoulder: 0.92, white: 1.9, mid_in: 0.18, mid_out: 0.215, saturation: 1.25, highlight_desaturation: 0.25 }
    }
}

impl ToneCurve {
    /// Shader constants: `[a, a * d, b, c]`.
    pub fn coefficients(&self) -> [f32; 4] {
        let (a, d, w, mi, mo) = (self.contrast as f64, self.shoulder as f64, self.white as f64, self.mid_in as f64, self.mid_out as f64);
        let den = (w.powf(a * d) - mi.powf(a * d)) * mo;
        let b = (-mi.powf(a) + w.powf(a) * mo) / den;
        let c = (w.powf(a * d) * mi.powf(a) - w.powf(a) * mi.powf(a * d) * mo) / den;
        [a as f32, (a * d) as f32, b as f32, c as f32]
    }

    /// Display-linear value of a neutral scene value.
    pub fn map_grey(&self, x: f32) -> f32 {
        let [a, ad, b, c] = self.coefficients();
        let x = x.max(0.0);
        (x.powf(a) / (x.powf(ad) * b + c)).clamp(0.0, 1.0)
    }

    /// CPU reference of the shader transform (scene-referred, exposed
    /// linear Rec.709 in, display-linear out).
    pub fn map(&self, c: Vec3) -> Vec3 {
        let c = c.max(Vec3::ZERO);
        let peak = c.max_element();
        if peak <= 1e-8 {
            return Vec3::ZERO;
        }
        let t = self.map_grey(peak);
        let mut ratio = c / peak;
        let l = ratio.dot(LUMA);
        ratio = (Vec3::splat(l) + (ratio - Vec3::splat(l)) * self.saturation).max(Vec3::ZERO);
        ratio /= ratio.max_element().max(1e-6);
        let g = 1.0 - 1.0 / (self.highlight_desaturation * (peak - t).max(0.0) + 1.0);
        ratio = ratio.lerp(Vec3::ONE, g);
        ratio * t
    }
}

const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

#[cfg(test)]
mod tests {
    use super::*;

    fn srgb8(x: f32) -> f32 {
        let e = if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        e * 255.0
    }

    #[test]
    fn camera_like_grey_ramp() {
        let t = ToneCurve::default();
        let at = |stops: f32| srgb8(t.map_grey(0.18 * stops.exp2()));
        // Mid grey near sRGB 120, real blacks, highlights that reach white.
        assert!((at(0.0) - 128.0).abs() < 3.0, "{}", at(0.0));
        assert!(at(-5.0) < 9.0 && at(-5.0) > 2.0, "{}", at(-5.0));
        assert!((at(-3.0) - 28.0).abs() < 4.0, "{}", at(-3.0));
        assert!((at(2.0) - 218.0).abs() < 5.0, "{}", at(2.0));
        assert!(at(3.45) > 252.0, "{}", at(3.45));
        let mut prev = 0.0;
        for i in 0..400 {
            let v = t.map_grey(i as f32 * 0.01);
            assert!(v >= prev, "not monotonic at {i}");
            prev = v;
        }
    }

    #[test]
    fn keeps_hue_and_saturation() {
        let t = ToneCurve { saturation: 1.0, ..Default::default() };
        // A sunlit ochre wall: ratios survive, only brightness is mapped.
        let ochre = Vec3::new(0.62, 0.40, 0.20);
        let out = t.map(ochre);
        let (a, b) = (ochre / ochre.max_element(), out / out.max_element());
        assert!((a - b).abs().max_element() < 0.01, "{a} vs {b}");
        // Grey stays grey.
        let g = t.map(Vec3::splat(0.3));
        assert!((g.x - g.y).abs() < 1e-5 && (g.y - g.z).abs() < 1e-5);
        // Far over white, colours converge to white like a saturated sensor.
        let hot = t.map(Vec3::new(40.0, 20.0, 10.0));
        assert!(hot.min_element() > 0.85, "{hot}");
    }
}
