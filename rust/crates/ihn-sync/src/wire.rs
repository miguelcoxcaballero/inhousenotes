//! Frame codec and Part fragmentation/reassembly (SYNC §10.5). Owner: W2.6.
//!
//! One frame = one data-channel or BroadcastChannel message. Decoders never panic: truncation, trailing bytes, an
//! unknown type or a limit violation rejects the whole frame (`None`), and the caller closes the channel.

use ihn_core::codec::{Reader, Writer};
use ihn_core::doc::{PresenceMode, PresenceState};
use ihn_core::id::{Hash, Id};
use ihn_core::op::{Body, Color, Tool};
use ihn_core::snapshot::{
    get_body, get_bytes, get_color, get_hash, get_id, get_str, put_body, put_bytes, put_color, put_hash, put_id, put_str,
    EncodedBatch, EncodedSnapshot,
};
use ihn_core::stamp::VersionVector;

pub const PROTO: u16 = 1;
/// Largest channel message.
pub const MAX_MESSAGE: usize = 64 << 10;
/// Frames above this travel as `Part`s on the bulk channel; also the payload of one `Part`.
pub const PART_DATA: usize = 60 << 10;
pub const MAX_PARTS: usize = 8192;
/// Largest reassembled frame, and the reservation limit per peer.
pub const MAX_ASSEMBLED: usize = 64 << 20;
pub const MAX_ASSEMBLIES: usize = 4;
pub const ASSEMBLY_TTL_MS: f64 = 60_000.0;
/// Live ink: points per stroke (5.x value).
pub const MAX_LIVE_POINTS: u64 = 24_000;

/// SYNC §8.3 `Profile` (relay header and `Hello`).
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Profile {
    pub email: String,
    pub name: String,
    pub photo_url: String,
    /// `b'm'` | `b't'` | `b'p'`.
    pub device_type: u8,
    pub label: String,
    pub save_phase: u8,
}

const PROFILE_LIMITS: [usize; 4] = [38, 22, 512, 16];

/// Longest prefix of `s` of at most `max` bytes that ends on a character boundary.
fn clip(s: &str, max: usize) -> &str {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

impl Profile {
    /// Over-long fields are clipped (byte limits), an unknown device type becomes `p`.
    pub fn encode(&self, w: &mut Writer) {
        put_str(w, clip(&self.email, PROFILE_LIMITS[0]));
        put_str(w, clip(&self.name, PROFILE_LIMITS[1]));
        put_str(w, clip(&self.photo_url, PROFILE_LIMITS[2]));
        w.u8(if matches!(self.device_type, b'm' | b't') { self.device_type } else { b'p' });
        put_str(w, clip(&self.label, PROFILE_LIMITS[3]));
        w.u8(self.save_phase);
    }
    pub fn decode(r: &mut Reader) -> Option<Profile> {
        let text = |r: &mut Reader, i: usize| get_str(r).filter(|s| s.len() <= PROFILE_LIMITS[i]).map(str::to_string);
        let (email, name, photo_url) = (text(r, 0)?, text(r, 1)?, text(r, 2)?);
        let device_type = r.u8().filter(|d| matches!(d, b'm' | b't' | b'p'))?;
        Some(Profile { email, name, photo_url, device_type, label: text(r, 3)?, save_phase: r.u8()? })
    }
}

/// SYNC §10.2 signalling mail entry (inside the sealed relay file).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mail {
    pub to: u64,
    pub sid: u64,
    /// 1 offer, 2 answer.
    pub kind: u8,
    pub ttl_ms: u32,
    pub sdp: String,
}

pub const MAIL_TTL_MS: u32 = 60_000;

impl Mail {
    pub fn encode(&self, w: &mut Writer) {
        w.u64(self.to);
        w.u64(self.sid);
        w.u8(self.kind);
        w.leb(self.ttl_ms.min(MAIL_TTL_MS) as u64);
        put_str(w, &self.sdp);
    }
    pub fn decode(r: &mut Reader) -> Option<Mail> {
        let (to, sid) = (r.u64()?, r.u64()?);
        let kind = r.u8().filter(|k| matches!(k, 1 | 2))?;
        let ttl_ms = u32v(r).filter(|t| *t <= MAIL_TTL_MS)?;
        Some(Mail { to, sid, kind, ttl_ms, sdp: get_str(r)?.to_string() })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Frame {
    Hello { doc_id: String, inst: u64, vv: VersionVector, profile: Profile, presence: PresenceState },
    /// Ack / anti-entropy.
    Have(VersionVector),
    Batches(Vec<EncodedBatch>),
    Part { msg_id: u32, index: u16, count: u16, data: Vec<u8> },
    AssetWant(Hash),
    AssetPart { hash: Hash, offset: u64, total: u64, data: Vec<u8> },
    SnapshotWant,
    Snapshot(EncodedSnapshot),
    InkBegin { live: u64, page: Id, tool: Tool, color: Color, width: f64, has_p: bool },
    InkPts { live: u64, from: u32, xy: Vec<f64>, p: Option<Vec<f32>> },
    InkEnd { live: u64, item: Id },
    InkCancel { live: u64 },
    Erase { gesture: u64, page: Id, hidden: Vec<Id>, fragments: Vec<(u64, Body)> },
    EraseEnd { gesture: u64 },
    Presence(PresenceState),
    ExportDone { head: String, vv: VersionVector },
    Ping(u32),
    Pong(u32),
    Bye(u8),
}

fn u32v(r: &mut Reader) -> Option<u32> { u32::try_from(r.leb()?).ok() }
fn flag(r: &mut Reader) -> Option<bool> { r.u8().filter(|&b| b <= 1).map(|b| b == 1) }
/// A count whose items take at least `min` bytes each: bounded by what is left, so it is safe to allocate.
fn count(r: &mut Reader, min: usize) -> Option<usize> { usize::try_from(r.leb()?).ok().filter(|&n| n <= r.buf.len() / min) }

const TOOLS: [Tool; 4] = [Tool::Pen, Tool::Highlighter, Tool::LegacyEraseArea, Tool::LegacyCover];

fn put_presence(w: &mut Writer, p: &PresenceState) {
    w.u8(p.mode as u8);
    put_id(w, p.page);
    put_color(w, &p.tool_colour);
    w.u8(p.save_phase);
}
fn get_presence(r: &mut Reader) -> Option<PresenceState> {
    let mode = match r.u8()? {
        0 => PresenceMode::Editing,
        1 => PresenceMode::Viewing,
        2 => PresenceMode::Reading,
        _ => return None,
    };
    Some(PresenceState { mode, page: get_id(r)?, tool_colour: get_color(r)?, save_phase: r.u8()? })
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        self.write(&mut w);
        w.buf
    }

    fn write(&self, w: &mut Writer) {
        match self {
            Frame::Hello { doc_id, inst, vv, profile, presence } => {
                w.u8(0x01);
                w.raw(&PROTO.to_le_bytes());
                put_str(w, doc_id);
                w.u64(*inst);
                w.u8(1);
                vv.encode(w);
                profile.encode(w);
                put_presence(w, presence);
            }
            Frame::Have(vv) => {
                w.u8(0x02);
                vv.encode(w);
            }
            Frame::Batches(batches) => {
                w.u8(0x03);
                w.leb(batches.len() as u64);
                batches.iter().for_each(|b| put_bytes(w, &b.0));
            }
            Frame::Part { msg_id, index, count, data } => {
                w.u8(0x04);
                w.u32(*msg_id);
                w.raw(&index.to_le_bytes());
                w.raw(&count.to_le_bytes());
                put_bytes(w, data);
            }
            Frame::AssetWant(hash) => {
                w.u8(0x05);
                put_hash(w, hash);
            }
            Frame::AssetPart { hash, offset, total, data } => {
                w.u8(0x06);
                put_hash(w, hash);
                w.leb(*offset);
                w.leb(*total);
                put_bytes(w, data);
            }
            Frame::SnapshotWant => w.u8(0x07),
            Frame::Snapshot(s) => {
                w.u8(0x08);
                put_bytes(w, &s.0);
            }
            Frame::InkBegin { live, page, tool, color, width, has_p } => {
                w.u8(0x10);
                w.u64(*live);
                put_id(w, *page);
                w.u8(*tool as u8);
                put_color(w, color);
                w.f64(*width);
                w.u8(*has_p as u8);
            }
            Frame::InkPts { live, from, xy, p } => {
                w.u8(0x11);
                w.u64(*live);
                w.leb(*from as u64);
                w.leb(xy.len() as u64 / 2);
                xy.iter().for_each(|v| w.f64(*v));
                p.iter().flatten().for_each(|v| w.f32(*v));
            }
            Frame::InkEnd { live, item } => {
                w.u8(0x12);
                w.u64(*live);
                put_id(w, *item);
            }
            Frame::InkCancel { live } => {
                w.u8(0x13);
                w.u64(*live);
            }
            Frame::Erase { gesture, page, hidden, fragments } => {
                w.u8(0x14);
                w.u64(*gesture);
                put_id(w, *page);
                w.leb(hidden.len() as u64);
                hidden.iter().for_each(|id| put_id(w, *id));
                w.leb(fragments.len() as u64);
                for (tmp, body) in fragments {
                    w.u64(*tmp);
                    put_body(w, body);
                }
            }
            Frame::EraseEnd { gesture } => {
                w.u8(0x15);
                w.u64(*gesture);
            }
            Frame::Presence(p) => {
                w.u8(0x18);
                put_presence(w, p);
            }
            Frame::ExportDone { head, vv } => {
                w.u8(0x19);
                put_str(w, head);
                vv.encode(w);
            }
            Frame::Ping(t) | Frame::Pong(t) => {
                w.u8(if matches!(self, Frame::Ping(_)) { 0x20 } else { 0x21 });
                w.u32(*t);
            }
            Frame::Bye(reason) => {
                w.u8(0x22);
                w.u8(*reason);
            }
        }
    }

    /// `None` for anything that is not exactly one well-formed frame.
    pub fn decode(bytes: &[u8]) -> Option<Frame> {
        let mut r = Reader::new(bytes);
        let f = Self::read(&mut r)?;
        r.is_empty().then_some(f)
    }

    fn read(r: &mut Reader) -> Option<Frame> {
        let u16le = |r: &mut Reader| Some(u16::from_le_bytes(r.take(2)?.try_into().ok()?));
        Some(match r.u8()? {
            0x01 => {
                if u16le(r)? != PROTO {
                    return None;
                }
                let doc_id = get_str(r)?.to_string();
                let inst = r.u64()?;
                if r.u8()? != 1 {
                    return None; // only editors are peers
                }
                Frame::Hello { doc_id, inst, vv: VersionVector::decode(r)?, profile: Profile::decode(r)?, presence: get_presence(r)? }
            }
            0x02 => Frame::Have(VersionVector::decode(r)?),
            0x03 => {
                let n = count(r, 1)?;
                let mut batches = Vec::with_capacity(n);
                for _ in 0..n {
                    let b = EncodedBatch(get_bytes(r)?.to_vec());
                    b.header()?;
                    batches.push(b);
                }
                Frame::Batches(batches)
            }
            0x04 => {
                let (msg_id, index, count) = (r.u32()?, u16le(r)?, u16le(r)?);
                let data = get_bytes(r)?;
                if count == 0 || count as usize > MAX_PARTS || index >= count || data.len() > PART_DATA {
                    return None;
                }
                Frame::Part { msg_id, index, count, data: data.to_vec() }
            }
            0x05 => Frame::AssetWant(get_hash(r)?),
            0x06 => {
                let (hash, offset, total) = (get_hash(r)?, r.leb()?, r.leb()?);
                let data = get_bytes(r)?;
                if offset.checked_add(data.len() as u64)? > total {
                    return None;
                }
                Frame::AssetPart { hash, offset, total, data: data.to_vec() }
            }
            0x07 => Frame::SnapshotWant,
            0x08 => Frame::Snapshot(EncodedSnapshot(get_bytes(r)?.to_vec())),
            0x10 => Frame::InkBegin {
                live: r.u64()?,
                page: get_id(r)?,
                tool: *TOOLS.get(r.u8()? as usize)?,
                color: get_color(r)?,
                width: r.f64()?,
                has_p: flag(r)?,
            },
            0x11 => {
                let (live, from) = (r.u64()?, u32v(r)?);
                let n = count(r, 16)?;
                if from as u64 + n as u64 > MAX_LIVE_POINTS {
                    return None;
                }
                let mut xy = Vec::with_capacity(n * 2);
                for _ in 0..n * 2 {
                    xy.push(r.f64()?);
                }
                // Pressure is present iff the stroke's InkBegin said so: the rest of the frame is n × f32 or nothing.
                let p = if n > 0 && r.buf.len() == n * 4 {
                    let mut p = Vec::with_capacity(n);
                    for _ in 0..n {
                        p.push(r.f32()?);
                    }
                    Some(p)
                } else {
                    None
                };
                Frame::InkPts { live, from, xy, p }
            }
            0x12 => Frame::InkEnd { live: r.u64()?, item: get_id(r)? },
            0x13 => Frame::InkCancel { live: r.u64()? },
            0x14 => {
                let (gesture, page) = (r.u64()?, get_id(r)?);
                let n = count(r, 16)?;
                let mut hidden = Vec::with_capacity(n);
                for _ in 0..n {
                    hidden.push(get_id(r)?);
                }
                let m = count(r, 9)?;
                let mut fragments = Vec::with_capacity(m);
                for _ in 0..m {
                    let tmp = r.u64()?;
                    fragments.push((tmp, get_body(r).filter(|b| !b.is_dropped())?));
                }
                Frame::Erase { gesture, page, hidden, fragments }
            }
            0x15 => Frame::EraseEnd { gesture: r.u64()? },
            0x18 => Frame::Presence(get_presence(r)?),
            0x19 => Frame::ExportDone { head: get_str(r)?.to_string(), vv: VersionVector::decode(r)? },
            0x20 => Frame::Ping(r.u32()?),
            0x21 => Frame::Pong(r.u32()?),
            0x22 => Frame::Bye(r.u8()?),
            _ => return None,
        })
    }
}

/// Splits an encoded frame into encoded `Part` frames for the bulk channel. `None` above 64 MiB.
pub fn fragment(msg_id: u32, frame: &[u8]) -> Option<Vec<Vec<u8>>> {
    if frame.len() > MAX_ASSEMBLED {
        return None;
    }
    let count = frame.len().div_ceil(PART_DATA).max(1) as u16;
    let mut chunks = frame.chunks(PART_DATA);
    Some(
        (0..count)
            .map(|index| Frame::Part { msg_id, index, count, data: chunks.next().unwrap_or(&[]).to_vec() }.encode())
            .collect(),
    )
}

struct Assembly {
    msg_id: u32,
    parts: Vec<Option<Vec<u8>>>,
    missing: usize,
    started: f64,
}

/// Outcome of one `Part`.
#[derive(Debug, PartialEq)]
pub enum Assembled {
    Pending,
    /// The complete inner frame bytes (decode with [`Frame::decode`]).
    Done(Vec<u8>),
    /// A limit was violated: close the channel.
    Close,
}

/// `Part` reassembly of one peer: ≤ 4 assemblies, ≤ 64 MiB reserved, 60 s TTL.
#[derive(Default)]
pub struct Reassembly {
    open: Vec<Assembly>,
}

impl Reassembly {
    fn reserved(&self) -> usize { self.open.iter().map(|a| a.parts.len() * PART_DATA).sum() }

    pub fn expire(&mut self, now: f64) { self.open.retain(|a| now - a.started < ASSEMBLY_TTL_MS) }

    /// Fields as decoded by [`Frame::decode`] (which already bounds `index`, `count` and the part size).
    pub fn push(&mut self, now: f64, msg_id: u32, index: u16, count: u16, data: Vec<u8>) -> Assembled {
        self.expire(now);
        let (index, count) = (index as usize, count as usize);
        if count == 0 || count > MAX_PARTS || index >= count || data.len() > PART_DATA {
            return Assembled::Close;
        }
        let at = match self.open.iter().position(|a| a.msg_id == msg_id) {
            Some(at) => at,
            None => {
                if self.open.len() >= MAX_ASSEMBLIES || self.reserved() + count * PART_DATA > MAX_ASSEMBLED + PART_DATA {
                    return Assembled::Close;
                }
                self.open.push(Assembly { msg_id, parts: vec![None; count], missing: count, started: now });
                self.open.len() - 1
            }
        };
        let a = &mut self.open[at];
        match a.parts.get(index) {
            // A different count, or a duplicate with different bytes, drops the assembly.
            None => drop(self.open.swap_remove(at)),
            Some(Some(old)) if *old != data => drop(self.open.swap_remove(at)),
            Some(Some(_)) => {}
            Some(None) => {
                // Only the last part may be short, so the size is known as soon as it is complete.
                a.parts[index] = Some(data);
                a.missing -= 1;
                if a.missing == 0 {
                    let done = self.open.swap_remove(at);
                    return Assembled::Done(done.parts.into_iter().flatten().flatten().collect());
                }
            }
        }
        Assembled::Pending
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('sync.wire.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ihn_core::id::ActorId;
    use ihn_core::op::{Batch, Op};
    use ihn_core::snapshot::encode_batch;
    use ihn_core::testutil::Rng;

    fn text(rng: &mut Rng, max: u64) -> String {
        (0..rng.below(max + 1)).map(|_| *rng.pick(&['a', 'Z', '9', '@', '.', ' ', 'ñ', '語', '🖊'])).collect()
    }
    fn id(rng: &mut Rng) -> Id { Id((rng.next() as u128) << 64 | rng.next() as u128) }
    fn hash(rng: &mut Rng) -> Hash { std::array::from_fn(|_| rng.next() as u8) }
    fn float(rng: &mut Rng) -> f64 {
        match rng.below(8) {
            0 => f64::from_bits(rng.next()), // any bit pattern, NaNs included
            1 => -0.0,
            _ => rng.range(-5000.0, 5000.0),
        }
    }
    fn color(rng: &mut Rng) -> Color {
        if rng.chance(0.7) {
            Color::Hex(rng.below(1 << 24) as u32)
        } else {
            Color::Css(text(rng, 12).into())
        }
    }
    fn vv(rng: &mut Rng) -> VersionVector {
        let mut vv = VersionVector::default();
        for _ in 0..rng.below(5) {
            vv.raise(ActorId(rng.next()), 1 + rng.below(1000) as u32);
        }
        vv
    }
    pub fn profile(rng: &mut Rng) -> Profile {
        Profile {
            email: clip(&text(rng, 30), 38).into(),
            name: clip(&text(rng, 20), 22).into(),
            photo_url: clip(&text(rng, 200), 512).into(),
            device_type: *rng.pick(b"mtp"),
            label: clip(&text(rng, 10), 16).into(),
            save_phase: rng.below(8) as u8,
        }
    }
    fn presence(rng: &mut Rng) -> PresenceState {
        let mode = *rng.pick(&[PresenceMode::Editing, PresenceMode::Viewing, PresenceMode::Reading]);
        PresenceState { mode, page: id(rng), tool_colour: color(rng), save_phase: rng.below(8) as u8 }
    }
    fn stroke(rng: &mut Rng) -> Body {
        let n = rng.below(40) as usize;
        Body::Stroke {
            tool: *rng.pick(&TOOLS),
            color: color(rng),
            width: float(rng),
            xy: (0..n * 2).map(|_| float(rng)).collect(),
            p: rng.chance(0.5).then(|| (0..n).map(|_| rng.unit() as f32).collect()),
        }
    }
    pub fn batch(rng: &mut Rng) -> EncodedBatch {
        let ops = (0..rng.below(4)).map(|_| Op::DocName { name: text(rng, 20) }).collect();
        encode_batch(&Batch { actor: ActorId(rng.next() >> 1), seq: 1 + rng.below(99) as u32, clock: rng.next() >> 20, ops, stamps: None })
    }
    fn bytes(rng: &mut Rng, max: u64) -> Vec<u8> { (0..rng.below(max + 1)).map(|_| rng.next() as u8).collect() }

    pub fn frame(rng: &mut Rng) -> Frame {
        match rng.below(19) {
            0 => Frame::Hello { doc_id: text(rng, 44), inst: rng.next(), vv: vv(rng), profile: profile(rng), presence: presence(rng) },
            1 => Frame::Have(vv(rng)),
            2 => Frame::Batches((0..rng.below(5)).map(|_| batch(rng)).collect()),
            3 => {
                let count = 1 + rng.below(MAX_PARTS as u64) as u16;
                Frame::Part { msg_id: rng.next() as u32, index: rng.below(count as u64) as u16, count, data: bytes(rng, 300) }
            }
            4 => Frame::AssetWant(hash(rng)),
            5 => {
                let data = bytes(rng, 300);
                let offset = rng.below(1 << 30);
                Frame::AssetPart { hash: hash(rng), offset, total: offset + data.len() as u64 + rng.below(9), data }
            }
            6 => Frame::SnapshotWant,
            7 => Frame::Snapshot(EncodedSnapshot(bytes(rng, 500))),
            8 => Frame::InkBegin { live: rng.next(), page: id(rng), tool: *rng.pick(&TOOLS), color: color(rng), width: float(rng), has_p: rng.chance(0.5) },
            9 => {
                let n = rng.below(30) as usize;
                let p = (n > 0 && rng.chance(0.5)).then(|| (0..n).map(|_| f32::from_bits(rng.next() as u32)).collect());
                Frame::InkPts { live: rng.next(), from: rng.below(20_000) as u32, xy: (0..n * 2).map(|_| float(rng)).collect(), p }
            }
            10 => Frame::InkEnd { live: rng.next(), item: id(rng) },
            11 => Frame::InkCancel { live: rng.next() },
            12 => Frame::Erase {
                gesture: rng.next(),
                page: id(rng),
                hidden: (0..rng.below(6)).map(|_| id(rng)).collect(),
                fragments: (0..rng.below(4)).map(|_| (rng.next(), stroke(rng))).collect(),
            },
            13 => Frame::EraseEnd { gesture: rng.next() },
            14 => Frame::Presence(presence(rng)),
            15 => Frame::ExportDone { head: text(rng, 60), vv: vv(rng) },
            16 => Frame::Ping(rng.next() as u32),
            17 => Frame::Pong(rng.next() as u32),
            _ => Frame::Bye(rng.next() as u8),
        }
    }

    /// Bit-exact comparison (NaN payloads included): equal frames encode to equal bytes.
    fn same(a: &Frame, b: &Frame) -> bool { a.encode() == b.encode() }

    #[test]
    fn frame_bytes_are_the_sync_10_5_layout() {
        assert_eq!(Frame::Ping(0x0403_0201).encode(), [0x20, 1, 2, 3, 4]);
        assert_eq!(Frame::Bye(7).encode(), [0x22, 7]);
        assert_eq!(Frame::SnapshotWant.encode(), [0x07]);
        let mut vv = VersionVector::default();
        vv.raise(ActorId(0x0100), 300);
        assert_eq!(Frame::Have(vv.clone()).encode(), [0x02, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0xac, 0x02]);
        assert_eq!(Frame::Part { msg_id: 1, index: 2, count: 3, data: vec![9, 8] }.encode(), [0x04, 1, 0, 0, 0, 2, 0, 3, 0, 2, 9, 8]);
        let ink = Frame::InkPts { live: 5, from: 2, xy: vec![1.0, 2.0], p: Some(vec![0.5]) }.encode();
        assert_eq!(ink.len(), 1 + 8 + 1 + 1 + 16 + 4);
        assert_eq!(&ink[..11], [0x11, 5, 0, 0, 0, 0, 0, 0, 0, 2, 1]);
        let presence = PresenceState { mode: PresenceMode::Reading, page: Id(1), tool_colour: Color::Hex(0x112233), save_phase: 4 };
        let hello = Frame::Hello { doc_id: "d".into(), inst: 0x200, vv, profile: Profile { device_type: b't', ..Default::default() }, presence }.encode();
        assert_eq!(&hello[..14], [0x01, 1, 0, 1, b'd', 0, 2, 0, 0, 0, 0, 0, 0, 1]);
        // … VV (11 bytes), Profile: three empty strings, 't', empty label, phase; Presence: mode, page, colour, phase.
        assert_eq!(&hello[25..31], [0, 0, 0, b't', 0, 0]);
        assert_eq!(hello[31], 2);
        assert_eq!(hello.len(), 31 + 1 + 16 + 5 + 1);
    }

    #[test]
    fn random_frames_round_trip_bit_exactly() {
        let mut rng = Rng::new(61);
        for _ in 0..4000 {
            let f = frame(&mut rng);
            let bytes = f.encode();
            let back = Frame::decode(&bytes).unwrap_or_else(|| panic!("rejected {f:?}"));
            assert!(same(&f, &back), "{f:?}\n{back:?}");
        }
    }

    #[test]
    fn damaged_frames_are_rejected_whole_and_never_panic() {
        let mut rng = Rng::new(62);
        for round in 0..3000 {
            let good = frame(&mut rng).encode();
            // Every strict prefix is rejected, and so is trailing garbage.
            let cut = rng.below(good.len() as u64) as usize;
            assert!(Frame::decode(&good[..cut]).is_none(), "prefix {cut} of {good:?}");
            let mut long = good.clone();
            long.push(rng.next() as u8);
            assert!(Frame::decode(&long).is_none());
            // Bit flips and random bytes: any outcome but a panic; what is accepted re-encodes to itself.
            let mut bad = good.clone();
            for _ in 0..1 + rng.below(3) {
                let at = rng.below(bad.len() as u64) as usize;
                bad[at] ^= 1 << rng.below(8);
            }
            let noise = bytes(&mut rng, 80);
            for input in [&bad, &noise] {
                if let Some(f) = Frame::decode(input) {
                    let again = Frame::decode(&f.encode()).expect("accepted frame re-encodes");
                    assert!(same(&f, &again), "round {round}");
                }
            }
        }
        // Hostile counts never allocate beyond the input.
        for ty in [0x03u8, 0x11, 0x14] {
            let mut huge = vec![ty];
            huge.extend_from_slice(&[0xff; 9]);
            huge.push(0x01);
            assert!(Frame::decode(&huge).is_none());
        }
    }

    #[test]
    fn mail_and_profile_round_trip_and_reject_damage() {
        let mut rng = Rng::new(64);
        for _ in 0..2000 {
            let mail = Mail { to: rng.next(), sid: rng.next(), kind: 1 + rng.below(2) as u8, ttl_ms: rng.below(60_001) as u32, sdp: text(&mut rng, 300) };
            let p = profile(&mut rng);
            let mut w = Writer::default();
            mail.encode(&mut w);
            p.encode(&mut w);
            let mut r = Reader::new(&w.buf);
            assert_eq!((Mail::decode(&mut r), Profile::decode(&mut r)), (Some(mail), Some(p)));
            assert!(r.is_empty());
            let cut = rng.below(w.buf.len() as u64) as usize;
            let mut r = Reader::new(&w.buf[..cut]);
            assert!(Mail::decode(&mut r).and_then(|_| Profile::decode(&mut r)).is_none());
            let mut bad = w.buf.clone();
            let at = rng.below(bad.len() as u64) as usize;
            bad[at] ^= 1 << rng.below(8);
            let mut r = Reader::new(&bad);
            let _ = Mail::decode(&mut r).and_then(|_| Profile::decode(&mut r));
        }
        let mut w = Writer::default();
        Mail { to: 1, sid: 2, kind: 3, ttl_ms: 5, sdp: String::new() }.encode(&mut w);
        assert!(Mail::decode(&mut Reader::new(&w.buf)).is_none());
        w.buf[16] = 1;
        w.buf.splice(17..18, [0xe1, 0xd4, 0x03]); // ttl 60,001
        assert!(Mail::decode(&mut Reader::new(&w.buf)).is_none());
    }

    #[test]
    fn frame_limits() {
        let part = |index, count, len| Frame::Part { msg_id: 1, index, count, data: vec![0; len] }.encode();
        assert!(Frame::decode(&part(0, 1, PART_DATA)).is_some());
        assert!(Frame::decode(&part(0, 1, PART_DATA + 1)).is_none());
        assert!(Frame::decode(&part(1, 1, 4)).is_none());
        assert!(Frame::decode(&part(0, 0, 4)).is_none());
        assert!(Frame::decode(&part(0, MAX_PARTS as u16 + 1, 4)).is_none());
        assert!(part(0, 1, PART_DATA).len() <= MAX_MESSAGE);
        let pts = |from| Frame::InkPts { live: 1, from, xy: vec![0.0; 2], p: None }.encode();
        assert!(Frame::decode(&pts(MAX_LIVE_POINTS as u32 - 1)).is_some());
        assert!(Frame::decode(&pts(MAX_LIVE_POINTS as u32)).is_none());
        // Role 0 (viewer) and another protocol version are not peers.
        let mut hello = Frame::Hello {
            doc_id: "d".into(),
            inst: 1,
            vv: VersionVector::default(),
            profile: Profile { device_type: b'p', ..Default::default() },
            presence: PresenceState { mode: PresenceMode::Editing, page: Id(0), tool_colour: Color::Hex(0), save_phase: 0 },
        }
        .encode();
        assert!(Frame::decode(&hello).is_some());
        hello[13] = 0;
        assert!(Frame::decode(&hello).is_none());
        hello[13] = 1;
        hello[1] = 2;
        assert!(Frame::decode(&hello).is_none());
        // Profile fields are clipped on a character boundary when written and bounded when read.
        let long = Profile { email: "é".repeat(40), name: "n".repeat(30), device_type: b'x', ..Default::default() };
        let mut w = Writer::default();
        long.encode(&mut w);
        let back = Profile::decode(&mut Reader::new(&w.buf)).unwrap();
        assert_eq!((back.email.len(), back.name.len(), back.device_type), (38, 22, b'p'));
        let mut w = Writer::default();
        put_str(&mut w, &"e".repeat(39));
        assert!(Profile::decode(&mut Reader::new(&w.buf)).is_none());
    }

    #[test]
    fn parts_reassemble_in_any_order() {
        let mut rng = Rng::new(63);
        for len in [0usize, 1, PART_DATA - 1, PART_DATA, PART_DATA + 1, 5 * PART_DATA + 17] {
            let inner: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            let mut parts = fragment(9, &inner).unwrap();
            assert_eq!(parts.len(), len.div_ceil(PART_DATA).max(1));
            assert!(parts.iter().all(|p| p.len() <= MAX_MESSAGE));
            rng.shuffle(&mut parts);
            let mut re = Reassembly::default();
            let mut out = None;
            for (i, bytes) in parts.iter().enumerate() {
                let Some(Frame::Part { msg_id, index, count, data }) = Frame::decode(bytes) else { panic!("not a part") };
                // An exact duplicate is harmless.
                for _ in 0..2 {
                    match re.push(i as f64, msg_id, index, count, data.clone()) {
                        Assembled::Done(bytes) => out = Some(bytes),
                        Assembled::Pending => {}
                        Assembled::Close => panic!("closed"),
                    }
                    if out.is_some() {
                        break;
                    }
                }
            }
            assert_eq!(out.as_deref(), Some(&inner[..]));
            assert!(re.open.is_empty());
        }
        assert!(fragment(1, &vec![0u8; MAX_ASSEMBLED]).is_some_and(|p| p.len() <= MAX_PARTS));
        assert!(fragment(1, &vec![0u8; MAX_ASSEMBLED + 1]).is_none());
    }

    #[test]
    fn reassembly_limits() {
        let mut re = Reassembly::default();
        // Four assemblies at most; the fifth closes the channel.
        for id in 0..4 {
            assert_eq!(re.push(0.0, id, 0, 2, vec![1]), Assembled::Pending);
        }
        assert_eq!(re.push(1.0, 4, 0, 2, vec![1]), Assembled::Close);
        // After the TTL the stale ones are gone and a late part of them starts over.
        assert_eq!(re.push(ASSEMBLY_TTL_MS, 4, 0, 2, vec![1]), Assembled::Pending);
        assert_eq!(re.open.len(), 1);
        assert_eq!(re.push(ASSEMBLY_TTL_MS + 1.0, 4, 1, 2, vec![2]), Assembled::Done(vec![1, 2]));
        // A duplicate with different bytes, or another count for the same id, drops the assembly.
        assert_eq!(re.push(0.0, 7, 0, 3, vec![1]), Assembled::Pending);
        assert_eq!(re.push(0.0, 7, 0, 3, vec![2]), Assembled::Pending);
        assert!(re.open.is_empty());
        assert_eq!(re.push(0.0, 8, 0, 3, vec![1]), Assembled::Pending);
        assert_eq!(re.push(0.0, 8, 4, 5, vec![1]), Assembled::Pending);
        assert!(re.open.is_empty());
        // At most 64 MiB reserved per peer.
        let big = (MAX_ASSEMBLED / PART_DATA) as u16;
        assert_eq!(re.push(0.0, 1, 0, big, vec![1]), Assembled::Pending);
        assert_eq!(re.push(0.0, 2, 0, 3, vec![1]), Assembled::Close);
        // Out-of-range fields close.
        assert_eq!(Reassembly::default().push(0.0, 1, 2, 2, vec![]), Assembled::Close);
        assert_eq!(Reassembly::default().push(0.0, 1, 0, 0, vec![]), Assembled::Close);
    }
}
