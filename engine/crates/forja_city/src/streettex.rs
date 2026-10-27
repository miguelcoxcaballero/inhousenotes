//! Procedural textures of the street furniture: moulded HDPE of the waste
//! containers (stipple, sun fading, grime and scuffs), the sticker atlas of
//! the municipal waste service (fraction labels, reflective chevrons,
//! information panels, a generic city emblem, stickers and tags) and the
//! faces of the traffic-signal accessories (push-button panel, countdown,
//! bicycle lenses). All generic: no real trademarks.

use forja_assets::TextureData;
use forja_math::Vec2;

use crate::geom::Rng;
use crate::noise::{fbm, fbm_aniso, hashf, smoothstep};
use crate::texgen::{Canvas, GenSet, rgb};

/// Waste fractions as the Valencian service colours them.
pub const FRACTIONS: usize = 5;
/// Fraction index: yellow packaging.
pub const ENVASES: u8 = 0;
/// Fraction index: blue paper and cardboard.
pub const PAPEL: u8 = 1;
/// Fraction index: green glass.
pub const VIDRIO: u8 = 2;
/// Fraction index: grey general waste.
pub const RESTO: u8 = 3;
/// Fraction index: brown organic waste.
pub const ORGANICO: u8 = 4;

/// Lid / band colour of each fraction (sRGB), measured on the reference
/// photos in daylight: the pigments are a little desaturated by UV.
pub const FRACTION_RGB: [u32; FRACTIONS] = [0xf0c419, 0x1f63b8, 0x3aa845, 0x9a9e9f, 0x8a5a36];
/// Body grey of the current containers (the moulded grey HDPE).
pub const BODY_RGB: u32 = 0x5a5e60;

const NAMES: [(&str, &str); FRACTIONS] = [("ENVASOS", "ENVASES"), ("PAPER I CARTÓ", "PAPEL Y CARTÓN"), ("VIDRE", "VIDRIO"), ("RESTA", "RESTO"), ("ORGÀNIC", "ORGÁNICO")];

/// Moulded HDPE, one repeat per metre: a fine orange-peel stipple (normal),
/// UV fading in broad patches, rain-washed grime streaks running down,
/// whitish scuffs and scratches (lighter and rougher), grey-scale so the
/// fraction colour is the material tint.
pub fn hdpe() -> GenSet {
    let n = 512u32;
    let mut c = Canvas::new(n, n, [1.0; 4]);
    let mut height = vec![0.0f32; (n * n) as usize];
    let mut rough = vec![0.0f32; (n * n) as usize];
    let mut rng = Rng::new(0x4d9e);
    let scratches: Vec<(Vec2, Vec2, f32)> = (0..70)
        .map(|_| {
            let a = Vec2::new(rng.f(), rng.f());
            let ang = rng.range(-0.5, 0.5) + if rng.chance(0.5) { 0.0 } else { std::f32::consts::FRAC_PI_2 };
            let l = rng.range(0.02, 0.14);
            (a, a + Vec2::from_angle(ang) * l, rng.range(0.6, 1.6) / n as f32)
        })
        .collect();
    for y in 0..n {
        let v = (y as f32 + 0.5) / n as f32;
        for x in 0..n {
            let u = (x as f32 + 0.5) / n as f32;
            let i = (y * n + x) as usize;
            let stipple = fbm(u, v, 128, 2, 3);
            // Fine stipple plus a soft moulding waviness.
            height[i] = stipple * 0.6 + fbm(u, v, 8, 3, 21) * 2.5;
            let fade = smoothstep(0.45, 0.8, fbm(u, v, 3, 4, 5));
            let streak = smoothstep(0.5, 0.8, fbm_aniso(u, v, 18, 2, 3, 7));
            let grime = smoothstep(0.55, 0.85, fbm(u, v, 6, 4, 9));
            let mut scuff = 0.0f32;
            let p = Vec2::new(u, v);
            for (a, b, w) in &scratches {
                let d = crate::texgen::sd_segment(p, *a, *b);
                scuff = scuff.max(smoothstep(*w * 1.5, 0.0, d));
            }
            let speck = if hashf(x as i32, y as i32, 11) > 0.997 { 0.12 } else { 0.0 };
            let g = (0.88 + 0.07 * fade - 0.13 * streak - 0.1 * grime + 0.1 * scuff - speck + 0.03 * (stipple - 0.5)).clamp(0.4, 1.0);
            c.px[i] = [g, g * 0.995, g * 0.985, 1.0];
            rough[i] = 0.5 + 0.12 * fade + 0.15 * grime + 0.2 * scuff;
        }
    }
    // Gentle: moulded HDPE reads satin, not stucco (cycle-2 review).
    let normal = normal_from_height(&height, n, n, 0.28);
    GenSet { color: c.to_texture("hdpe", true), normal: Some(normal), orm: Some(orm_from(&rough, n, n)) }
}

fn normal_from_height(h: &[f32], w: u32, hh: u32, strength: f32) -> TextureData {
    let at = |x: i64, y: i64| h[((y.rem_euclid(hh as i64)) * w as i64 + x.rem_euclid(w as i64)) as usize];
    let mut rgba = Vec::with_capacity((w * hh * 4) as usize);
    for y in 0..hh as i64 {
        for x in 0..w as i64 {
            let dx = (at(x + 1, y) - at(x - 1, y)) * strength;
            let dy = (at(x, y + 1) - at(x, y - 1)) * strength;
            let nl = (dx * dx + dy * dy + 1.0).sqrt();
            let (nx, ny, nz) = (-dx / nl, dy / nl, 1.0 / nl);
            rgba.extend([((nx * 0.5 + 0.5) * 255.0) as u8, ((ny * 0.5 + 0.5) * 255.0) as u8, ((nz * 0.5 + 0.5) * 255.0) as u8, 255]);
        }
    }
    TextureData { name: "hdpe_normal".into(), width: w, height: hh, rgba8: std::sync::Arc::from(rgba), srgb: false }
}

fn orm_from(rough: &[f32], w: u32, h: u32) -> TextureData {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for r in rough {
        rgba.extend([255u8, (r.clamp(0.0, 1.0) * 255.0) as u8, 0, 255]);
    }
    TextureData { name: "hdpe_orm".into(), width: w, height: h, rgba8: std::sync::Arc::from(rgba), srgb: false }
}

/// Regions of the sticker atlas ([`street_atlas`]), in UV (0..1, v down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    /// Big fraction sticker (pictograms and bilingual name), portrait.
    Fraction(u8),
    /// Wide fraction band: coloured strip with the name (lid fronts,
    /// underground mouths).
    Band(u8),
    /// Red and white retro-reflective chevron strip (portrait use: rotate).
    Chevron,
    /// Teal "thank you for separating" pill.
    Thanks,
    /// Generic city emblem: white crowned shield on transparent.
    Emblem,
    /// Old white information sticker with rules and pictograms.
    Info,
    /// Round recycling sticker.
    Round,
    /// Graffiti tag on transparent.
    Tag(u8),
    /// Litter-bin plate: emblem and "NETA" slogan on dark green.
    BinPlate,
    /// Traffic-signal push-button panel ("PULSE / ESPERE VERDE").
    Button,
    /// Small fly-posted sticker (bands, flats to let, etc.).
    Sticker(u8),
}

impl Label {
    /// `(min, max)` UV of the region.
    pub fn uv(self) -> (Vec2, Vec2) {
        let px = |x0: f32, y0: f32, x1: f32, y1: f32| (Vec2::new(x0, y0) / ATLAS as f32, Vec2::new(x1, y1) / ATLAS as f32);
        match self {
            Label::Fraction(f) => {
                let x = (f as usize % FRACTIONS) as f32 * 192.0;
                px(x + 2.0, 2.0, x + 190.0, 254.0)
            }
            Label::Band(f) => {
                let y = 256.0 + (f as usize % FRACTIONS) as f32 * 64.0;
                px(2.0, y + 2.0, 510.0, y + 62.0)
            }
            Label::Chevron => px(512.0, 256.0, 1024.0, 320.0),
            Label::Thanks => px(514.0, 322.0, 1022.0, 382.0),
            Label::Emblem => px(512.0, 384.0, 640.0, 512.0),
            Label::Info => px(642.0, 386.0, 894.0, 638.0),
            Label::Round => px(896.0, 384.0, 1024.0, 512.0),
            Label::Tag(k) => {
                let x = (k % 3) as f32 * 341.0;
                px(x, 640.0, x + 340.0, 832.0)
            }
            Label::BinPlate => px(0.0, 832.0, 256.0, 1024.0),
            Label::Button => px(256.0, 832.0, 384.0, 1024.0),
            Label::Sticker(k) => {
                let x = 512.0 + (k % 4) as f32 * 128.0;
                px(x + 4.0, 836.0, x + 124.0, 1020.0)
            }
        }
    }
}

const ATLAS: u32 = 1024;

/// Pictogram of a fraction drawn in `col` inside the box `min..max`.
fn pictogram(c: &mut Canvas, f: u8, ctr: Vec2, s: f32, col: [f32; 4]) {
    match f {
        ENVASES => {
            // Bottle, can and carton.
            c.rect(ctr + Vec2::new(-0.9, -0.2) * s, ctr + Vec2::new(-0.55, 0.8) * s, col);
            c.rect(ctr + Vec2::new(-0.8, -0.45) * s, ctr + Vec2::new(-0.65, -0.2) * s, col);
            c.rect(ctr + Vec2::new(-0.35, 0.1) * s, ctr + Vec2::new(0.05, 0.8) * s, col);
            c.poly(&[ctr + Vec2::new(0.3, 0.8) * s, ctr + Vec2::new(0.9, 0.8) * s, ctr + Vec2::new(0.9, -0.05) * s, ctr + Vec2::new(0.6, -0.35) * s, ctr + Vec2::new(0.3, -0.05) * s], col);
        }
        PAPEL => {
            // Folded box and a newspaper.
            c.poly(&[ctr + Vec2::new(-0.9, 0.0) * s, ctr + Vec2::new(0.1, 0.0) * s, ctr + Vec2::new(0.1, 0.8) * s, ctr + Vec2::new(-0.9, 0.8) * s], col);
            c.poly(&[ctr + Vec2::new(-0.9, 0.0) * s, ctr + Vec2::new(-0.6, -0.35) * s, ctr + Vec2::new(0.4, -0.35) * s, ctr + Vec2::new(0.1, 0.0) * s], col);
            c.rect(ctr + Vec2::new(0.3, -0.1) * s, ctr + Vec2::new(0.9, 0.8) * s, col);
        }
        VIDRIO => {
            c.rect(ctr + Vec2::new(-0.6, -0.2) * s, ctr + Vec2::new(-0.2, 0.8) * s, col);
            c.rect(ctr + Vec2::new(-0.48, -0.6) * s, ctr + Vec2::new(-0.32, -0.2) * s, col);
            c.rect(ctr + Vec2::new(0.1, 0.2) * s, ctr + Vec2::new(0.6, 0.8) * s, col);
        }
        RESTO => {
            // Tied rubbish bag.
            c.circle(ctr + Vec2::new(0.0, 0.3) * s, 0.55 * s, col);
            c.poly(&[ctr + Vec2::new(-0.2, -0.2) * s, ctr + Vec2::new(0.2, -0.2) * s, ctr + Vec2::new(0.35, -0.6) * s, ctr + Vec2::new(-0.35, -0.6) * s], col);
        }
        _ => {
            // Apple core and a leaf.
            c.circle(ctr + Vec2::new(-0.3, 0.3) * s, 0.45 * s, col);
            c.rect(ctr + Vec2::new(-0.34, -0.45) * s, ctr + Vec2::new(-0.26, -0.1) * s, col);
            c.poly(&[ctr + Vec2::new(0.3, 0.8) * s, ctr + Vec2::new(0.9, 0.2) * s, ctr + Vec2::new(0.6, -0.3) * s, ctr + Vec2::new(0.2, 0.2) * s], col);
        }
    }
}

/// Generic crowned shield: three bars (not any real coat of arms) under a
/// simple crown, filled with `col`.
fn emblem(c: &mut Canvas, ctr: Vec2, s: f32, col: [f32; 4], bg: [f32; 4]) {
    let (hw, top, bot) = (0.42 * s, -0.25 * s, 0.75 * s);
    c.fill(ctr + Vec2::new(-hw, top), ctr + Vec2::new(hw, bot), col, |p| {
        let q = p - ctr;
        let body = crate::texgen::sd_box(q - Vec2::new(0.0, (top + bot * 0.5) * 0.5), Vec2::new(hw, (bot * 0.5 - top) * 0.5));
        let tip = (q - Vec2::new(0.0, bot * 0.35)).length() - hw;
        body.min(if q.y > bot * 0.3 { tip } else { f32::MAX })
    });
    for k in 0..3 {
        let x = -0.25 * s + k as f32 * 0.25 * s;
        c.rect(ctr + Vec2::new(x - 0.06 * s, top + 0.12 * s), ctr + Vec2::new(x + 0.06 * s, 0.55 * s), bg);
    }
    // Crown.
    c.rect(ctr + Vec2::new(-0.36 * s, -0.42 * s), ctr + Vec2::new(0.36 * s, -0.3 * s), col);
    for k in 0..5 {
        let x = -0.32 * s + k as f32 * 0.16 * s;
        c.circle(ctr + Vec2::new(x, -0.52 * s), 0.06 * s, col);
    }
    // Bat wings (heraldic flourish) as two arcs.
    c.ring(ctr + Vec2::new(-0.62 * s, 0.1 * s), 0.18 * s, 0.05 * s, col);
    c.ring(ctr + Vec2::new(0.62 * s, 0.1 * s), 0.18 * s, 0.05 * s, col);
}

/// 1024 × 1024 sticker atlas, sRGB with alpha (tags and emblem are cut
/// out; everything else is opaque). See [`Label::uv`].
pub fn street_atlas() -> TextureData {
    // Largest size up to `max` whose rendered width stays within `room`.
    let fit = |s: &str, max: f32, room: f32| {
        let w = Canvas::text_width(s, max);
        if w > room { max * room / w } else { max }
    };
    let mut c = Canvas::new(ATLAS, ATLAS, [0.0, 0.0, 0.0, 0.0]);
    let white = rgb(0xf2f2ee);
    let ink = rgb(0x262a2c);
    for f in 0..FRACTIONS as u8 {
        let col = rgb(FRACTION_RGB[f as usize]);
        let dark_text = f == ENVASES || f == RESTO;
        let tc = if dark_text { ink } else { white };
        // Fraction sticker: coloured header, white field with pictograms.
        let x = f as f32 * 192.0;
        c.rect(Vec2::new(x + 2.0, 2.0), Vec2::new(x + 190.0, 254.0), white);
        c.rect(Vec2::new(x + 2.0, 2.0), Vec2::new(x + 190.0, 80.0), col);
        let (va, es) = NAMES[f as usize];
        let size = fit(va, 26.0, 172.0);
        c.text_centered(va, Vec2::new(x + 96.0, 30.0), size, 1.1, tc);
        let size2 = fit(es, 16.0, 172.0);
        c.text_centered(es, Vec2::new(x + 96.0, 62.0), size2, 0.9, tc);
        pictogram(&mut c, f, Vec2::new(x + 96.0, 140.0), 50.0, col);
        // "Yes" and "no" rows: small pictograms and lines of text.
        for k in 0..4 {
            let y = 200.0 + k as f32 * 12.0;
            c.rect(Vec2::new(x + 20.0, y), Vec2::new(x + 20.0 + 60.0 + ((k * 37) % 70) as f32, y + 5.0), rgb(0x8a8e90));
        }
        c.circle(Vec2::new(x + 160.0, 222.0), 16.0, col);
        // Wide band.
        let y = 256.0 + f as f32 * 64.0;
        c.rect(Vec2::new(2.0, y + 2.0), Vec2::new(510.0, y + 62.0), col);
        c.rect(Vec2::new(10.0, y + 8.0), Vec2::new(60.0, y + 56.0), white);
        pictogram(&mut c, f, Vec2::new(35.0, y + 34.0), 18.0, col);
        let label = format!("{va} · {es}");
        let size = fit(&label, 30.0, 420.0);
        c.text_centered(&label, Vec2::new(285.0, y + 32.0), size, 1.1, tc);
    }
    // Chevron strip, drawn lying down: used on vertical corner strips
    // (13 × 50 cm) with u running up the strip, so the "<" shapes read as
    // stacked V chevrons pointing down. Arms are 2x longer along u to
    // stay at 45 degrees after that stretch.
    {
        let (x0, y0, x1, y1) = (512.0, 256.0, 1024.0, 320.0);
        c.rect(Vec2::new(x0, y0), Vec2::new(x1, y1), rgb(0xf4f2ec));
        let red = rgb(0xd0202a);
        let mid = (y0 + y1) * 0.5;
        let mut k = x0;
        while k < x1 {
            c.poly(&[Vec2::new(k, mid), Vec2::new(k + 66.0, y0), Vec2::new(k + 130.0, y0), Vec2::new(k + 64.0, mid)], red);
            c.poly(&[Vec2::new(k, mid), Vec2::new(k + 64.0, mid), Vec2::new(k + 130.0, y1), Vec2::new(k + 66.0, y1)], red);
            k += 128.0;
        }
    }
    // Thanks pill.
    {
        let teal = rgb(0x2a9d96);
        c.fill(Vec2::new(514.0, 322.0), Vec2::new(1022.0, 382.0), teal, |p| crate::texgen::sd_box(p - Vec2::new(768.0, 352.0), Vec2::new(225.0, 1.0)) - 28.0);
        c.text_centered("GRÀCIES PER SEPARAR", Vec2::new(768.0, 345.0), 20.0, 1.0, white);
        c.text_centered("PER UNA CIUTAT MÉS NETA", Vec2::new(768.0, 368.0), 12.0, 0.9, white);
    }
    emblem(&mut c, Vec2::new(576.0, 452.0), 100.0, [0.95, 0.95, 0.93, 1.0], [0.0, 0.0, 0.0, 0.0]);
    // Old information sticker.
    {
        let (x0, y0) = (642.0, 386.0);
        c.rect(Vec2::new(x0, y0), Vec2::new(x0 + 252.0, y0 + 252.0), rgb(0xeeeee8));
        c.rect(Vec2::new(x0, y0), Vec2::new(x0 + 252.0, y0 + 40.0), rgb(0x2b6e45));
        c.text_centered("RESIDUS", Vec2::new(x0 + 126.0, y0 + 20.0), 22.0, 1.1, white);
        for r in 0..3 {
            for k in 0..3 {
                let ctr = Vec2::new(x0 + 46.0 + k as f32 * 80.0, y0 + 80.0 + r as f32 * 62.0);
                c.ring(ctr, 22.0, 4.0, rgb(0x5a5e60));
                pictogram(&mut c, ((r * 3 + k) % 5) as u8, ctr, 13.0, rgb(0x5a5e60));
                if (r + k) % 3 == 0 {
                    c.line(ctr + Vec2::new(-16.0, -16.0), ctr + Vec2::new(16.0, 16.0), 5.0, rgb(0xc62828));
                }
            }
        }
        c.text_centered("HORARI 20:00 - 23:00", Vec2::new(x0 + 126.0, y0 + 238.0), 14.0, 1.0, ink);
    }
    // Round recycling sticker: three chasing arrows.
    {
        let ctr = Vec2::new(960.0, 448.0);
        c.circle(ctr, 60.0, rgb(0x2a9d96));
        for k in 0..3 {
            let a = k as f32 * std::f32::consts::TAU / 3.0;
            let p0 = ctr + Vec2::from_angle(a) * 32.0;
            let p1 = ctr + Vec2::from_angle(a + 1.6) * 32.0;
            c.line(p0, p1, 9.0, white);
            let tip = ctr + Vec2::from_angle(a + 1.9) * 32.0;
            c.poly(&[p1 + Vec2::from_angle(a + 1.6) * 14.0, p1 - Vec2::from_angle(a + 1.6) * 14.0, tip], white);
        }
    }
    // Graffiti tags.
    let mut rng = Rng::new(0x7a65);
    for k in 0..3 {
        let x0 = k as f32 * 341.0;
        let col = [rgb(0x111111), rgb(0xc0392b), rgb(0x1e88e5)][k];
        let outline = [rgb(0xf5f5f5), rgb(0x111111), rgb(0xffeb3b)][k];
        let mut prev = Vec2::new(x0 + 30.0, 760.0);
        let pts: Vec<Vec2> = (0..14).map(|i| Vec2::new(x0 + 30.0 + i as f32 * 21.0, 700.0 + rng.range(-40.0, 60.0))).collect();
        for w in [16.0f32, 9.0] {
            prev = pts[0];
            for p in &pts[1..] {
                c.line(prev, *p, w, if w > 10.0 { outline } else { col });
                prev = *p;
            }
        }
        let _ = prev;
        c.line(Vec2::new(x0 + 40.0, 800.0), Vec2::new(x0 + 300.0, 790.0), 5.0, col);
    }
    // Litter-bin plate.
    {
        let green = rgb(0x24443a);
        c.rect(Vec2::new(0.0, 832.0), Vec2::new(256.0, 1024.0), green);
        emblem(&mut c, Vec2::new(128.0, 905.0), 80.0, white, green);
        c.text_centered("CIUTAT NETA", Vec2::new(128.0, 992.0), 22.0, 1.1, white);
    }
    // Push-button panel: yellow box face with a pedestrian and the
    // instructions above a black button ring.
    {
        let (x0, y0) = (256.0, 832.0);
        c.rect(Vec2::new(x0, y0), Vec2::new(x0 + 128.0, y0 + 192.0), rgb(0xf2c200));
        c.text_centered("PULSE", Vec2::new(x0 + 64.0, y0 + 22.0), 18.0, 1.1, ink);
        c.text_centered("ESPERE VERDE", Vec2::new(x0 + 64.0, y0 + 44.0), 11.0, 1.0, ink);
        c.circle(Vec2::new(x0 + 64.0, y0 + 100.0), 30.0, ink);
        c.circle(Vec2::new(x0 + 64.0, y0 + 100.0), 22.0, rgb(0x3a3e40));
        // Lamp "ESPERE" window.
        c.rect(Vec2::new(x0 + 24.0, y0 + 145.0), Vec2::new(x0 + 104.0, y0 + 175.0), ink);
        c.text_centered("ESPERE", Vec2::new(x0 + 64.0, y0 + 160.0), 12.0, 1.0, rgb(0x6a4a20));
    }
    // Fly-posted stickers.
    let sticker_cols = [(0xfafafa, 0xd32f2f), (0x111111, 0xf5f5f5), (0xffeb3b, 0x111111), (0x1565c0, 0xffffff)];
    let words = ["LLOGUE", "PUNK", "SE VENDE", "FESTA"];
    for k in 0..4 {
        let x = 512.0 + k as f32 * 128.0;
        let (bg, fg) = sticker_cols[k];
        c.rect(Vec2::new(x + 4.0, 836.0), Vec2::new(x + 124.0, 1020.0), rgb(bg));
        c.text_centered(words[k], Vec2::new(x + 64.0, 880.0), fit(words[k], 18.0, 106.0), 1.1, rgb(fg));
        for r in 0..5 {
            c.rect(Vec2::new(x + 16.0, 920.0 + r as f32 * 16.0), Vec2::new(x + 112.0 - (r * 13 % 40) as f32, 926.0 + r as f32 * 16.0), rgb(fg));
        }
    }
    c.to_texture("street_atlas", true)
}

/// Bicycle signal lens (red, amber or green cyclist on black).
pub fn bike_lens(color: u32) -> TextureData {
    let mut c = Canvas::new(128, 128, rgb(0x050505));
    let col = rgb(color);
    c.ring(Vec2::new(34.0, 86.0), 20.0, 7.0, col);
    c.ring(Vec2::new(94.0, 86.0), 20.0, 7.0, col);
    c.line(Vec2::new(34.0, 86.0), Vec2::new(56.0, 56.0), 6.0, col);
    c.line(Vec2::new(56.0, 56.0), Vec2::new(86.0, 56.0), 6.0, col);
    c.line(Vec2::new(86.0, 56.0), Vec2::new(94.0, 86.0), 6.0, col);
    c.line(Vec2::new(56.0, 56.0), Vec2::new(64.0, 86.0), 6.0, col);
    c.line(Vec2::new(64.0, 86.0), Vec2::new(34.0, 86.0), 6.0, col);
    c.line(Vec2::new(52.0, 46.0), Vec2::new(62.0, 46.0), 6.0, col);
    c.line(Vec2::new(86.0, 56.0), Vec2::new(82.0, 42.0), 6.0, col);
    c.circle(Vec2::new(70.0, 24.0), 9.0, col);
    c.to_texture("bike_lens", true)
}

/// Pedestrian countdown display: two amber/green seven-segment digits.
pub fn countdown(secs: u8, color: u32) -> TextureData {
    let mut c = Canvas::new(128, 64, rgb(0x070707));
    let col = rgb(color);
    let dim = [col[0] * 0.08, col[1] * 0.08, col[2] * 0.08, 1.0];
    let segs = |d: u8| -> [bool; 7] {
        match d {
            0 => [true, true, true, false, true, true, true],
            1 => [false, false, true, false, false, true, false],
            2 => [true, false, true, true, true, false, true],
            3 => [true, false, true, true, false, true, true],
            4 => [false, true, true, true, false, true, false],
            5 => [true, true, false, true, false, true, true],
            6 => [true, true, false, true, true, true, true],
            7 => [true, false, true, false, false, true, false],
            8 => [true; 7],
            _ => [true, true, true, true, false, true, true],
        }
    };
    for (k, d) in [(0, secs / 10), (1, secs % 10)] {
        let x = 22.0 + k as f32 * 50.0;
        let on = segs(d);
        // a top, b upper-left, c upper-right, d middle, e lower-left, f lower-right, g bottom.
        let lines = [((0.0, 0.0), (28.0, 0.0)), ((0.0, 0.0), (0.0, 22.0)), ((28.0, 0.0), (28.0, 22.0)), ((0.0, 22.0), (28.0, 22.0)), ((0.0, 22.0), (0.0, 44.0)), ((28.0, 22.0), (28.0, 44.0)), ((0.0, 44.0), (28.0, 44.0))];
        for (i, ((ax, ay), (bx, by))) in lines.iter().enumerate() {
            c.line(Vec2::new(x + ax, 10.0 + ay), Vec2::new(x + bx, 10.0 + by), 6.0, if on[i] { col } else { dim });
        }
    }
    c.to_texture("countdown", true)
}
