//! Ids and actors (ARCHITECTURE §4.1, SYNC §2): `Id(u128)`, `ActorId(u64)`, import actors, the id generator and
//! the deterministic 5.x id mapping (FNV-1a).

/// SHA-256 of an asset (ARCHITECTURE §4.5).
pub type Hash = [u8; 32];
/// A document is always named by its Drive file id (SYNC §2).
pub type DocId = String;

/// Set on import and migration actors; never set on an installation id.
pub const IMPORT_BIT: u64 = 1 << 63;
/// Id counters are reserved in blocks of this size (`kv[account,'ctr:'+actor]`).
pub const ID_BLOCK: u64 = 65_536;

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

pub fn fnv1a128(bytes: &[u8]) -> u128 {
    bytes
        .iter()
        .fold(0x6c62_272e_07bb_0142_62b8_2175_6295_c58d, |h, &b| (h ^ b as u128).wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b))
}

/// `inst | slot` for a tab, or `IMPORT_BIT | (fnv1a64(source) >> 1)` for an imported 5.x revision / migration.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct ActorId(pub u64);

impl ActorId {
    /// Makes a random u64 a valid installation id: low 8 bits (the tab slot) and the import bit cleared.
    pub fn inst_from_random(random: u64) -> u64 { random & !0xff & !IMPORT_BIT }
    /// The actor of tab `slot` of installation `inst`.
    pub fn tab(inst: u64, slot: u8) -> ActorId { ActorId(Self::inst_from_random(inst) | slot as u64) }
    /// The import actor of a Drive head revision id, or of [`migration_source`].
    pub fn import(source: &str) -> ActorId { ActorId(IMPORT_BIT | (fnv1a64(source.as_bytes()) >> 1)) }
    pub fn is_import(self) -> bool { self.0 & IMPORT_BIT != 0 }
    /// Installation of a tab actor (meaningless for import actors).
    pub fn inst(self) -> u64 { self.0 & !0xff }
    pub fn slot(self) -> u8 { self.0 as u8 }
}

/// `inst` in lowercase base 36, left-padded with `0` to 13 characters (SYNC §2).
pub fn inst13(inst: u64) -> String {
    let mut out = [b'0'; 13];
    let mut v = inst;
    for slot in out.iter_mut().rev() {
        *slot = b"0123456789abcdefghijklmnopqrstuvwxyz"[(v % 36) as usize];
        v /= 36;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Source string of the migration actor of one document: `"migrate:" ‖ inst13 ‖ ":" ‖ docId`.
pub fn migration_source(inst: u64, doc_id: &str) -> String { format!("migrate:{}:{doc_id}", inst13(inst)) }

/// Page, item and version id. New ids are `(actor << 64) | counter`; 5.x string ids map to `fnv1a128(string)`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Id(pub u128);

impl Id {
    pub const NONE: Id = Id(0);
    pub fn new(actor: ActorId, counter: u64) -> Id { Id((actor.0 as u128) << 64 | counter as u128) }
    /// Deterministic id of a 5.x string id: every device importing the same data derives the same id.
    pub fn legacy(v5_id: &str) -> Id { Id(fnv1a128(v5_id.as_bytes())) }
    /// 32 lowercase hex digits (`data-page-id`, `data-version-id`; BEHAVIOUR-CHANGES N-11).
    pub fn hex(self) -> String { format!("{:032x}", self.0) }
    pub fn from_hex(s: &str) -> Option<Id> {
        (s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u128::from_str_radix(s, 16).ok().map(Id))?
    }
}

/// Id generator of one actor. The counter is shared by every document the tab edits.
///
/// Crash safety: ids leave the tab only inside committed batches, so the session persists [`IdGen::persist_mark`]
/// in the same transaction as the batch that used them; after a restart the generator resumes at the persisted
/// mark and never reissues an id.
#[derive(Clone, Debug)]
pub struct IdGen {
    pub actor: ActorId,
    next: u64,
    reserved: u64,
}

impl IdGen {
    /// `high_water` = the persisted `kv[account,'ctr:'+actor]` (0 for a new actor). Counters start at 1.
    pub fn new(actor: ActorId, high_water: u64) -> IdGen { IdGen { actor, next: high_water.max(1), reserved: high_water } }
    pub fn take(&mut self) -> Id {
        let id = Id::new(self.actor, self.next);
        self.next += 1;
        id
    }
    /// `Some(new high-water mark)` when ids beyond the reserved block were issued: the caller stores it with the
    /// batch. The mark is the end of the block that contains the last issued counter.
    pub fn persist_mark(&mut self) -> Option<u64> {
        (self.next > self.reserved).then(|| {
            self.reserved = self.next.div_ceil(ID_BLOCK) * ID_BLOCK;
            self.reserved
        })
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.id.<name>', args)`.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    match name {
        "legacy" => Some(format!("\"{}\"", Id::legacy(args).hex())),
        "import_actor" => Some(format!("\"{:016x}\"", ActorId::import(args).0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_the_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a128(b"a"), 0xd228_cb69_6f1a_8caf_7891_2b70_4e4a_8964);
        assert_eq!(fnv1a128(b"foobar"), 0x343e_1662_793c_64bf_6f0d_3597_ba44_6f18);
        assert_eq!(Id::legacy("a").hex(), "d228cb696f1a8caf78912b704e4a8964");
        assert_eq!(Id::from_hex(&Id::legacy("st_legacy_1").hex()), Some(Id::legacy("st_legacy_1")));
        assert_eq!(Id::from_hex("xyz"), None);
    }

    #[test]
    fn actors_keep_slot_and_import_bit_apart() {
        let inst = ActorId::inst_from_random(u64::MAX);
        assert_eq!(inst, 0x7fff_ffff_ffff_ff00);
        let a = ActorId::tab(inst, 7);
        assert_eq!((a.inst(), a.slot(), a.is_import()), (inst, 7, false));
        let i = ActorId::import("0B7revision");
        assert!(i.is_import());
        assert_eq!(i, ActorId::import("0B7revision"));
        assert_ne!(i, ActorId::import("0B7revisioN"));
        assert_eq!(inst13(0), "0000000000000");
        assert_eq!(inst13(35), "000000000000z");
        assert_eq!(inst13(u64::MAX), "3w5e11264sgsf");
        assert_eq!(migration_source(36, "doc1"), "migrate:0000000000010:doc1");
    }

    #[test]
    fn id_generator_reserves_blocks_and_never_reissues() {
        let actor = ActorId::tab(0x1200, 3);
        let mut g = IdGen::new(actor, 0);
        assert_eq!(g.take(), Id::new(actor, 1));
        assert_eq!(g.persist_mark(), Some(ID_BLOCK));
        assert_eq!(g.persist_mark(), None);
        for _ in 0..ID_BLOCK {
            g.take();
        }
        assert_eq!(g.persist_mark(), Some(2 * ID_BLOCK));
        // A restart resumes at the persisted mark: above every id that may have been issued.
        let mut h = IdGen::new(actor, 2 * ID_BLOCK);
        assert!(h.take() > g.take());
        assert_eq!(Id::new(actor, 5).0 >> 64, actor.0 as u128);
    }
}
