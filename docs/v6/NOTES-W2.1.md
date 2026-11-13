# W2.1 MODEL — notes for the units that build on it

Files: `rust/crates/ihn-core/src/{id,stamp,op,doc,pos,snapshot,geom}.rs`. Branch `v6-w2_1`, based on `v6-w1_4`
(see deviation 1). `cargo test -p ihn-core`: 58 tests, ~6 s debug.

## What is there

- `id`: `Id(u128)`, `ActorId` (`tab`, `import`, `inst_from_random`), `IMPORT_BIT`, `fnv1a64/128`, `Id::legacy`,
  `inst13`, `migration_source`, `IdGen` (blocks of 65,536; persist `persist_mark()` in the same transaction as the
  batch), `IdMap`/`IdSet` (cheap hasher for `Id` keys), `Hash`, `DocId`.
- `stamp`: `Stamp`, `Reg<T>` (LWW), `Clock` (`next(wall)`, `observe(clock, wall)` with the 24 h horizon; rebuilt by `Doc::clock`),
  `VersionVector` (contiguous + extras; `encode/decode` = SYNC §4.1 `VV`, extras separately).
- `pos`: `Pos` (base-256 fractional key), `Pos::between`, `Pos::rank(i)` for imports.
- `op`: `Op`, `Batch` (`stamp(i)`, `max_clock()`), `Body`, `Z` (`Z::TOP` placeholder, `fragment(k)`), `Color`,
  `Tool`, `Template`, `Background`, `PageProp`, `ImageRect`, `VersionMeta`.
- `doc`: `Doc::{apply, apply_owned, apply_part, merge, order, draw_list, page_items, hit_strokes, split_strokes,
  lasso, selection_bounds, inverse, chunk_items, chunk_pages, take_dirty, reindex, clock, max_clock}`, `Changes`,
  `Author` (`new`, `resume`), `Tx`,
  and the §4.4 interface: `Replica`, `SyncEngine`, `SyncStatus`, `ExportPhase`, `SyncError`, `ReceiveResult`,
  `PeerRef`, `PeerView`, `PeerLink`, `LivePreview`, `PresenceState`, `PresenceMode`, `Compacted`, `BlobHandle`.
- `snapshot`: SYNC §4.2–4.3 batch/op codec (`encode_batch`, `decode_batch`, `BatchDecoder`, `EncodedBatch::header`),
  field codecs for `ihn-sync::wire` (`put_/get_` `id, hash, bytes, str, stamp, z, color, body`), and the snapshot
  format (layout in the module header): `encode`, `encode_header`, `encode_chunk`, `Loader`, `decode`.
- `geom`: `within`, `point_segment_within`, `segment_distance`, `Sweep` (swept capsule), `polyline_hit`,
  `split_ranges`, `bounds`, `eraser_pad`, lasso tests, `image_corners`, `SelectionBounds`, `Affine`.

## How to use it

- **Opening a document (W3.1 — mandatory):** the writer's clock is not stored anywhere (the `docs` row has `seq`
  and `kv` has the id mark, nothing else; snapshot bytes must not depend on the device). After the chunks are
  loaded and the journal is applied, build the writer with
  `Author::resume(IdGen::new(actor, mark), seq, &doc, wall_ms)`. It sets `clock.observed` to the greatest stamp in
  the document within the 24 h horizon and `clock.last` to the greatest stamp of this actor (`Doc::clock`, one pass
  over the registers). **Never use `Author::new` for a document that has content**: a clock restarting at zero
  stamps below the existing registers whenever a peer was ahead or the wall clock stepped back, and LWW then drops
  the local edit on every replica (a moved stroke snaps back, a new stroke lands underneath). `Author::new` is for
  a brand-new empty document and tests. If the writer is created before the load finishes, call
  `author.clock = doc.clock(actor, wall_ms)` once it has.
- **Snapshot merge:** stamps arrive without batches, so observe them:
  `author.clock.observe(other.max_clock(wall_ms), wall_ms)` then `doc.merge(&other)`.
- **Local edit:** `let mut tx = Tx::new(&doc, &mut author, wall_ms); …; let b = tx.commit(); let inv =
  doc.inverse(&b); doc.apply(&b);` — `inverse` must run before `apply`. Skip empty transactions (`tx.is_empty()`).
- **Undo/redo (`editor/history.rs`):** undo = push the stored inverse ops into a new `Tx` (`tx.push`) and commit; the
  **redo ops are `doc.inverse(&undo_batch)`**, taken before applying the undo batch — not the original forward ops
  (re-applying a forward `ItemPut` would not undelete the item). Redo's inverse becomes the next undo entry.
- **Remote batch:** `clock.observe(b.max_clock(), wall_ms)` then `doc.apply_owned(b)`; for a large one use
  `BatchDecoder::step` and `doc.apply_part(&mut b, range)` in idle slices (a 20,000-op batch: ~0.8 ms per 500 ops on
  the build machine; indexes are consistent after every slice; the batch enters `vv` with the last slice).
- **Store:** header chunk = `encode_header`, page chunk = `encode_chunk(doc, page)` for each page of
  `doc.take_dirty()`; load = `Loader::header` + `Loader::chunk`… + `finish()`. Snapshot merge =
  `doc.merge(&snapshot::decode(bytes)?)`.
- **Erasers:** `hit_strokes(page, x0, y0, x1, y1, r)` with the previous and current sample (same point twice = one
  sample = 5.18.0 exactly). Area eraser: `doc.split_strokes(page, &sweep, r)` for the first cut, `geom::split_ranges`
  on the tool's working fragments afterwards, `tx.split_ranges(item, &ranges)` at pen-up with ranges relative to the
  original stroke.
- **Selection transform:** `Affine::new(centre, sx, sy, rot, tx, ty)` reproduces `applyTransformToSelection`
  bit for bit (a pure move is `Affine::new(centre, 1, 1, 0, dx, dy)`, not `p + d`).

## Rules writers must keep (the model relies on them)

1. **`ItemDel{deleted:false}` is always preceded in the same batch by an `ItemPut` of that item** (`inverse` and
   restore plans do this). A snapshot omits the body of an item whose `deleted` stamp is greater than its body
   stamp; the put that comes with the undelete has a greater stamp and revives it on every replica. An undelete
   without a put could show an item that has a body on one replica and none on another.
2. **A register is never written twice with the same stamp and different values.** With equal stamps the first
   write stays. Native stamps are unique; importers (W2.2) must give two different values of one register different
   stamps (e.g. the revision time + import actor), and identical stamps only to identical values.
3. Page sizes are ≥ 64 and `Pos` keys are non-empty without a trailing zero byte: the batch decoder rejects anything
   else (`Tx` clamps sizes; use `Pos::rank`/`Pos::between`).
4. `Body::Dropped` never goes into an op (the decoder rejects it).
5. A writer of a loaded document is built with `Author::resume` (see above). Known limit, accepted: `resume` reads
   the registers, so a batch of this actor whose every write was since overwritten by a stamp **beyond** the horizon
   (a peer with a corrupt clock) is not counted; those writes had already lost, and the only effect is that the
   next batch may be stamped below that lost one.

## Deviations from the plan / architecture text

1. **Based on `v6-w1_4`, not `v6-w1_1`**: `geom` needs `jsmath::{hypot, sin, cos}` and the architecture allows only
   one `hypot` (W1.4's). `v6-w1_4` is a fast-forward of `v6-w1_1`, nothing else came with it.
2. **`Tx` needs a writer identity**: `Tx::new(doc, &mut Author, wall_ms)`; `Author{actor, ids, clock, seq}` holds what
   `commit` assigns (ARCHITECTURE shows `Tx{doc, ops}` and `commit(self)`, which cannot produce ids or a clock).
3. **`Tx::split(item, fragments)` returns the new ids** and has a sibling `split_ranges(item, &[(a, b)])` that keeps
   pressure and avoids copies. Extra `Tx` methods: `push`, `page_set`, `page_restore`, `version_add`, `version_del`.
4. **`Body::Dropped`** (third variant) = "the body a snapshot dropped / an item known only by its `ItemDel`".
5. **`VersionMeta{id, data}`**: `data` is the opaque record of `history.rs` (W2.9 owns its codec); the wire
   `bytes meta` = `Id ‖ data`. `Doc::versions: BTreeMap<Id, Version{data: Reg, deleted: Reg}>`.
6. **`SyncEngine` has associated types `Effect` and `Io`** because `SyncEffect`/`SyncIo` live in `ihn-sync`, which
   depends on `ihn-core`. `PeerRef` is defined in `ihn-core::doc` for the same reason (W2.6 re-exports it).
7. **`SmallVec<Id>` = `Vec<Id>`** (no external crates).
8. **No spatial grid**: each page's draw list carries bounds and width, and hit tests scan it contiguously
   (≈ 170 µs for a 21,000-stroke page with dense hits, a few µs for ordinary pages). A grid can be added behind
   `hit_strokes` without an API change.
9. **`codec.rs` strings are u32-length-prefixed (ABI)**; SYNC §4.1 `bytes`/`str` are `uv`-prefixed, so those two
   primitives are `snapshot::{put_bytes, get_bytes, put_str, get_str}`.
10. **`Doc::apply` skips a batch already in `vv`** and reports `Changes{applied: false}`; `seq == 0` is never a
    batch. `PageNew` does not write `deleted`; a page is listed once its `PageNew` (or a size write) arrived.
11. `Color::parse` maps `#rrggbb` of either case to `Hex`; `css()` writes lowercase.

## Acceptance (all in `cargo test -p ihn-core`)

- Convergence: `doc::tests::replicas_converge_under_reordering_duplication_loss_and_snapshots` (2–5 replicas ×
  2,000 actions each, plus 30 short seeds; `IHN_CONVERGE_SEEDS=N` soaks more — 4,000 seeds passed), every op pair
  commutes, apply twice = once, imports R0/R1 in both orders.
- Stamps: same-millisecond batches reversed; far-future clock applied, not observed; VV contiguity + extras;
  `reload_resumes_the_clock` (reload after a peer that was ahead, after the wall clock stepped back two days, and
  with a far-future stamp in the document). The convergence and undo tests rebuild the writer from the store at
  every compaction (`Rep::reload`).
- Undo: inverse of every `Tx` restores the visible state (also across a compaction), SYNC §5 table, revival of a
  dropped body, undo under concurrent remote edits.
- Z rules: area-erase fragments and undo at the original z, cross-page move on top, covers → images → strokes.
- Snapshot: canonical round trip, chunked loading, merge, damage never panics, size guard (20 pages × 150 strokes).
- Geometry: ported `editor.rs` / `editor_input` tests (hit, 1-ULP thresholds, split vs. literal original), swept
  capsule vs. dense sampling, lasso, affine order.
