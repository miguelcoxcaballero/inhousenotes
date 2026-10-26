//! Leaf cards and fruit on the leafy stems of a skeleton.

use forja_math::Vec3;

use crate::mesh::{Card, Rect};
use crate::rng::Rng;
use crate::skeleton::{Skeleton, StemKind, rotate};
use crate::species::{FruitKind, Species};
use crate::texture::{FoliageAtlas, FoliageStyle, SlotKind};

#[derive(Clone, Copy, Debug)]
pub struct Fruit {
    pub center: Vec3,
    pub radii: Vec3,
    pub wind: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Foliage {
    pub cards: Vec<Card>,
    pub fruits: Vec<Fruit>,
    pub crown_center: Vec3,
    pub crown_radii: Vec3,
}

/// Wind sway weight along a stem: 0 at the ground, growing with height and
/// branch order (trunks barely move, twig tips move most).
pub fn stem_wind(sk: &Skeleton) -> Vec<Box<dyn Fn(f32) -> f32 + Send + Sync>> {
    let h = sk.height.max(1.0);
    let mut base = vec![0.0f32; sk.stems.len()];
    let mut out: Vec<Box<dyn Fn(f32) -> f32 + Send + Sync>> = Vec::with_capacity(sk.stems.len());
    for (i, s) in sk.stems.iter().enumerate() {
        let b = match s.parent {
            Some(p) => {
                let np = sk.stems[p].pts.len().max(2) as f32 - 1.0;
                let pt = (s.at / np).clamp(0.0, 1.0);
                let ps = &sk.stems[p];
                stem_w(ps.kind, ps.level, base[p], pt, ps.pts.first().map(|p| p.y).unwrap_or(0.0), ps.pts.last().map(|p| p.y).unwrap_or(0.0), h)
            }
            None => 0.0,
        };
        base[i] = b;
        let (kind, level) = (s.kind, s.level);
        let (y0, y1) = (s.pts.first().map(|p| p.y).unwrap_or(0.0), s.pts.last().map(|p| p.y).unwrap_or(0.0));
        out.push(Box::new(move |t| stem_w(kind, level, b, t, y0, y1, h)));
    }
    out
}

fn stem_w(kind: StemKind, level: u8, base: f32, t: f32, y0: f32, y1: f32, h: f32) -> f32 {
    match kind {
        StemKind::Trunk => {
            let y = (y0 + (y1 - y0) * t).max(0.0) / h;
            (0.22 * y * y).min(0.3)
        }
        StemKind::Root => 0.0,
        StemKind::Aerial => {
            if y1 < 0.0 { 0.0 } else { (base + 0.5 * t).min(1.0) }
        }
        StemKind::Branch => {
            let flex = [0.0, 0.16, 0.24, 0.3, 0.34][level.min(4) as usize];
            (base + flex * t.powf(1.5)).min(0.95)
        }
    }
}

/// Places leaf cards along leafy stems.
pub fn place(sk: &Skeleton, st: &FoliageStyle, atlas: &FoliageAtlas, rng: &mut Rng) -> Foliage {
    let sp: &Species = st.sp;
    let lf = &sp.leaves;
    let winds = stem_wind(sk);
    let leafy: Vec<usize> = (0..sk.stems.len()).filter(|i| sk.stems[*i].leafy && !sk.stems[*i].dead).collect();
    let lens: Vec<f32> = leafy.iter().map(|i| sk.stems[*i].length() * (1.0 - lf.start)).collect();
    let total: f32 = lens.iter().sum::<f32>().max(0.1);
    let mut n = total * lf.density * (0.45 + 0.55 * st.health.clamp(0.0, 1.0));
    if st.leafless() {
        n *= 0.4;
    }
    let lo = if st.leafless() { lf.min_cards as f32 * 0.4 } else { lf.min_cards as f32 };
    let n = n.clamp(lo, lf.max_cards as f32 * if st.leafless() { 0.5 } else { 1.0 });
    // First pass: origins and stem directions.
    let mut spots: Vec<(Vec3, Vec3, f32)> = Vec::new();
    let mut carry = 0.0f32;
    for (k, &si) in leafy.iter().enumerate() {
        let s = &sk.stems[si];
        let len = s.length();
        let want = n * lens[k] / total + carry;
        let m = want.floor() as usize;
        carry = want - m as f32;
        for j in 0..m {
            let t = lf.start + (1.0 - lf.start) * if m == 1 { 1.0 } else { (j as f32 + 0.5 + rng.signed() * 0.3) / m as f32 };
            let (p, d, _) = s.sample(len * t.min(1.0));
            spots.push((p, d, winds[si](t)));
        }
    }
    if spots.is_empty() {
        return Foliage::default();
    }
    let (mut lo3, mut hi3) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (p, _, _) in &spots {
        lo3 = lo3.min(*p);
        hi3 = hi3.max(*p);
    }
    let center = (lo3 + hi3) * 0.5;
    let radii = ((hi3 - lo3) * 0.5).max(Vec3::splat(0.3));
    let pick = |k: SlotKind| atlas.of(k);
    let leaves = pick(if st.leafless() { SlotKind::Bare } else { SlotKind::Leaves });
    let inner = pick(SlotKind::Inner);
    let bloom = pick(SlotKind::Bloom);
    let inner_bloom = pick(SlotKind::InnerBloom);
    let bloom_share = sp.bloom.as_ref().map(|b| if b.before_leaves { 0.85 } else { b.coverage }).unwrap_or(0.0);
    let mut cards = Vec::with_capacity(spots.len());
    let mut fruits = Vec::new();
    let fruit = sp.fruit.clone();
    for (p, d, w) in spots {
        let q = (p - center) / radii;
        let depth = q.length();
        let cn = (q / radii.max(Vec3::splat(0.1)) * radii.length() + Vec3::Y * 0.2).normalize_or(Vec3::Y);
        let noise = rng.unit() * 0.45;
        let a = (d * (1.0 - lf.outward) + cn * lf.outward + Vec3::Y * (0.3 - lf.droop) + noise).normalize_or(Vec3::Y);
        let n0 = cn - a * a.dot(cn);
        let n0 = if n0.length_squared() < 1e-4 { crate::skeleton::perp(a) } else { n0.normalize() };
        let normal = rotate(n0, a, rng.signed() * 0.9);
        let size = lf.card_size * (1.0 + lf.card_var * rng.signed());
        let in_bloom = st.blooming() && !bloom.is_empty() && rng.chance(bloom_share);
        let deep = depth < 0.62 && rng.chance(0.75);
        let list: &Vec<Rect> = match (in_bloom, deep) {
            (true, true) if !inner_bloom.is_empty() => &inner_bloom,
            (true, _) => &bloom,
            (false, true) if !inner.is_empty() && !st.leafless() => &inner,
            _ => &leaves,
        };
        let list = if list.is_empty() { &leaves } else { list };
        if list.is_empty() {
            break;
        }
        let slot = rng.below(list.len());
        cards.push(Card {
            origin: p - a * size * 0.08,
            axis: a,
            normal,
            length: size,
            width: size,
            rect: list[slot],
            mirror: rng.chance(0.5),
            wind: (w + 0.25).min(1.0),
            crown_n: cn,
            fold: lf.fold + rng.signed() * 10.0,
        });
        if let Some(f) = &fruit {
            let winter_ok = f.kind == FruitKind::Seedball || !st.leafless();
            if winter_ok && f.kind != FruitKind::Seedball && rng.chance(f.per_card * (0.5 + depth.min(1.2))) {
                let r = f.radius * rng.range(0.85, 1.15);
                let c = p + a * size * rng.range(0.15, 0.55) + normal * rng.signed() * size * 0.15 - Vec3::Y * r;
                let radii = match f.kind {
                    FruitKind::Cone => Vec3::new(r * 0.8, r * 1.1, r * 0.8),
                    _ => Vec3::new(r, r * 0.95, r),
                };
                fruits.push(Fruit { center: c, radii, wind: w });
            }
            if f.kind == FruitKind::Seedball && !st.leafless() && rng.chance(f.per_card) {
                let r = f.radius;
                fruits.push(Fruit { center: p - Vec3::Y * (0.12 + r), radii: Vec3::splat(r), wind: (w + 0.3).min(1.0) });
            }
        }
    }
    Foliage { cards, fruits, crown_center: center, crown_radii: radii }
}

