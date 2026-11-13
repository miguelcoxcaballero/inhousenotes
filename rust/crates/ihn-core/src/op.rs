//! Operations, batches and the property types of pages and items (ARCHITECTURE §4.1–4.2). Codecs: `snapshot.rs`.

use crate::id::{ActorId, Hash, Id};
use crate::pos::Pos;
use crate::stamp::Stamp;

/// Smallest page side (ARCHITECTURE §4.1: page size = integers ≥ 64).
pub const MIN_PAGE_SIDE: u32 = 64;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Template {
    #[default]
    Default,
    Agenda,
    Diary,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Background {
    /// The page shows its template.
    #[default]
    Template,
    /// Page `page` (1-based) of the source PDF `asset`.
    Pdf { asset: Hash, page: u32 },
    /// A photo page.
    Custom { asset: Hash },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PageProp {
    Template(Template),
    Size((u32, u32)),
    Background(Background),
    /// Kept opaque, never shown (REQUIREMENTS §1b).
    SidePanel(Option<Vec<u8>>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Highlighter,
    /// 5.x `eraser-area` stroke (rendered with the legacy rule; never hit-tested or selected).
    LegacyEraseArea,
    /// 5.x legacy cover stroke: drawn below images.
    LegacyCover,
}

/// `#rrggbb` as a number, or the exact 5.x string when it has another form.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Color {
    Hex(u32),
    Css(Box<str>),
}

impl Color {
    /// `#rrggbb` (either case) becomes `Hex`; anything else is kept verbatim.
    pub fn parse(s: &str) -> Color {
        match s.strip_prefix('#').filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())) {
            Some(hex) => Color::Hex(u32::from_str_radix(hex, 16).unwrap_or(0)),
            None => Color::Css(s.into()),
        }
    }
    /// The canvas/CSS string: lowercase `#rrggbb`, or the stored string.
    pub fn css(&self) -> String {
        match self {
            Color::Hex(v) => format!("#{:06x}", v & 0xff_ffff),
            Color::Css(s) => s.to_string(),
        }
    }
}

/// Draw order within a kind: `(Z, item Id)`. A new item gets `Z{at: its op stamp, sub: 0}`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Z {
    pub at: Stamp,
    pub sub: u32,
}

impl Z {
    /// Placeholder used inside a `Tx`: `commit` replaces it with the stamp of its op ("on top").
    pub const TOP: Z = Z { at: Stamp { clock: u64::MAX, actor: ActorId(u64::MAX), idx: u32::MAX }, sub: u32::MAX };
    /// Z of fragment `k` of an area-erased item: the original position (BEHAVIOUR-CHANGES B-E4).
    /// `sub` only orders fragments of one original among themselves, so wrapping is harmless.
    pub fn fragment(self, k: u32) -> Z { Z { at: self.at, sub: self.sub.wrapping_mul(1 << 16).wrapping_add(k) } }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Body {
    Stroke {
        tool: Tool,
        color: Color,
        width: f64,
        /// `x0, y0, x1, y1, …` — lossless f64, never simplified.
        xy: Vec<f64>,
        /// Pressure per point: stored, never rendered.
        p: Option<Vec<f32>>,
    },
    Image {
        asset: Hash,
        cx: f64,
        cy: f64,
        w: f64,
        h: f64,
        rotation: f64,
    },
    /// The body of a deleted item that a snapshot dropped (ARCHITECTURE §4.2), or of an item known only by an
    /// `ItemDel`. Never travels in an op; an undo of the deletion carries the real body.
    Dropped,
}

impl Body {
    pub fn is_dropped(&self) -> bool { matches!(self, Body::Dropped) }
    pub fn points(&self) -> &[f64] {
        match self {
            Body::Stroke { xy, .. } => xy,
            _ => &[],
        }
    }
    /// Number of stroke points (undo memory accounting: 25,000 points).
    pub fn point_count(&self) -> usize { self.points().len() / 2 }
}

/// Centre, size and rotation of an image on a page.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ImageRect {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
}

/// A timeline entry. `data` is the `VersionMeta` record of `ihn-core::history` (ts, author, kind, summary, device,
/// snapshot asset hash); the model only needs the id and keeps the rest opaque.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VersionMeta {
    pub id: Id,
    pub data: Vec<u8>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Op {
    PageNew { page: Id, pos: Pos, template: Template, size: (u32, u32), background: Background },
    PageMove { page: Id, pos: Pos },
    /// `deleted: false` = restore (undo / timeline restore).
    PageDel { page: Id, deleted: bool },
    PageSet { page: Id, prop: PageProp },
    /// Create, or write page + z + body (transform, recolour, move).
    ItemPut { item: Id, page: Id, z: Z, body: Body },
    /// Erase / undo of erase. Writers put `ItemPut` before `ItemDel{deleted: false}` in the same batch, so that a
    /// replica that dropped the body revives it.
    ItemDel { item: Id, deleted: bool },
    DocName { name: String },
    VersionAdd { meta: VersionMeta },
    VersionDel { version: Id },
}

/// One user action = one batch = one undo step = one sync unit; applied atomically.
#[derive(Clone, PartialEq, Debug)]
pub struct Batch {
    pub actor: ActorId,
    /// 1, 2, 3… contiguous per (document, actor).
    pub seq: u32,
    pub clock: u64,
    pub ops: Vec<Op>,
    /// `Some` iff `actor` is an import actor: one explicit stamp per op.
    pub stamps: Option<Vec<Stamp>>,
}

impl Batch {
    /// Stamp of op `i`: `{clock, actor, i}` for a native batch, the explicit stamp for an import batch.
    pub fn stamp(&self, i: usize) -> Stamp {
        match &self.stamps {
            Some(stamps) => stamps.get(i).copied().unwrap_or(Stamp::ZERO),
            None => Stamp { clock: self.clock, actor: self.actor, idx: i as u32 },
        }
    }
    /// Greatest clock the batch carries: what the receiver's HLC observes (`Clock::observe`).
    pub fn max_clock(&self) -> u64 {
        self.stamps.iter().flatten().map(|s| s.clock).fold(self.clock, u64::max)
    }
    /// Structural invariant checked by the decoder: import batches carry one stamp per op, native ones none.
    pub fn is_well_formed(&self) -> bool {
        self.seq != 0
            && match &self.stamps {
                Some(stamps) => self.actor.is_import() && stamps.len() == self.ops.len(),
                None => !self.actor.is_import(),
            }
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.op.<name>', args)`.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    match name {
        "color" => Some(format!("\"{}\"", Color::parse(args).css().replace(['"', '\\'], ""))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_keep_non_hex_strings_verbatim() {
        assert_eq!(Color::parse("#002FD9"), Color::Hex(0x002fd9));
        assert_eq!(Color::parse("#002fd9").css(), "#002fd9");
        for s in ["red", "#fff", "rgba(0,47,217,0.45)", "#12345g", "", "#1234567"] {
            assert_eq!(Color::parse(s), Color::Css(s.into()));
            assert_eq!(Color::parse(s).css(), s);
        }
    }

    #[test]
    fn stamps_and_z_rules() {
        let a = ActorId(0x500);
        let native = Batch { actor: a, seq: 1, clock: 77, ops: vec![Op::DocName { name: "n".into() }; 3], stamps: None };
        assert_eq!(native.stamp(2), Stamp::new(77, a, 2));
        assert!(native.is_well_formed() && native.max_clock() == 77);
        let imp = ActorId::import("rev");
        let stamps = vec![Stamp::new(5, ActorId(9), 0), Stamp::new(900, imp, 1)];
        let import = Batch { actor: imp, seq: 1, clock: 10, ops: native.ops[..2].to_vec(), stamps: Some(stamps.clone()) };
        assert_eq!((import.stamp(0), import.stamp(1), import.max_clock()), (stamps[0], stamps[1], 900));
        assert!(import.is_well_formed());
        assert!(!Batch { stamps: None, ..import.clone() }.is_well_formed());
        assert!(!Batch { stamps: Some(stamps), ..native.clone() }.is_well_formed());
        assert!(!Batch { seq: 0, ..native }.is_well_formed());
        // Fragments stay at the original's position: same `at`, ordered by k; any real stamp is below TOP.
        let z = Z { at: Stamp::new(40, a, 3), sub: 0 };
        assert!(z.fragment(0) == z && z.fragment(1) > z && z.fragment(1).fragment(2).at == z.at);
        assert!(Z { at: Stamp::new(41, a, 0), sub: 0 } > z.fragment(65_535));
        let _ = Z { at: z.at, sub: u32::MAX }.fragment(9); // wraps, never panics
        assert!(Z::TOP > Z { at: Stamp::new(u64::MAX - 1, a, 0), sub: 0 });
    }
}
