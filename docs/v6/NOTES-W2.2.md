# W2.2 V5-READERS — notes for the units that use the 5.x readers

Files: `rust/crates/ihn-core/src/v5/*` (branch `v6-w2_2`). `cargo test -p ihn-core --lib v5::` → 27 tests, ~4 s
debug, run against W1.2's fixtures (`tests/v6/fixtures/v5`, read through the test-only `v5/fixtures.rs`).

## What is there

| Module | Provides |
|---|---|
| `mod.rs` | `V5Error` (5.18.0 messages), limits (`MAX_PDF_BYTES`, `MAX_PDF_PAGES`, section limits, `WORKER_PARSE_THRESHOLD`), `check_pdf_blob`, `check_page_count`, `simple_hash`, `legacy_page_id`, `legacy_item_id`, `local_timeline_key`, `to_base36` |
| `keywords.rs` | `parse` (sections located exactly as 5.18.0), `is_v5`, `section_names`, `strokes_json`, `structure_value`, `fields_value`, `sync_envelope`, `b64_zlib` |
| `json.rs` | `Value` (also the input of `idb`: structured clones, `Bytes`, `Blob`), `parse`, `stringify`, `stable` (5.x hashes) |
| `pages.rs` | `V5Page`/`V5Stroke`/`V5Image`/`V5Stamp`, `page`, `entries`, `deletion_stamps`, `dimension` |
| `structfields.rs` | `structure` (`IH_STRUCT`), `fields` (`IH_FIELDS`) |
| `timeline.rs` | `read`, `read_json` (schema 3 archive with integrity hashes, legacy array), `hash_string`, `content_hash` |
| `idb.rs` | `meta_payload`, `page_row`, `stroke_ops`, `working_copy`, `timeline_row`, `snapshot`, `offline_document`, `outbox`, `iso_ms` |
| `checkpoint.rs` | `unpack` (`xor-v1` bit-exact, `raw-v1`), `range`, `xor_encode/decode` |
| `inflate.rs`, `sha256.rs` | zlib inflate with limits; SHA-256 for asset hashes (an import is one pure function) |
| `import.rs` | `from_pdf`, `supersede`, `from_local`, `source_hash`, `overlay_refs`, `PdfFacts`, `V5LocalDoc`, `Import`, `Asset`, `Source` |

## How to use the importer (W3.4, W4.5/SYNC §15.3, W5.x migration)

- **Drive revision / user import:** `from_pdf(keywords, &PdfFacts{doc_id, name, page_sizes, revision_sha256,
  clean_attachment, strict}, ActorId::import(head_revision_id), modified_ms)` → `Import{batches, assets, versions,
  source, legacy_baked, needs_resave, content_hash, strokes, pages, v5}`. Store `assets` (inline images) before
  applying; `source = (Source, Hash)` says which bytes the `Background::Pdf` pages need under that hash:
  `Saved` = the downloaded file, `Stripped` = the file with `overlay_refs(keywords)` XObjects removed (export worker),
  `Attachment` = `IH_CLEAN_ORIGINAL.pdf`. Hash = `source_hash(revision_sha256, source)`.
  `strict: true` for Drive/offline/shared opens (errors carry 5.18.0's text), `false` for "Import PDF".
- **Every later revision of a document (SYNC §15.3):** apply `from_pdf`'s batches **and**
  `supersede(&doc, &batches, &keep)` (when `Some`). 5.x devices delete strokes by leaving them out of the next save
  (fixture `r1` drops `rev-s2`/`rev-s5` with no tombstone), so a union would resurrect them. `keep` = the migration
  actors of this installation (their local-only strokes are never deleted by a revision). Order-independent: R0 then
  R1, or R1 then R0, give the same registers (cargo test).
- **Migration:** build a `V5LocalDoc` with `V5LocalDoc::from_working(doc_id, &meta, idb::working_copy(..), name)` and
  `V5LocalDoc::from_snapshot(doc_id, &snapshot, updatedAt, baseMeta.modified_ms, source_asset)`, then
  `from_local(&src, migration_actor, Some(&doc))` for each source in turn (seq and stamp indexes continue from what
  `doc` holds, so several sources and resumed runs never reuse a `(actor, seq)`; a second run emits no puts).
- **Versions:** `Import::versions` are normalised `V5Version`s (ids, ts, author, summary, kind, hashes, pages); the
  conversion to `VersionAdd` belongs to `history` (W2.9) / `doc/versions.rs` (W5.3), whose record codec is not
  defined yet.

## Deviations (closest correct option)

1. **`from_pdf`/`from_local` return `Import`, not `Vec<Batch>`**: the caller also needs the assets found in the data
   (images), the timeline, the source variant and 5.18.0's flags (legacy-baked, `needsMigrationResave`,
   `driveConfirmedContentHash`). `from_pdf` takes `&PdfFacts` with the fields above (attachment hashes were not
   needed; `doc_id`, `name` and `strict` were).
2. **`supersede` is new** (see above): without it R0+R1 ≠ R1 for the real fixtures. Rule: if the revision is the
   newest held (page positions carry the revision stamp), live items/pages whose newest write is a 5.x/import stamp
   older than it and that it lacks are deleted at `{rev_time, rev_actor, low32(id)}`; if it is older, what only it
   has is deleted at the newest revision's stamp. Its batch actor is derived from its content.
3. **5.x stamp actors are `ActorId::import("v5:" + actor)`** (ARCHITECTURE said `fnv64(actor)`): same hash family,
   but in the import domain, which is how `supersede` tells registers written from 5.x data from native v6 writes.
4. **Page order comes from the `STROKES_*` entry order, not `IH_STRUCT` ranks**, always stamped with the revision:
   5.18.0 displays entry order (fixture `structure-changed` has ranks 0/1024/1024 that disagree with it), and
   rank-derived positions under 5.x order stamps would break the one-value-per-stamp rule when another page moves.
   Page registers (template, size, background, side panel) are stamped with the revision too.
5. **Migrated content uses the migration actor for every register** (5.x clock, else `savedAt`), not the 5.x stamp,
   so `supersede(…, keep)` can protect it.
6. **Repaired ids are deterministic**: a repeated stroke id without `IH_NORM` becomes `id~pageId~index` (5.18.0:
   random); a document without a Drive id (user import) uses `import:<actor hex>` as the key of legacy ids (5.18.0:
   random). A 5.x id already used on an earlier page maps to `Id::legacy("pageId:id")` (5.x allows it per page).
   The fixture tests compare ids only where 5.18.0's are not random.
7. **Clean-original preference**: overlay strip first (1:1 page mapping by construction), then the attachment;
   5.18.0 tried IDB → attachment → strip. Pixels are the same; this avoids the documented page-mapping defect.
8. `eraser-stroke` strokes are not imported (5.18.0 never draws them); counted in `Import::strokes` like 5.18.0.
   Images whose `src` is not a base64 data URL (dead `blob:` URLs in local records) are skipped.
9. Pressure is `f32` in the model (W2.1), so the fixture point hash formats pressure with Rust's shortest `f32` text.

## Acceptance

- Every Drive fixture (15) imports with 5.18.0's page count, page ids, sizes, templates, backgrounds/`pdfPageIndex`,
  side panels, stroke ids in draw order, tools, colours, widths, point counts, `pointsSha256`, images (rect and
  `srcSha256`), cover strokes, legacy-baked flag, name and timeline entries; twice → identical (`Import` equality).
- R0/R1 both orders (and re-imports) → identical registers, visible state = R1 alone; a v6 edit after R0 survives;
  migrated strokes survive with `keep`.
- Storage dumps (8 scenarios): working cache (+ background database) and offline snapshot migrated over the base
  fixture give 5.18.0's page/stroke sets; every unsynced stroke is emitted; nothing the base already has; idempotent.
- `xor-v1` bit-exact (codec tests + the `xor-packed` rows), strict-mode and limit failures with 5.18.0's strings.
- Parity: not run — this unit owns no UI state (pure `ihn-core`). `npm run build` (report-only size gate) passes.
