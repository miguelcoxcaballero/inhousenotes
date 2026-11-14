# W2.9 HISTORY — notes for the units that build on it

File: `rust/crates/ihn-core/src/history.rs` (tests inline). Branch `v6-w2_9`. `cargo test -p ihn-core history`:
6 tests, ~1.5 s debug.

## API

- `Record` = the opaque `VersionMeta.data` (`encode`/`decode`): `ts` (ms), `email`, `name`, `photo`, `device`,
  `kind`, `milestone`, `summary`, `content` (FNV-1a 64 of the self-contained snapshot encoding = 5.x
  `contentHash`), `snapshot` (SHA-256 of the asset), `base` (asset this one is a delta of), `depth`.
- `entries(doc)` → listed `(Id, Record)` oldest first (`(ts, id)`), what the timeline panel lists.
- `Snapshot{name, pages: [SnapPage{id, template, size, background, side_panel, items: [SnapItem{id, z, body}]}]}`;
  `Snapshot::of(doc)`, `encode(base)`, `content_hash()`; `materialize(hash, &|h| assets.get(h))` follows the chain
  and fails closed (`None`) on any missing/damaged asset.
- Capture (W5.3 `doc/versions.rs`): `plan_capture(doc, actor, &Who{email, name, photo, device}, ts, hint, assets)`
  → `None` (no pages / identical content) or `Planned{replace, record, asset}`; hash `asset` (SHA-256), store it in
  `assets`, then `planned.commit(hash, &doc, &mut tx)` pushes the `VersionAdd` (coalesced: same id, LWW rewrite)
  plus the prune `VersionDel`s. `Hint{milestone, kind, summary}` = 5.x `pendingVersionHint`
  (`pre-restore` with `Before restoring <date>`, `restore`, `revert-original`, `manual`).
- `needed_assets(doc)`: snapshot assets to keep (listed versions + their chain bases, also pruned ones). GC and the
  export's `IH_VERSIONS` budget selection use it.
- `plan_restore(doc, &target)` → ops for one `Tx` (after the pre-restore capture; capture again with the restore
  hint afterwards). Applying them makes `Snapshot::of(doc).pages == target.pages` (name when the target has one).
- `summarize(prev, next)` (5.x strings), `diff_page(prev, next)` / `diff(prev_entry, entry)` → `Highlight{x, y, w,
  h, mark: Added|Changed|Removed}` in page coordinates, ready for `drawTimelineDiffHighlights`' canvas calls.
- `test_cmd('constants')`.

## Rules for callers

- The asset closure must be able to serve this actor's chain tail and the newest entry's snapshot; when it cannot,
  capture falls back to a checkpoint / the `Saved` summary (never fails).
- Imported 5.x `IH_VERSIONS` (W2.2): build a `Snapshot` per entry, `encode(None)` (or as a chain) and a `Record`
  with `content = snapshot.content_hash()`; version ids `Id::legacy(5.x id)`.

## Deviations

1. Tests and the recorded 5.18.0 fixture are inline in `history.rs` (the unit owns one file). The fixture was
   recorded by running 5.18.0's own `collectTimelineDiffHighlights`/`summarizeVersionDelta` (extracted from
   `app-v5.js`) over 11 seeded page pairs; outputs compared bit for bit.
2. Coalescing also requires the newest entry to belong to this actor (chains stay within one actor).
3. Prune: 5.x branch-id regrouping (`id~hash`) is not ported (v6 ids never collide); duplicate-content ties go to
   the greater id instead of the larger stable JSON. `parentId` is not stored (never shown).
4. Restore puts only target items whose page, z or body differ or that are not live, instead of re-stamping every
   item as 5.18.0 does (same visible result, far fewer bytes; the batch is still a new causal write).
5. Checkpoints every 8 versions per actor; milestones are not forced to be checkpoints. Snapshot deltas list every
   live item id (17 bytes for an unchanged item); a delta with 30 new strokes on a ~1,500-stroke doc is ~1/4.5 of
   the checkpoint.
