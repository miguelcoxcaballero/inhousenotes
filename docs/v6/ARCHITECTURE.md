# Inhouse Notes 6.0.0 — Architecture

Status: definitive design by the lead architect, revision 2 (2026-10-30, after review). Binding inputs: `REQUIREMENTS.md` (user decisions),
`spec/*.md` (5.17.22 behaviour; **5.18.0 at `1d6e8a0` wins where they differ**), `spec/existing-rust-and-rebuild.md`
(reuse verdicts). The sync wire protocol is specified by the peer document `SYNC.md`; this document owns the
document model, stamps and actors, the core↔sync interface, the storage schema (including every field sync
persists), rendering, the browser boundary, the build and the release. Where both documents touch a topic, the text
here is normative for the model/interface/storage and SYNC.md is normative for transports, export triggers and
authorship (SYNC §14).

Reading order for builders: §1 (shape), §2 (crates and files), §3 (browser boundary and ABI), then the section of
your work unit. `BUILD-PLAN.md` says which files you own; `TESTING.md` says how you prove parity.

---

## 0. Principles (the decisions every unit must respect)

1. **The UI is 5.18.0's, not a re-creation.** `index.html` (markup and the single `<style>` block), `offline-v5.css`,
   `vendor/fonts/*`, `icons/*`, the agenda/diary SVG bytes and the shared updater files are copied **byte for byte**.
   Every element that 5.18.0 creates at runtime is created by v6 with the same tag, classes, attributes, inline
   styles and text, in the same parent and order. Every Canvas 2D call 5.18.0 makes for visible output, v6 makes
   with the same arguments in the same order. Port, don't reinvent: each Rust UI/render function carries a comment
   `// v5: app-v5.js:<function>` naming the 5.18.0 function it reproduces, and the builder reads that function.
2. **Rust owns state and decisions; JavaScript only touches the platform.** Document model, edit operations,
   undo, layout, hit testing, render geometry, Home catalogue, Drive request construction, auth state machine,
   migration, sync protocol and every UI state machine live in Rust. JS applies DOM/canvas command buffers,
   performs I/O the Rust side asked for, and forwards events. No JS fallbacks.
3. **Collaborative by construction.** Every mutation is a stamped, idempotent, commutative operation on a
   state-based CRDT (§4). Local edits, undo, remote ops, migration and import all go through the same `apply`.
   No operation requires causal delivery, so sync only needs at-least-once delivery with dedupe.
4. **The PDF in Drive is a derived, self-contained export**, never on the edit path (§8).
5. **Least code.** Zero external crates (runtime and dev). One wasm module. One JS host (< 3,000 lines total
   including boot, service worker and export worker). No compatibility code except the 5.x *readers* (§9) and the
   one-time migration (§10). No dead code: if 5.18.0 has an unreachable function (spec quirk lists), it is not ported.
6. **Same frame, less work.** Pen input is handled exactly when 5.18.0 handles it (coalesced samples on
   `pointermove`, drawing in the next `requestAnimationFrame`) but with no GC, no object allocation per sample,
   no layout reads and no background work during contact.

---

## 1. Shape of the system

```
 ┌──────────────────────────── browser main thread ─────────────────────────────┐
 │ boot.js (sync, <head>)   index.html (5.18.0 markup+CSS, byte-identical)      │
 │        │                                                                      │
 │ host/*.js  ── events ──►  ihn.wasm  (crates ihn-app + ihn-core + ihn-sync)    │
 │   ▲   apply out-buffer ◄──  DOM ops · canvas cmds · effects                   │
 │   │                                                                           │
 │   ├─ DOM / CSS / WAAPI       ├─ IndexedDB (inhouse-notes-v6)                  │
 │   ├─ Canvas 2D + Path2D      ├─ fetch (Drive REST, OAuth, GitHub catalogs)    │
 │   ├─ GIS / InhouseNative     ├─ WebRTC + BroadcastChannel (per SYNC.md)       │
 │   └─ pdf.js (lazy, own worker)                                                │
 └───────────────────────────────────────────────────────────────────────────────┘
   export-worker.js : pdf-lib + a second instance of ihn.wasm (PDF assembly, §8)
   export-worker.js also runs large 5.x imports/migration parsing (§9.1) so their memory is released afterwards
   service-worker.js: shell cache only (same URL, classic script, same policy as 5.18.0; §12)
   ?offline-worker  : same page, headless mode (Android WorkManager contract, §12)
```

One compiled `WebAssembly.Module` is shared: the page instantiates it, then `postMessage`s the module to the export
worker (structured clone), which creates its own instance and memory. GitHub Pages cannot send COOP/COEP, so there are
no wasm threads; none are needed.

---

## 2. Crates, modules and files

### 2.1 Repository layout (v6 additions; v5 runtime files stay deployed but unreferenced until 6.0.1, §10.4)

```
index.html                 5.18.0 copy; only <head> version/native map and the script tags differ (§3.1)
boot.js                    from boot-v5.js (theme, skeleton reveal, release watcher, InhouseAssetUrl, InhouseBackground)
service-worker.js          v6 service worker at the SAME URL as 5.18.0, classic script, keeps the line
                           `const VERSION='6.0.0'` (release watcher, §3.7); shell cache only (§12)
host/main.js               wasm loader (instantiateStreaming), ABI, out-buffer dispatcher, event registry, rAF/timers
host/dom.js                DOM op applier, node handles, layout measurement imports
host/canvas.js             canvas command replayer, Path2D and image handle tables
host/idb.js                IndexedDB transaction executor, Web Locks (family 3.9), read-only dumper for 5.x databases
host/fetch.js              fetch with deadline/idle timeout/abort, resumable + keepalive uploads, blob handles,
                           SHA-256 digest (family 3.8), Compression/DecompressionStream (family 3.10)
host/pdf.js                pdf.js adapter (lazy load, open with security-core limits, metadata, attachments,
                           render page → blob URL)
host/peer.js               WebRTC + BroadcastChannel transports (contract in SYNC.md)
host/platform.js           GIS, InhouseNative bridges (OAuth, export bridge v2, background), updater create, app
                           mode, file chooser, clipboard, share, download, fullscreen, history, location, Intl
host/test-api.js           window.__IHN_TEST_API__ (localhost + ?e2e=1 only)
host/export-worker.js      worker: pdf-lib + ihn.wasm instance; assembles PDFs; runs large v5 imports (§9.1)
wasm/ihn.wasm              build output: generated into build/ on the v6 branch, committed only by the release PR (§14.2)
rust/                      cargo workspace (replaces rust-core/)
  Cargo.toml  Cargo.lock
  crates/ihn-core/         pure library: model, codecs, render geometry, 5.x readers. No host access.
  crates/ihn-sync/         pure library: sync protocol state machines (SYNC.md). Depends on ihn-core.
  crates/ihn-app/          the application: UI state machines, input, scheduler, storage/Drive/auth logic,
                           driven through the `Host` trait. Depends on ihn-core, ihn-sync.
  crates/ihn-wasm/         cdylib: `extern "C"` exports/imports binding ihn-app to host/main.js. ~200 lines.
scripts/build-wasm.mjs     reproducible build + import allowlist + size gate (replaces build-rust-core.mjs)
scripts/build-offline-assets.mjs, scripts/build-native-assets.mjs   kept, asset list updated
tests/v6/                  lean suite (TESTING.md): e2e/, parity/, mock/, perf/
```

### 2.2 `ihn-core` modules (pure, `#![no_std]`-compatible style but uses `alloc`/`std` collections; no I/O)

| Module | Responsibility | Reuse |
|---|---|---|
| `id.rs` | `Id(u128)`, `ActorId(u64)` (`inst \| slot`, import actors), id generation from (actor, counter), deterministic legacy ids (128-bit FNV-1a of the 5.x string id) | — |
| `stamp.rs` | `Stamp{clock:u64, actor:ActorId, idx:u32}` hybrid logical clock, total order `(clock, actor, idx)`, observation clamp; `VersionVector` (contiguous + extras) | rebuild-v2 ideas |
| `op.rs` | `Op`, `Batch`, page/item/doc property types (§4) | — |
| `doc.rs` | `Doc` state, `apply`, queries used by the UI, inverse computation for undo, dirty-page tracking | — |
| `pos.rs` | fractional position keys for page order | — |
| `snapshot.rs` | binary snapshot codec (doc header + per-page chunks), op/batch codec | `docio/codec.rs` (branch) for base64 |
| `codec.rs` | LEB128, f64 LE, length-prefixed strings, base64 (one implementation) | `docio/codec.rs` |
| `history.rs` | timeline versions: capture/coalesce/prune rules, version snapshot delta encoding, restore planning, diff for highlights | `editor_render/timeline_diff.rs` algorithm |
| `jsfmt.rs` | ES `Number::toString` (used by `dom.rs` for every number written into the DOM) | `editor_history` (branch) ES formatter |
| `jsmath.rs` | V8-exact `Math.round`, `hypot`, and ports of V8's `src/base/ieee754.cc` (fdlibm) `sin`, `cos`, `atan2`, `exp`, `pow`; std `f64` transcendental methods are banned in `ihn-core`/`ihn-app` by a build-script grep gate (§6.5) | V8 sources, one `hypot` |
| `geom.rs` | bounds, exact point–segment hit test, area-eraser split, lasso point-in-polygon, selection transforms | `rust-core/src/editor.rs`, `editor_input` (branch) algorithms |
| `render/` | `cmd.rs` canvas command encoder; `pen.rs` centripetal Catmull–Rom; `highlighter.rs`; `template.rs` bullet template + SVG template draw; `page.rs` repaint plans (sync, staged, damage); `live.rs` live ink segments; `overlay.rs` lasso/selection/eraser cursor/remote ink; `thumb.rs` previews; `pdfink.rs` PDF content streams (§8) | `pdf_ink.rs` (extended to Bézier), bullet-template.js logic |
| `v5/` | 5.x readers only (§9): `keywords.rs`, `json.rs` (minimal JSON reader), `pages.rs` (STROKES_Z/STROKES_DATA entries), `structfields.rs` (IH_STRUCT/IH_FIELDS), `timeline.rs` (IH_VERSIONS schema 3 + legacy array), `idb.rs` (MetaPayload, page rows, stroke-ops, offline snapshots, outbox), `checkpoint.rs` (xor-v1/raw-v1), `import.rs` (deterministic conversion to `Batch`es) | `rust-core/src/checkpoint.rs` as-is |
| `catalog.rs` | Home catalogue: merge Drive listings with cached entries, order, views (recents/starred/shared/folder), retention | `offline/library.rs` (branch) algorithm |
| `palette.rs` | folder colour maths (HSL, luminance, warm hue, tint) | `drive_home/palette.rs` (branch) as-is |
| `motion.rs` | folder open/close plan (29 values) and label fade, ported 1:1 from 5.18.0 `rust-core-v5.js` `folderMotion/folderFade` | 5.18.0 JS |
| `rows.rs` | variable-height grid rows (prefix sums + binary search), visible window planner for pages and cards | `folder.rs` rows, `editor_view` (branch) |

### 2.3 `ihn-app` modules

| Module | Responsibility |
|---|---|
| `lib.rs`, `app.rs` | `App` root state; event dispatch; scheduler (timers, idle windows, input gating — the `document_work.rs` policy); effect id allocation |
| `abi.rs` | out-buffer writer (records, §3.3); event decoder |
| `dom.rs` | DOM op builder, handle registry, `bind_static(id)`, HTML fragment templates, `jsfmt` for inline style numbers |
| `host.rs` | `trait Host` (§3.4) + `FakeHost` for cargo tests |
| `testapi.rs` | dispatcher for `__IHN_TEST_API__.cmd(name,args)` → `<module>::test_cmd` |
| `shell/` | boot routing (welcome/home/editor/share), theme, welcome sheet, profile menus (both, including the menu `Refresh`; the editor menu's doc-stats block is filled by `doc/stats.rs`), Soon toast, loading overlay, status line, save indicator rendering (§6.6), fullscreen buttons, connectivity probe (`/about?fields=kind`) and `ihn-network-offline`, version labels, global keyboard/click handlers |
| `auth.rs` | token lifecycle, web GIS flow, Android PKCE flow, refresh, reconnect state, sign-out effects |
| `drive/` | `client.rs` request builder (`driveFetch` semantics: deadline incl. token, 401 retry once, error mapping); typed calls in one module per owner: `home.rs` (lists, search, thumbnails, about, open download), `manage.rs` (create folder, upload, copy, move, rename, trash, delete), `perms.rs` (permissions), `revisions.rs` (head/etag/v2 metadata, conditional media PUT, resumable, create with reserved id), `relay.rs` (sync folder, id pool, properties PATCH, relay/archive/asset file I/O) |
| `store/` | IndexedDB schema (§7), transactions as effects, kv, assets, Web Lock usage, retention |
| `home/` | Home view: tabs, lists, cards, folder cards, opened folder view, breadcrumbs, search, pull-to-refresh, auto-refresh, fast scroller, offline decoration and card sync badges (§6.6); `dialogs/` create menu, folder picker, new folder, card ⋮ menu + colour popup, move to bin, bin, upload, offline settings dialog, offline sync-error dialog, download-app modal |
| `editor/` | `view.rs` editor skeleton binding, read/edit mode, title; `layout.rs` page column metrics; `viewport.rs` pan/zoom/wheel/inertia/overscroll/pull-to-add; `pages.rs` page DOM rows, activation (≤3 active), canvas resolution sync, previews; `input.rs` pointer pipeline and tool dispatch; `pen.rs` pen/highlighter; `eraser.rs` stroke/area erasers; `selection.rs` lasso, selection box, transforms, cross-page move; `images.rs`; `history.rs` undo/redo; `toolbar.rs` toolbar drag/snap, tool and colour menus, eraser menu, Draw with finger |
| `doc/` | `links.rs` shared/public link parsing and open (`?drive=…`), viewer mode, signed-out public poll (SYNC §12.4); `session.rs` open document session (model + persistence + sync binding); `open.rs` open from Drive/cache/shared link; `v5open.rs` 5.x and plain-PDF classification, foreign-revision recovery; `create.rs`; `import.rs` user "Import PDF"; `backgrounds.rs` pdf.js page rasters (scale policy, LRU 6, interaction deferral); `save_state.rs` §6.6 status derivation; `stats.rs` editor profile-menu doc stats; `versions.rs` timeline capture trigger (§4.6) |
| `collab/` | binding of `ihn-sync` to the session (§5), presence avatars + popup UI, remote live ink overlay |
| `panels/` | `pages_panel.rs` Manage-Pages sidebar (list, drag reorder, delete, add, Scan→Soon), `photo_page.rs` photo/cover modal, `slider.rs` slide-to-confirm modal, `timeline.rs` timeline panel (list, preview, restore), `share.rs` share modal + permissions |
| `export/` | `pdf.rs` export job planning (§8), `upload.rs` first create (reserved id) and conditional Drive upload, `user.rs` export to the user (modal, bridge v2, share, download), `lifecycle.rs` Home exit / pagehide / visibility / background suspension, `headless.rs` `?offline-worker` runs |
| `migrate.rs` | one-time 5.x local data migration (§10) |
| `updater.rs` | nothing but the app-mode decision and the `[data-inhouse-update-settings]` reveal; the updater itself is JS (§3.7) |

### 2.4 Dependencies

None. `Cargo.lock` lists only the four workspace crates. Property tests use an in-repo seeded PRNG
(`ihn-core/src/testutil.rs`, `#[cfg(test)]`). This keeps the wasm build path-independent (no registry paths) and the
CI reproducible without `--remap-path-prefix` (the flags are still passed, harmlessly).

---

## 3. Browser boundary

### 3.1 What stays JavaScript, and why

| JS file | Why it cannot be Rust | Size target |
|---|---|---|
| `boot.js` | Must run synchronously in `<head>` before wasm can be fetched (theme, first-paint skeletons, release watcher). Copied from `boot-v5.js`, unchanged logic. | ~190 lines |
| `host/*.js` | Platform APIs: DOM, Canvas, IndexedDB, fetch, WebRTC, GIS, Android `@JavascriptInterface` bridges and the global callbacks Android evaluates (`handleInhouseNativeOAuth`, `handleInhousePdfExportResult`, `handleInhouseUpdateResult` is set by the updater). Each file is a dumb executor of encoded commands. | ≤ 2,400 lines together |
| `shared/inhouse-services-updater/updater.{js,css}` | Canonical shared component synced by `tools/sync.py`; policy forbids local edits (AGENTS.md). Loaded unmodified. | unchanged |
| `service-worker.js` | Service workers are JS entry points; the URL and classic type are fixed by 5.x registrations (§12). | ~100 lines |
| vendor `pdf.js` 3.11.174, `pdf-lib` | PDF parsing/rendering and page copying already proven; pdf.js output is part of pixel parity. Both now **lazy-loaded** (first PDF background / import / export). jsPDF is dropped (§8.4). | vendored |

Generated files (`wasm/ihn.wasm`, `offline-assets.js`, `native-assets.json`, the `#inhouse-native-assets` map and
every `native=<sha>` query in `index.html`) are produced by `npm run build` into `build/` (gitignored on the v6
branch); `tests/v6/server.mjs --overlay build/` serves them over the tree. The committed `index.html` on the v6 branch
holds the placeholder `native=__NATIVE__` and an empty map. Only the release PR copies `build/` over the tree and
commits it (§14.2), so parallel units never conflict on generated bytes.

`index.html` script section in v6 (everything else byte-identical to 5.18.0):

```html
<script type="application/x-inhouse-config" id="inhouse-app-version">const APP_VERSION = '6.0.0';</script>
<script type="application/json" id="inhouse-native-assets">{…generated by the build…}</script>
<script src="boot.js?v=6.0.0&amp;native=…"></script>
…5.18.0 <style>, offline-v5.css, ui.css, GIS client, updater.css unchanged…
<script defer src="shared/inhouse-services-updater/updater.js?v=1.2.3&amp;native=…"></script>
<script type="module" src="host/main.js?v=6.0.0&amp;native=…"></script>
```

`pdf.min.js`/`pdf-lib.min.js` keep their SRI hashes and are injected by `host/pdf.js`/the worker on first use
(`<script integrity=… crossorigin=anonymous>` for pdf.js; `importScripts` in the worker for pdf-lib). CSP is unchanged
(`'wasm-unsafe-eval'` is already allowed; module scripts are `'self'`).

### 3.2 Exports and imports (the ABI, version `IHN_ABI = 6`)

Exports (`ihn-wasm`):

| Export | Purpose |
|---|---|
| `ihn_abi() -> u32` | returns 6; loader refuses other values |
| `ihn_alloc(len) -> ptr`, `ihn_free(ptr,len)` | scratch buffers for event payloads |
| `ihn_init(ptr,len)` | config: APP_VERSION, URL, UA, mode flags (app / web / headless / e2e), viewport, DPR, `touchDrawEnabled`… |
| `ihn_event(kind:u32, ptr, len) -> u32` | one event (§3.5); return bit 0 = `preventDefault`, bit 1 = `stopPropagation` |
| `ihn_frame(t:f64)` | rAF callback (only requested when Rust asked for a frame) |
| `ihn_out_ptr() -> ptr`, `ihn_out_len() -> u32` | the out-buffer produced by the last call (§3.3); JS applies it, then calls `ihn_out_reset()` |
| `ihn_worker(ptr,len)` | export-worker entry (same module, different role) |

Imports (module `env`; the build script asserts **exactly** this list):

| Import | Why synchronous |
|---|---|
| `host_flush()` | apply the out-buffer written so far (before a measurement) |
| `host_rect(handle, out_ptr)` | `getBoundingClientRect` → 4 f64 (implies flush) |
| `host_metric(handle, which) -> f64` | `scrollTop/Left/Height`, `clientWidth/Height`, `offsetTop`, `innerWidth/Height`, `visualViewport` (implies flush) |
| `host_now() -> f64` | `performance.now()` |
| `host_time() -> f64` | `Date.now()` |
| `host_random(ptr,len)` | `crypto.getRandomValues` |
| `host_ls(op, kptr,klen, vptr,vlen) -> i32` | `localStorage`/`sessionStorage` get/set/remove (sync in 5.18.0 too; get writes into a scratch buffer, returns length or −1) |
| `host_intl(op, ptr,len) -> i32` | `toLocaleTimeString`, `toLocaleDateString`, month names, `Intl.Collator.compare`, `toLocaleLowerCase` — results identical to 5.18.0 because the browser computes them |
| `host_log(level, ptr,len)` | console |

Re-entrancy rule: a DOM op applied during `host_flush` can fire synchronous events (`focus`, `blur`, `scroll`).
`host/main.js` keeps an `inCall` flag; events that arrive while Rust is running are queued and delivered right after
the current export returns. Rust never holds a borrow across an import call.

### 3.3 Out-buffer (one ordered stream of records)

`[u8 family][u8 op][u32 len][payload]`, little-endian; strings are UTF-8 with u32 length; numbers are f64 unless
noted. Families are applied in stream order, so a DOM insert is visible to a canvas op that follows it.

| Family | Owner (Rust → JS) | Ops (complete list frozen in `abi.rs` doc comments; ranges reserved per family) |
|---|---|---|
| 1 DOM | `dom.rs` → `host/dom.js` | `bind(id)→h`, `query(h, selector)→h`, `create(tag)→h`, `insert_html(parent, before, html)→h`, `child(h, path)→h`, `insert(parent, before, h)`, `remove(h)`, `release(h)`, `attr(h,name,val|null)`, `prop(h,name,val)` (value, checked, disabled, hidden), `text(h,s)`, `class(h,name,on)`, `class_set(h,s)`, `style(h,prop,val|null)`, `listen(h,type,listener_id,flags{capture,passive,once})`, `unlisten`, `focus(h)`, `blur(h)`, `scroll_to(h,x,y,behavior)`, `animate(h, keyframes_json, options_json)→anim`, `anim_cancel/finish`, `pointer_capture(h,id,on)`, `dialog(h, show|showModal|close)`, `click(h)` (file inputs) |
| 2 Canvas | `render/cmd.rs` → `host/canvas.js` | `ctx(canvas_h, settings)→c`, `offscreen(w,h)→c`, `size(c,w,h)`, `transform/set_transform`, `save/restore`, `fill_style/stroke_style(str)`, `line_width/cap/join`, `global_alpha`, `composite(str)`, `begin/move/line/bezier/quad/arc/round_rect/close`, `stroke/fill`, `path_new(p)/path_* /stroke_path(c,p)/path_drop(p)`, `clear_rect/fill_rect`, `draw_image(c, img_h, 4|8 args)`, `radial_gradient…` (bullet dots), `get_image_data`→event, `to_data_url/to_blob(c, type, q)`→event |
| 3 Effects | various → host modules | async I/O with a u32 request id; completion arrives as an event (§3.5). Sub-families and the unit that implements the executor **completely** before anyone depends on it: 3.1 timers/rAF/idle (`host/main.js`, W1.1); 3.2 IndexedDB (`host/idb.js`, W2.4); 3.3 fetch/blob incl. resumable and keepalive uploads (`host/fetch.js`, W1.3); 3.4 pdf (`host/pdf.js`, W3.1); 3.5 peer + BroadcastChannel (`host/peer.js`, W2.8 executor, used by W4.5); 3.6 platform: GIS, Android bridges incl. export bridge v2, file chooser, clipboard, share, download, fullscreen, history (`host/platform.js`, W1.3); 3.7 worker messages (`host/main.js`, W1.1; worker side W3.4/W5.4); 3.8 crypto: `digest('SHA-256')` only (`host/fetch.js`, W1.3; used by PKCE, assets, relay digests); 3.9 Web Locks: request (exclusive, `ifAvailable`), release, query (`host/idb.js`, W2.4); 3.10 deflate/inflate (raw and zlib) via Compression/DecompressionStream (`host/fetch.js`, W1.3) |

Numbers that end up in DOM strings (inline styles, attributes, `aria-valuenow`) are formatted in Rust with `jsfmt`
(ES `Number::toString`), so `outerHTML` matches 5.18.0. Colours and composite modes are passed as the exact strings
5.18.0 uses.

### 3.4 `trait Host` (Rust side of the boundary)

`ihn-app` never calls `extern` functions directly; it calls `&mut dyn Host`, implemented by `ihn-wasm` (imports +
out-buffer) and by `FakeHost` in cargo tests (records the out-buffer, scripted measurements, manual clock, scripted
effect completions). This is how `cargo test` covers UI state machines without a browser.

```rust
pub trait Host {
    fn out(&mut self) -> &mut OutBuf;                  // DOM / canvas / effect records
    fn flush(&mut self);
    fn rect(&mut self, h: Node) -> Rect;
    fn metric(&mut self, h: Node, m: Metric) -> f64;
    fn now(&self) -> f64;  fn time(&self) -> f64;
    fn random(&mut self, buf: &mut [u8]);
    fn ls(&mut self, op: LsOp, key: &str, val: Option<&str>) -> Option<String>;
    fn intl(&mut self, op: IntlOp, input: &str) -> String;
}
```

### 3.5 Events (JS → Rust)

| Kind | Payload |
|---|---|
| `DOM` | listener id, handle, `type` code, `key/code/ctrl/shift/alt/meta/isComposing`, `button/buttons`, client x/y, `target` handle if known (`data-ihn` lookup via a `WeakMap<Node,handle>`), input `value`/`checked`, `deltaX/Y/mode`, file list handle |
| `POINTER` | batch: per sample `{type, pointerId, pointerType, buttons, x, y (client), pressure, tiltX, tiltY, t}`; for `pointermove`, the coalesced samples **plus** the raw event as v5 does (`getCoalescedEvents()`, empty list → raw sample) |
| `FRAME` | `t` |
| `TIMER` / `IDLE` | timer id / deadline |
| `IO` | family, request id, status, payload (bytes in wasm memory, or a JS handle for Blobs/ImageBitmaps/PDF documents) |
| `LIFECYCLE` | `visibilitychange`, `pagehide`, `beforeunload` (return bit 0 ⇒ set `returnValue=''`), `online/offline`, `resize`, `fullscreenchange`, `prefers-color-scheme` change, window `blur` |
| `MESSAGE` | Android callbacks (`handleInhouseNativeOAuth(payload)`, `handleInhousePdfExportResult(r)`), worker messages, `paste` data, peer/sync messages (SYNC.md) |

### 3.6 How Rust drives the existing DOM

- **Static markup** (everything in 5.18.0 `index.html`) is bound once at init: `dom::bind("drive-home")` returns a
  handle; the `Ui` struct holds typed handles for every `id` in `index.html`. W1.1 generates `ui_ids.rs` once from
  `index.html` with `scripts/gen-ui-ids.mjs` (committed; regenerated only if the markup changes, which it does not), so
  no later unit edits the table.
- **Dynamic markup** is produced from Rust string templates that reproduce 5.18.0's builders exactly
  (`createDriveFileCard`, `createDriveFolderCard`, `createDriveFolderArt`, page rows, `renderPagesList`, timeline list
  rows, share people rows, presence avatars, selection box, eraser menu state, offline settings dialog, Soon toast,
  …). A subtree is inserted with one `insert_html`; descendants that Rust must update later are addressed with
  `child(h, path)`. Lists use keyed diffing in Rust (the `diffRenderDriveList` algorithm), so unchanged cards keep
  their node, image and animation state.
- **Events**: `listen(h, type, id, flags)` mirrors 5.18.0's `addEventListener` calls one-to-one, including
  `{passive:false}` on the wheel/touch listeners and capture-phase document listeners. `ihn_event` returns
  `preventDefault`/`stopPropagation` synchronously.
- **Animations**: CSS transitions/keyframes come from the copied stylesheet. The few WAAPI animations (folder
  zoom, label fade) use `animate(h, keyframes, options)` with keyframes computed by `motion.rs` exactly as 5.18.0.
- **Layout reads** go through `host_rect/host_metric` at the same moments 5.18.0 reads layout (once per gesture,
  menu positioning after display) — never per pointer sample.

### 3.7 Platform contracts kept exactly (Android APK loads the live site)

| Contract | v6 owner | Notes |
|---|---|---|
| App mode: `?inhouse_app=1|true`, `?app=`, UA `InhouseNotesApp/<n>`, `localStorage.inhouseNotesAppMode`; `?inhouse_app=0` clears | `host/platform.js` computes, Rust receives in `ihn_init` | `html[data-inhouse-app]`, `window.INHOUSE_NOTES_APP`, removal of `#btn-drive-fullscreen`/`#btn-fullscreen` |
| `InhouseNative.openAuthUrl(url)` (PKCE in Custom Tab) and `window.handleInhouseNativeOAuth(payload)` | `auth.rs` + `host/platform.js` | PKCE: verifier = base64url(32 random bytes) (Rust), challenge via `crypto.subtle.digest` effect; token POST to `oauth2.googleapis.com/token` with the Android client id, no secret; scope `https://www.googleapis.com/auth/drive` (5.18.0) |
| PDF export bridge v2: `getPdfExportApiVersion()`, `beginPdfExport(name)`, `appendPdfExportChunk(b64 ≤240000)`, `finishPdfExport()`, await `handleInhousePdfExportResult` ≤30 s; legacy `savePdf(name,b64)`; app `navigator.share`; web `<a download>` | `export/user.rs` + `host/platform.js` | 5.18.0 removed bridge v1; same here |
| Updater: `InhouseServicesUpdater.create({appId:'notes', appName:'Inhouse Notes', repository:'miguelcoxcaballero/inhousenotes', installedVersion:'1.0.2', install})` with `installInhouseUpdate(url, sha256)` → `installAppUpdate(url)` → throw `'Abre la app Android para instalar.'`; `window.InhouseUpdater = updates; updates.start()` | `host/platform.js` (verbatim from 5.18.0 `app-v5.js`) | only in app mode; the updater reads `getAppVersion`, `getUpdateState`, etc. itself |
| Background: `InhouseBackgroundChannel` → `InhouseBackground.{schedule,cancel,complete(json)}`; headless page `?inhouse_app=1&offline-worker=1` must call `complete({busy,pending,errors})` | `boot.js` (unchanged facade) + `export/headless.rs` | WorkManager succeeds iff `!busy && pending==0 && errors.length==0` |
| Native assets: `?native=<sha256>` on every same-origin asset URL; `#inhouse-native-assets` map; `InhouseAssetUrl(path)` | `scripts/build-native-assets.mjs`, `boot.js` | new files (wasm, host/*.js) simply join the manifest; old APKs fetch them from the network |
| Version label: `[data-app-version]` in both profile menus shows `v6.0.0` from the inert `APP_VERSION` block | `shell` | missing block → refuse to start with the 5.18.0 error text |
| Release watcher (`/service-worker.js?release-check=` → `const VERSION='x.y.z'`) and SW registration of `/service-worker.js` | `boot.js` unchanged; v6's worker **is** `/service-worker.js` (classic, same scope) with the `const VERSION='6.0.0'` line | a still-open 5.x tab reloads into 6.0.0 at its next Home/visibility check, and the browser's update check of the 5.x registration installs the v6 worker (§10.4, §12) |

---

## 4. Document model and edit operations

This section is the **single normative text** for ids, actors, stamps, version vectors, ops, the editing API, the
`Replica`/`SyncEngine` interface and the persistence hooks. SYNC.md §2–§3 and §18.2 summarise it and refer here.

### 4.1 State

```rust
pub struct Doc {
    pub name: Reg<String>,                       // doc:exportName
    pub pages: HashMap<Id, Page>,                // includes deleted pages (tombstoned)
    pub items: HashMap<Id, Item>,                // strokes and images of all pages
    pub versions: BTreeMap<Id, VersionMeta>,     // timeline (grow-only; prune = tombstone register)
    pub vv: VersionVector,                       // contiguous seq per actor + extras (below)
    // derived, not serialized: page order cache, per-page draw lists (§6.2), spatial grid, dirty set
}
pub struct Page {
    pub pos: Reg<Pos>, pub deleted: Reg<bool>,
    pub template: Reg<Template>,                 // Default | Agenda | Diary (agenda/diary only render their SVG)
    pub size: Reg<(u32,u32)>,                    // integers ≥ 64
    pub background: Reg<Background>,             // Template | Pdf{asset: Hash, page: u32 (1-based in that asset)} | Custom{asset: Hash}
    pub side_panel: Reg<Option<Bytes>>,          // kept opaque, never shown (REQUIREMENTS §1b)
}
pub struct Item { pub page: Reg<Id>, pub z: Reg<Z>, pub body: Reg<Body>, pub deleted: Reg<bool> }
pub enum Body {
    Stroke { tool: Tool /*Pen|Highlighter|LegacyEraseArea|LegacyCover*/, color: Color, width: f64,
             xy: Vec<f64> /*x0,y0,x1,y1… lossless f64*/, p: Option<Vec<f32>> /*stored, never rendered*/ },
    Image  { asset: Hash, cx: f64, cy: f64, w: f64, h: f64, rotation: f64 },
}
pub struct Stamp { pub clock: u64, pub actor: ActorId, pub idx: u32 }   // total order (clock, actor, idx)
pub struct Reg<T> { pub v: T, pub at: Stamp }    // last-writer-wins by Stamp
pub struct Z { pub at: Stamp, pub sub: u32 }     // draw order within a kind: (Z, item Id)
pub type Hash = [u8; 32];                         // SHA-256 of an asset (§4.5)
```

- **Ids.** `Id(u128)`: new ids are `(actor << 64) | counter`; 5.x string ids map to `fnv1a128(string)`, so every
  device that imports the same 5.x data derives the same ids (§9.3). Ids never reach the DOM except page and version
  rows, where TESTING §3.2 normalises them (BEHAVIOUR-CHANGES N-11).
- **Actors.** `inst` = random u64 with the low 8 bits zero, one per (browser profile, account), `kv[account,'inst']`.
  Each tab holds a slot 0..255 through Web Lock `ihn6-slot:<account>:<slot>` and uses `ActorId = inst | slot`, so two
  tabs never share an id counter or `seq` (SYNC §7.1). Import actors: `IMPORT_BIT | (fnv1a64(source) >> 1)`,
  `IMPORT_BIT = 1 << 63`, source = a Drive head revision id (§9.3) or `"migrate:" ‖ inst13 ‖ ":" ‖ docId` (§10).
  Id counters: `kv[account,'ctr:'+actor]`, reserved in blocks of 65,536.
- **Stamps (HLC).** All ops of a native batch share `batch.clock`; op *i* has `Stamp{batch.clock, actor, i}`. The next
  batch clock = `max(prev_batch_clock + 1, wall_ms, observed + 1)`. Applying a batch observes its greatest clock,
  except clocks greater than `wall_ms + 24 h`, which are applied but never observed. Import and migration batches
  carry an explicit stamp per op (codec: SYNC §4.2).
- **Version vector.** `vv[actor]` = highest contiguous applied seq; batches above a gap are applied at once
  (commutativity) and kept in a per-actor extras set until the gap fills; dedupe checks both.
- **Z order.** `z` is an LWW register. A new item gets `Z{at: its op stamp, sub: 0}`. A cross-page move writes a new
  `Z` = the move's stamp, so moved items draw on top of the target page, as 5.18.0's `moveSelectionToPage` appends
  them (app-v5.js:11966). Area-erase fragments get `Z{at: original z.at, sub: original sub·2^16 + k}` (original
  position, B-E4). Transforms and recolours within a page keep `z`.
- Colours are stored as the exact 5.x string when not `#rrggbb` (`Color::Hex(u32) | Color::Css(Box<str>)`).
- Page order = live pages sorted by `(pos, id)`; `Pos` is a variable-length fractional key. If every page is deleted,
  `order()` shows the page with the greatest `deleted` stamp (projection only); the first local edit on it emits
  `PageDel{page, false}`.

### 4.2 Operations

```rust
pub enum Op {
    PageNew  { page: Id, pos: Pos, template: Template, size: (u32,u32), background: Background },
    PageMove { page: Id, pos: Pos },
    PageDel  { page: Id, deleted: bool },             // false = restore (undo / timeline restore)
    PageSet  { page: Id, prop: PageProp },            // Template | Size | Background | SidePanel
    ItemPut  { item: Id, page: Id, z: Z, body: Body },// create, or write page+z+body (transform, recolour, move)
    ItemDel  { item: Id, deleted: bool },             // erase / undo of erase
    DocName  { name: String },
    VersionAdd { meta: VersionMeta },                  // timeline entry; its snapshot is an asset (§4.6)
    VersionDel { version: Id },                       // pruning
}
pub struct Batch { pub actor: ActorId, pub seq: u32, pub clock: u64, pub ops: Vec<Op>,
                   pub stamps: Option<Vec<Stamp>> }    // Some(..) iff actor & IMPORT_BIT
```

Semantics: every field is an LWW register; `apply` writes a register only if the incoming stamp is greater. An
`ItemPut` for an unknown item creates it; for a known item it writes the `page`, `z` and `body` registers. Ops
referencing unknown pages are stored and become visible when the page arrives. Therefore **apply is idempotent and
commutative**: any delivery order, duplicates and gaps converge (property-tested, TESTING §1). A `Batch` is applied
atomically (one user action = one batch = one undo step = one sync unit). Deletions are tombstones forever (no GC in
6.0); a snapshot may drop the body of a deleted item.

### 4.3 The editing API the UI uses (`doc.rs`)

```rust
impl Doc {
    pub fn apply(&mut self, b: &Batch) -> Changes;                 // local or remote; Changes = touched pages/items/props
    pub fn order(&self) -> &[Id];                                  // live pages (all-deleted projection, §4.1)
    pub fn draw_list(&self, page: Id) -> DrawList<'_>;             // covers, images, strokes — each in (Z, Id) order (§6.2)
    pub fn hit_strokes(&self, page: Id, x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> SmallVec<Id>; // swept capsule
    pub fn lasso(&self, page: Id, poly: &[f64]) -> Vec<Id>;
    pub fn inverse(&self, b: &Batch) -> Vec<Op>;                   // against the pre-state, before apply
}
pub struct Tx<'a> { doc: &'a Doc, ops: Vec<Op> }                   // builder used by tools
impl Tx<'_> {
    pub fn add_stroke(&mut self, page: Id, body: Body) -> Id;
    pub fn erase(&mut self, items: &[Id]);
    pub fn split(&mut self, item: Id, fragments: Vec<Vec<f64>>);  // area eraser: del + puts at the original z (§4.1)
    pub fn transform(&mut self, items: &[Id], m: Affine);         // selection move/scale/rotate (z kept)
    pub fn recolor(&mut self, items: &[Id], c: Color);
    pub fn move_to_page(&mut self, items: &[Id], page: Id, dy: f64); // new z = top of the target page
    pub fn duplicate(&mut self, items: &[Id], dx: f64, dy: f64) -> Vec<Id>;
    pub fn add_image(&mut self, page: Id, asset: Hash, rect: ImageRect) -> Id;
    pub fn page_new(&mut self, after: Option<Id>, tpl: Template, size: (u32,u32), bg: Background) -> Id;
    pub fn page_move(&mut self, page: Id, before: Option<Id>);
    pub fn page_delete(&mut self, page: Id);
    pub fn set_name(&mut self, name: &str);
    pub fn commit(self) -> Batch;                                  // clock + stamps assigned here
}
```

Undo/redo (`editor/history.rs`) stores, per step, the **inverse ops** computed at commit time and the forward ops.
Undo applies the inverse as a *new* batch (new stamps), exactly like 5.18.0's "re-stamp" semantics. Inverses (SYNC §5
has the table): creation → `ItemDel{true}`; replace → `ItemPut{old page, old z, old body}`; deletion →
`ItemPut{item, page, z, body}` **then** `ItemDel{false}` (the body travels, so a replica that dropped it can revive
it); `PageSet`/`DocName` → old value; page ops are not undoable (5.18.0). Limits are 5.18.0's: 10 steps, 25,000
points, halved when hidden. Clearing rules (open, import, restore, photo page, page delete, remote structural change)
are kept.

### 4.4 The session and the core↔sync interface

`doc/session.rs` owns one open document: `Doc`, the local `ActorId`, the undo stack, the persistence queue and the
sync binding. The whole document (all chunks) is loaded into memory on open; only rasters (PDF backgrounds, images,
previews) are lazy (§6.3). `ihn-sync` sees the document only through:

```rust
/// Implemented by ihn-app::doc::session; called by ihn-sync.
pub trait Replica {
    fn doc_id(&self) -> &DocId;                                    // always the Drive file id (§7.1)
    fn version_vector(&self) -> &VersionVector;                    // contiguous + extras
    fn batches_since(&self, have: &VersionVector, max_bytes: usize)
        -> Result<Vec<EncodedBatch>, Compacted>;                   // Err only when the range left the journal (§7.1)
    fn receive(&mut self, b: &[EncodedBatch], from: PeerRef) -> ReceiveResult;  // dedupe, apply (budgeted), persist
    fn snapshot(&self) -> EncodedSnapshot;
    fn load_snapshot(&mut self, s: &EncodedSnapshot, from: PeerRef) -> ReceiveResult; // merge, never replace
    fn missing_assets(&self) -> Vec<Hash>;
    fn put_asset(&mut self, h: Hash, bytes: BlobHandle);
    fn asset(&self, h: &Hash) -> Option<BlobHandle>;               // serve AssetWant / relay asset uploads
    fn can_edit(&self) -> bool;                                    // Drive capability; viewers never send ops
    fn retain(&mut self, covered: &VersionVector);                 // journal entries covered by peers/tabs/seal/export
}
pub struct ReceiveResult { pub applied: u32, pub duplicate: u32, pub queued: u32, pub changed_pages: Vec<Id> }

/// Implemented by ihn-sync::Sync; called by the session.
pub trait SyncEngine {
    fn local_batch(&mut self, b: &EncodedBatch);                   // after the IDB commit (degraded mode: below)
    fn live(&mut self, p: LivePreview);                            // Begin/Points/End{item}/Cancel/Erase/EraseEnd
    fn presence(&mut self, p: PresenceState);                      // editing/viewing, page, tool colour, save phase
    fn tick(&mut self, now: f64, r: &mut dyn Replica) -> Vec<SyncEffect>;
    fn on_io(&mut self, ev: SyncIo, r: &mut dyn Replica) -> Vec<SyncEffect>;
    fn status(&self) -> SyncStatus;                                // §6.6
    fn peers(&self) -> &[PeerView];                                // presence avatars (SYNC §9.1)
}
pub struct SyncStatus {
    pub unsealed_local: u32,             // own batches not yet sealed in the relay
    pub relay_durable_through: VersionVector,
    pub relay_writing: bool, pub not_created: bool,
    pub export: ExportPhase,             // Idle | Building | Checking | Merging | Uploading | Preserving | Failed{code, detail}
    pub export_pending: bool,
    pub online: bool, pub auth_paused: bool, pub last_error: Option<SyncError>,
    pub peers_connected: u32,
}
```

`SyncEffect`/`SyncIo` are listed in SYNC §18.2.

Persistence hooks (session → store, §7):
- **Write path.** A local batch is appended to `ops` (with `docs.seq[actor]`) in one transaction; only after commit is
  `SyncEngine::local_batch` called. Received batches are applied and appended in one transaction.
- **Compaction and journal retention.** When the pen is idle (180 ms window, 1200 ms within a burst — 5.18.0's
  gating), journal entries are folded into page chunks, but entries stay in `ops` until `retain()` reports them
  covered by the relay seal (own batches), by every known peer/tab VV and by the latest known export, or until they
  are 24 h old, within 8 MiB per document. So reconnects and new tabs get batches, not snapshots.
- **Degraded storage** (IndexedDB unavailable: open blocked 1.5 s / timeout 5 s, or quota errors after 3 retries):
  batches go to an in-memory journal and `local_batch` is called at once; the relay seal is the durability point; the
  UI keeps 5.18.0's local-save error state until `unsealed_local == 0`; `pagehide` triggers the keepalive relay
  flush; Home/exit waits for the seal (BEHAVIOUR-CHANGES N-6).
- **Remote apply budget.** Remote batches are decoded and applied at once while no pointer is in contact. While a
  pointer is in contact, `receive` applies at most ~0.5 ms of work per frame (measured with `host_now`) and queues
  the rest (`ReceiveResult.queued`); the queue drains in idle slices of the `document_work.rs` scheduler. A remote
  batch touching the page being drawn on is repainted at pen-up; other pages repaint at once.

### 4.5 Assets

Image bytes (inline images, photo pages) and source PDFs are content-addressed: `Hash` = SHA-256 of the bytes
(effect family 3.8). They live in the `assets` store and travel through sync (`missing_assets`/`put_asset`/`asset`,
P2P pulls, relay asset files) and, for images, inside exported PDFs (§8.2). Image compression rules are 5.18.0's
(`compressImageForDocument`: ≤3200 px, ≤8 MP, PNG ≤4 MiB kept, else JPEG 0.94 on white; photo pages JPEG 0.94
page-sized composite).

Source PDFs are **not** attached to exports; the exported pages themselves carry them (§8.2). Two rules keep hashes
deterministic across devices:
- A source asset recovered from a Drive file (a plain PDF adopted on first open, or the clean original of a 5.x
  document) is keyed `Hash = SHA-256("ihn6-src" ‖ SHA-256(downloaded revision bytes) ‖ u32 variant)`, independent of the
  bytes this device derives from it.
- A device missing a source asset rebuilds it from the latest export: the export worker copies the export's pages that
  carry that source, strips the `/IHN6Ink` content stream (§8.2), and stores the result in `assets` under the same
  `Hash` with a page remap (`assets.remap: [source page → page in this blob]`). Rendering through pdf.js is identical
  because the page content objects are copied unchanged.

### 4.6 Timeline (version history)

Versions keep 5.18.0's semantics (coalescing for the same author/device within 5 min, milestones
`baseline|restore|revert-original|pre-restore|manual`, summary strings, pruning to 80 entries / 30 milestones,
`Before restoring <date>` pre-restore milestone, restore as a new causal write).
- **Who captures.** Each editor captures its own versions (author and device = itself) at the moments 5.18.0 saved:
  after its own edits once its input has been idle for the 5.18.0 save debounce, and on exit/Home. Capture is
  independent of exports and of the scribe. Trigger: `doc/versions.rs` (W5.3), called by the session's idle and
  lifecycle hooks; rules in `ihn-core::history` (W2.9).
- **Storage.** `VersionAdd{meta}` carries id, ts, author, kind, summary, device and the version-snapshot asset hash.
  A version snapshot = per page `(page id, props, live item ids)` plus bodies of items not present in the author's
  previous version (dedupe by `(id, body hash)`). Delta chains stay **within one actor**, with a self-contained
  checkpoint every 8 versions, so concurrent capture on two devices never shares a base. A pruned version's asset is
  kept while a later version of the same chain needs it.
- **Transport.** Version assets travel like other assets; the export attaches the newest ones within 5.18.0's 3.5 MB
  budget (`IH_VERSIONS` budget), the rest stay as relay assets (SYNC §14.6).
- Restore = `history::plan_restore(doc, target)` → ops that delete current items absent from the target,
  undelete/put target items (with their target `z`) and reset page/doc registers. Imported 5.x `IH_VERSIONS` archives
  become versions on first open (§9). Diff highlights reuse the timeline-diff algorithm.

---

## 5. Sync placement (summary; SYNC.md is authoritative)

- `ihn-sync` is pure and owns protocol state; `host/peer.js` owns sockets and the BroadcastChannel;
  `ihn-app::drive` executes the Drive requests the sync engine asks for.
- Viewers (`can_edit == false`) receive but never send ops; peers must be vouched by editor-only seals
  (fixes the 5.x security finding, BEHAVIOUR-CHANGES B-C3).
- Two tabs of one browser are two replicas; BroadcastChannel is just another transport.
- No v5 interop: v6 does not read or write `ink-journal-v1`, `ink-preview-v1`, presence properties or P2P comments
  of 5.x (a 5.x device's unsynced work reaches v6 through its own migration, §10).

---

## 6. Rendering

### 6.1 Surfaces (identical DOM to 5.18.0)

Per active page (max 3): `.page-wrapper` (CSS preview/template background) → `canvas.page-canvas` (CSS background
+ bitmap) → `canvas.lasso-overlay` → `canvas.cursor-overlay` → `canvas.remote-live-overlay` → `.selection-box`,
`.page-number`. Backing-store sizes, overlay scales, `dataset.renderScale/overlayScale`, hidden-empty-overlay
toggling and the 90 ms resolution resync are computed in `editor/pages.rs` with 5.18.0's formulas
(`getTargetPageCanvasScale`, device type detection from UA). Contexts are plain `getContext('2d')`
(`desynchronized` stays false).

### 6.2 Canvas command generation (`ihn-core::render`)

Rust computes geometry; JS replays. For each stroke the pen path (centripetal Catmull–Rom, straight below 2 px,
`moveTo` + `bezierCurveTo`/`lineTo`) is emitted once into a `Path2D` handle cached by `(item id, body stamp)`;
repaints emit `stroke_path`. Highlighter, single-point dot, legacy cover and legacy erase-area rules, image draws,
template draws, live ink (definitive segments to the page bitmap each frame, newest segment on the cursor overlay,
remaining segments at lift), eraser cursor, damage-region repaint, staged repaint (>128 strokes or >2048-point
strokes: offscreen canvas, ≤4 ms slices, atomic blit, generation guard) and remote live overlay are ported from the
5.18.0 functions named in `spec/editor.md` §6 with identical call sequences. Numbers are the same IEEE f64 maths
(+, −, ×, ÷, `sqrt` are correctly rounded in both); every other `Math.*` goes through `jsmath` (§6.5).

Draw order per page is 5.18.0's `redrawPage` order (app-v5.js:13008-13019): legacy cover strokes, then all images,
then all other strokes; inside each kind, items are sorted by `(Z, Id)`. `Doc::draw_list` returns these three runs;
lasso selection, hit testing and thumbnails/exports use the same order.

Bullet template: ported from `bullet-template.js` into `render/template.rs` (same `roundRect`, stamped radial
gradient dots, footer boxes, circles). Agenda/diary templates: the exact SVG bytes are shipped as
`icons/agenda.svg`, `icons/diary.svg` (new files, bytes = 5.18.0 `_AGENDA_SVG_B64`/`_DIARY_SVG_B64` decoded) and drawn
with `drawImage(svgImage, 0, 0, w, h)` after a white fill, with the 5.18.0 cache-flush-on-load behaviour. Template
CSS background JPEGs (scale policy 3.6/4.8 PC, 3.0/4.2 touch, q 0.86) and template canvas caches (6 entries) are
kept.

### 6.3 Images, PDF backgrounds, thumbnails

- Images: `host/canvas.js` decodes assets into `ImageBitmap`-free `HTMLImageElement`s (as 5.18.0, so
  `drawImage` resampling is identical); not-yet-decoded images trigger the `{assetsReady}` redraw.
- PDF backgrounds: `doc/backgrounds.rs` decides *which* page and *what* scale (5.18.0 `getPdfBackgroundScale`, LRU 6
  non-active, interaction deferral 240 ms, one page per frame); `host/pdf.js` opens the source asset with pdf.js
  3.11.174 (hardened params from `security-core-v5.js` ported to the adapter), renders with intent `display` on
  white, encodes JPEG 0.92 to a blob URL, and returns the handle; Rust sets the CSS background exactly as 5.18.0.
- Thumbnails: page previews (`renderPagePreview` 0.18 scale, JPEG 0.6), scroll previews (≤320×440, JPEG 0.65, ≤3 ms
  stroke slices), pages-panel thumbnails and timeline previews are command streams to CPU canvases
  (`willReadFrequently`) followed by `to_data_url`, as 5.18.0.

### 6.4 Pixel-parity rule

A unit is done only when its parity states pass the screenshot gate at tolerance 0 against 5.18.0 (TESTING §3),
except entries listed in `BEHAVIOUR-CHANGES.md`. The DOM and canvas-trace comparisons are diagnostics attached to a
failing screenshot (they locate the cause); they are not independent gates. W2.3 additionally passes cargo
trace-golden tests against 5.18.0 traces recorded by W1.2 (TESTING §1).

### 6.5 Exact maths

V8 computes `Math.sin/cos/atan2/exp/pow` with its fdlibm-derived `ieee754` code; Rust's wasm `f64` methods use musl
`libm`, which differs by 1 ULP on some inputs (and `pow` is a different algorithm). 5.18.0 uses these in image and
selection rotation (app-v5.js:4803, 12006, 12131, 12264, 13600, 29838), wheel/pinch zoom (7760, 14283), pull-to-refresh
(10210-10211), inertia (15875) and luminance (9258). `ihn-core::jsmath` ports V8's `src/base/ieee754.cc` versions of
these five functions plus V8's `Math.round` and `hypot`; cargo tests compare them with values recorded from Node 22
over a grid of inputs and the edge cases (±0, subnormals, ±π multiples, overflow). `scripts/build-wasm.mjs` fails if
`ihn-core` or `ihn-app` sources outside `jsmath.rs` call `.sin(`, `.cos(`, `.tan(`, `.atan2(`, `.exp(`, `.ln(`,
`.log`, `.powf(`, `.powi(` or `.hypot(`.

### 6.6 Status mapping (save indicator, Home badges, offline dialogs)

`doc/save_state.rs` derives one presentation from `SyncStatus` (§4.4) plus the local commit state, with 5.18.0's
precedence (`getSavePresentation`, app-v5.js:31374-31465). "Unsynced" always means **own batches not sealed in the
relay** (SYNC §6 invariant 3), never "export not written".

| Condition (first match wins) | Save indicator class (`#save-indicator`, W3.1) | Home card badge `button.document-sync-status` text and `data-phase` (W2.5) |
|---|---|---|
| local commit failed / degraded storage with `unsealed_local > 0` / `last_error` and nothing running | `error` | `Retry upload` (`error`) |
| a local batch is not yet committed | `saving` | — (badge only from the persisted flags below) |
| export `Building` on this device | `building` | `Preparing…` (`building`) |
| export `Checking` / `Merging` / `Preserving` | `uploading` | `Checking…` / `Merging…` / `Keeping both versions…` |
| relay write in flight with own batches, document create in flight, or export `Uploading` | `uploading` | `Uploading…` (`uploading`) |
| catch-up reads after open/reconnect (SYNC §8.8) | `syncing` | `Syncing…` (`syncing`) |
| `unsealed_local > 0` or `not_created`, and offline or `auth_paused` | `paused` | `Saved offline` (offline) / `Waiting to upload` |
| `unsealed_local > 0` or `not_created` (waiting for the flush) | `paused` | `Waiting to upload` |
| otherwise (everything sealed; an export may still be pending) | `saved` | no badge |

- Home reads the persisted flags of documents that are not open: badge present iff `docs.flags.relay_pending` or
  `!docs.flags.created` or the document is the active job. `data-offline-available` = the document has chunks;
  `data-offline-pinned` = `docs.pinned`.
- Offline documents dialog (W5.5): queue line `N pending · Waiting for connection|Will sync automatically` with N =
  documents with `relay_pending` or not created; `Up to date` when N = 0.
- `#offline-sync-error` dialog (W5.5): same markup and report format `Inhouse Notes <version>\n<phase> · <code>\n<detail>`;
  `<phase>` is one of 5.18.0's phase names, plus `relay` for relay-write failures (BEHAVIOUR-CHANGES N-2).
- Save-details popup (W3.1): 5.18.0's markup and line order; lines that described PDF build/upload phases now show
  the export phase, and the "Strokes saved" line means "sealed in the relay". W3.1 lists every changed string in
  BEHAVIOUR-CHANGES N-2 before merging.
- Remote save rings on presence avatars use the same class names from the peer's `save_phase` (SYNC §9.1).

---

## 7. Storage layout

### 7.1 IndexedDB `inhouse-notes-v6` (version 1; schema frozen by W2.4)

| Store | Key | Value |
|---|---|---|
| `docs` | `docId` — always the Drive file id (below) | `{account, name, drive_name{v, observed_at: Stamp}, driveMeta{resourceKey, parents, canEdit, ownedByMe, owner…}, vv, seq{actor→u32}, relay{file_id (current epoch), epoch_day, gen, archives[{file_id, digest, ranges}], assets[{hash, file_id, size}], sealed{actor→seq}, perm (digest of permissionIds), applied{inst→(gen, d16)}}, exports{headRevisionId→vv} (8 newest), last_export{headRevisionId, vv, etag, at}, flags{created, relay_pending, export_pending, migrated}, lastOpenedAt, pinned}` |
| `chunks` | `[docId, chunkId]` | binary: chunk 0 = doc header (pages, registers, versions index); chunk per page = page items (columnar f64) |
| `ops` | `[docId, actor, seq]` | encoded `Batch` (journal, retention §4.4) |
| `assets` | `hash` | `{blob, mime, size, refs, remap?}` (§4.5) |
| `catalog` | `[account, fileId]` | Drive entry cache for Home (fields 5.18.0 caches), `localPreview` |
| `kv` | `[account, key]` | `inst`, `ctr:<actor>`, `idpool`, `relay-folder`, `relay-nolink`, `relay-gc`, `preferences{retentionDays}`, `home-views`, `drive-root`, `migration`, `recoveries` |

Mapping of `SyncEffect::Persist(SyncRecord)` (SYNC §18.2) to stores, so W2.4 and W2.7 agree:

| `SyncRecord` | Store / field | Transaction |
|---|---|---|
| `Seq{actor, seq}` | `docs.seq` | with the `ops` append |
| `RelayPlan{gen, file_id, epoch_day}` | `docs.relay.gen/file_id/epoch_day` | before the upload (SYNC §8.5 step 4) |
| `RelaySealed{sealed, archives, assets, perm}` | `docs.relay.*` + `flags.relay_pending` | after both PATCHes succeed |
| `RelayApplied{inst, gen, d16}` | `docs.relay.applied` | with the received batches' append |
| `KnownExport{head, vv}` / `LastExport{…}` | `docs.exports` / `docs.last_export` | own |
| `Flags{relay_pending?, export_pending?, created?}` | `docs.flags` | own |
| `DriveName{v, observed_at}` | `docs.drive_name` | own |
| `IdPool(ids)` / `RelayFolder(id)` / `NoLink(bool)` / `RelayGc(ids)` | `kv` keys `idpool` / `relay-folder` / `relay-nolink` / `relay-gc` | own |

Web Locks (complete list, executed by `host/idb.js`):

| Name | Holder | Purpose |
|---|---|---|
| `ihn6-slot:<account>:<n>` | each tab, lifetime | actor slot (§4.1) |
| `ihn6-net:<account>:<docId>` | one tab or the Android headless page per document | polling, relay writes, peers (SYNC §11) **and compaction** of that document's chunks |
| `ihn6-migrate:<account>` | the migrating tab | one-time migration (§10) |

Rules:
- **Document ids.** A new document (FAB create, Import PDF to a new document, a migrated 5.x `local-` document)
  takes an id from the pool (SYNC §8.2) at creation, so its key, `ops`, chunks, locks and BroadcastChannel name never
  change. `flags.created=false` until `export/upload.rs` (W5.4) creates the Drive file with that id (multipart POST,
  the first export); the sync session stays `NotCreated` (no relay) until then (SYNC §16.1). Offline with an empty
  pool (more than ~10 creations since the last connection), creation fails with 5.18.0's `Failed to create PDF`
  status (BEHAVIOUR-CHANGES N-12). There are no `local:` ids and no re-keying.
- One transaction per batch append (ops + seq) and per compaction (chunks + journal trimming).
- Opening loads all chunks of the document (§4.4); rasters are lazy.
- `onblocked` 1.5 s / open timeout 5 s / repeated quota errors → degraded mode (§4.4).
- Retention pruning keeps 5.18.0's rules (unpinned, nothing unsealed, not open, older than `retentionDays`, default 14).

### 7.2 localStorage / sessionStorage (kept keys, same formats as 5.18.0)

Kept because they are the live session, not legacy: `inhouse-theme`, `inhouseNotesAppMode`, `drive-token-v1`,
`drive-remember-v1`, `drive-refresh-token-v1`, `drive-pkce-verifier-v1`, `google-scope-version-v1`,
`google-granted-scopes-v1`, `drive-consent-granted`, `drive-hint-email`, `drive-profile-v1`, `touchDrawEnabled`,
`notebook-view-v1`, updater keys (`inhouse-services-updater:notes*`), session `ihn-release-reload`, `ihn-home-tab`,
`ihn-network-offline`. `boot.js` reads them unchanged, so a signed-in 5.18.0 user is still signed in after the
upgrade; the Android headless page (same origin, has localStorage) uses the same tokens. Read by migration then
removed: `drive-session-v1`, `drive-pending-exit-upload-v1`, `notebook-data-v3`, `notebook-view-mode-v1`,
`presence-client-id-v1`, `ihn-live-*`.

### 7.3 Cache Storage

`inhouse-shell-6.0.0` (service-worker.js). `inhouse-shell-5.18.0` is kept until 6.0.1 for still-open 5.x tabs
(§10.4). Home thumbnails and avatars are cached as blobs in `assets`/`catalog` (the 5.x `inhouse-media-*` caches are
deleted with the 5.x databases, §10.2).

---

## 8. PDF export to Drive

### 8.1 When

Who writes the export and when is defined **only** in SYNC §14 (scribe after 10 s idle, ≤ 1/60 s; a device leaving
alone; forced export on user Export and in the Android headless page; never during pen contact; `pagehide` never
exports). Exports are optional for correctness: ops are durable through the relay.

### 8.2 Container (v6 writes, v6 reads)

```
Keywords = "STROKES_Z:!;IHN6:" base64( zlib( snapshot bytes ) )
attachments: "IHN6-<hex hash>" for referenced image assets (inline images, photo pages) and the newest
             version-snapshot assets within 3.5 MB (§4.6); never source PDFs
Title 'Cuaderno Digital', Subject 'Notas con S-Pen', Creator 'Cuaderno Digital App' (unchanged)
appProperties: { ihn6: "1", ihn6_vv: <compact vv hash> }
```

- The `STROKES_Z:!` prefix is a **fence**: a still-open 5.x device opening a v6 file fails ("invalid STROKES")
  instead of opening an empty document and overwriting it. It costs 12 bytes and no code in v6.
- The snapshot makes the PDF self-contained: a signed-out reader or a device without relay access opens the full
  editable document from it; late joiners can start from it.
- Pages: `Background::Pdf` pages are copied from their source asset with pdf-lib `copyPages`, keeping the source
  page's content objects unchanged (per-page provenance, BEHAVIOUR-CHANGES B-D2); template pages get the template JPEG
  (q 0.85) as 5.18.0; custom pages get the photo. Ink and images are appended to each page as **one separate content
  stream** that only does `q /IHN6Ink Do Q`, where `/IHN6Ink` is a Form XObject from `render/pdfink.rs` (pen =
  Bézier path matching the screen spline; highlighter = `/Multiply` blend with `/CA 0.22` and `0.10` passes; images as
  image XObjects). Stripping that stream and the resource name restores the clean source page (§4.5). The snapshot
  records, for each exported page, the `(source hash, source page)` it carries.
- Assembly runs in `host/export-worker.js`: the core instance in the worker produces content streams and the page
  plan; pdf-lib copies pages, embeds images, attaches assets, sets Keywords, saves with object streams. Exports of
  annotated PDFs are therefore about the size of the source plus ink (no second copy of the source).

### 8.3 Upload

`export/upload.rs` + `drive/revisions.rs`:
- **First upload = create** with the document's reserved id (§7.1): multipart (≤ 4 MiB) or resumable POST
  `{id, name, mimeType:'application/pdf', parents}`; 409 = already created by a lost response → continue as update.
  Sets `flags.created`; the relay may start.
- **Update** follows SYNC §14.2: bytes are built first; then `GET` v2 etag + `headRevisionId`; then immediately a
  conditional media PUT (`If-Match`) ≤ 4 MiB, else resumable 1 MiB chunks with server-confirmed offsets and the
  5.18.0 zero-progress/idle deadlines. A 412 with an unchanged head is retried at once with the same bytes.
- Keepalive only ≤ 48 KiB (relay flushes; exports never use keepalive). File-name rules (`sanitizeFilename`, `.pdf`)
  and the name rule of SYNC §14.7.

### 8.4 Dropped export paths

jsPDF raster fallback, PNG/raw overlay rasters, `IH_OVERLAY_REFS`, `IH_CLEAN_ORIGINAL.pdf`, `IH_SYNC`, `IH_STRUCT`,
`IH_FIELDS`, `IH_VERSIONS` **writing** (all still **read**, §9). User-visible consequence (vector ink in exported PDFs,
smaller files) is BEHAVIOUR-CHANGES B-D1.

### 8.5 Export to the user (`export/user.rs`, W5.4)

Export modal (name prefill, `sanitizeFilename`, stores the name as a `DocName` op unless read-only) → editors run a
**forced export** (SYNC §14.1) and then download that head revision byte for byte, with 5.18.0's loading texts
(`'Guardando últimos cambios en Drive...'`, `'Descargando PDF de Drive...'`, `'Generando PDF...'`); viewers download
the current head; a document not yet created in Drive is built locally with the high-quality profile. Delivery
through the Android bridge v2 (`beginPdfExport`/`appendPdfExportChunk` ≤ 240,000 chars/`finishPdfExport`, await
`handleInhousePdfExportResult` ≤ 30 s, legacy `savePdf`) / `navigator.share` / `<a download>` exactly as 5.18.0.

---

## 9. Reading existing Drive documents (5.x formats)

### 9.1 Open path

`doc/open.rs`: download (cached copy if `modifiedTime` matches, as 5.18.0) → `host/pdf.js` opens with pdf.js and
returns `info.Keywords` (any string encoding pdf.js decodes) and the attachment list → Rust parses Keywords.

Every PDF from any source (Drive download, user import, upload, relay/P2P asset, signed-out download) passes
5.18.0's security-core limits, ported to `host/pdf.js` (pdf.js params) and `v5::import`/`open.rs` (checks): blob
≤ 64 MiB and type empty/`application/pdf`/`application/octet-stream`, else `This PDF is larger than the 64 MB safety
limit.`; `isEvalSupported:false`; `maxImageSize ≤ 40,000,000` px; 1..2000 pages, else `PDF page count exceeds the safe
limit`; v5 section limits (STROKES/IH_VERSIONS encoded ≤ 48 MiB, inflated ≤ 96 MiB; IH_STRUCT/IH_FIELDS ≤ 8 MiB).

Inflating and parsing a 5.x payload larger than 4 MiB (inflated) runs in the export-worker instance, which returns
compact encoded batches; the worker is terminated and recreated after such a job, so the main instance's linear
memory never grows to the size of a legacy JSON document (§14 memory gate).

1. Contains `;IHN6:` → v6 file: inflate (effect 3.10) → `load_snapshot` (merge into the local doc if one exists) →
   image assets from attachments if missing locally; source PDFs rebuilt from the pages on demand (§4.5).
2. Starts with `STROKES_` → 5.x file → `v5::import` (strict mode: invalid STROKES/STRUCT/FIELDS aborts the open with
   the 5.18.0 error, as 5.18.0 did for Drive/shared opens).
3. Neither → plain PDF: one `Background::Pdf` page per PDF page, sizes from the pdf.js viewport at scale 1 in points
   used as px (the 5.x convention); the source asset is the downloaded file (§4.5 hash rule). For an existing v6
   document this is a foreign overwrite: SYNC §15.1 (recovered copy).

### 9.2 What `v5::import` reads (all of `spec/documents.md` §3–§6)

`STROKES_Z` (zlib) and `STROKES_DATA` page entries (strokes with optional `syncStamp`, images with `src` data URLs →
assets, `deletedStrokeIds`/`deletedStrokeStamps` → tombstones, `backgroundSource`/`templateKind`/`pdfPageIndex`/sizes,
`sidePanel` kept opaque, legacy-raster entries without `images`), `IH_NORM` (duplicate-id repair when absent),
`IH_STRUCT` (page ranks/stamps/tombstones → `Pos` and page registers), `IH_FIELDS` (exportName, page background/template/
size registers), `IH_CAL` ignored (calendar removed), `IH_SYNC` ignored, `IH_VERSIONS` (schema 3 delta/checkpoint
archive and the legacy array) → versions, clean original from `IH_CLEAN_ORIGINAL.pdf` or the saved PDF with
`IH_OVERLAY_REFS` XObjects stripped (stripping done by pdf-lib in the export worker) → source asset, legacy ids
(`pg_legacy_`/`st_legacy_`/`im_legacy_` with `simpleHash`) generated exactly as 5.18.0, legacy cover strokes and
`eraser-area` strokes as legacy tools (rendered as 5.18.0 renders them).

### 9.3 Determinism (required for collaboration)

```rust
// W2.2 provides (pure; runs in the main instance or the export worker):
pub fn from_pdf(keywords: &str, facts: &PdfFacts, import_actor: ActorId, rev_time_ms: u64) -> Result<Vec<Batch>, V5Error>;
pub fn from_local(records: &V5LocalDoc, migration_actor: ActorId, base: Option<&Doc>) -> Result<Vec<Batch>, V5Error>;
```

- `import_actor = IMPORT_BIT | (fnv1a64(headRevisionId) >> 1)`; `rev_time_ms` = that revision's Drive `modifiedTime`
  (both from the same `files.get`). `PdfFacts` = page count, page sizes, attachment names/hashes, the source-asset
  hash of §4.5.
- Ids from 5.x ids (or 5.x deterministic legacy rules); `Pos` from ranks; one batch per page in page order, then one
  for document fields; seq 1, 2, …; explicit per-op stamps: 5.x stamps → `{clock, fnv64(actor string), 0}`;
  stampless registers → `{rev_time_ms, import_actor, idx}`, so the newer of two imported revisions wins regardless of
  import order (cargo test imports R0 and R1 in both orders).
- `from_local` (migration, §10) uses the migration actor `IMPORT_BIT | (fnv1a64("migrate:" ‖ inst13 ‖ ":" ‖ docId) >> 1)`
  and stamps stampless local data with the record's `savedAt`; it emits only what is not already in `base`.
- Two devices importing the same revision produce identical batches and converge with no duplicates; a later import
  of a newer revision merges by id (union minus tombstones, LWW registers).

### 9.4 "A 5.x device saved after v6 last saw it"

SYNC §15.3: an unknown head revision whose Keywords start with `STROKES_` is imported with its own import actor and
merged (never overwritten); the scribe then exports. This is the only place v6 reads 5.x data after migration.

---

## 10. One-time migration of 5.x local data (`migrate.rs`)

### 10.1 Sources (read-only dumps by `host/idb.js`, records decoded by `ihn-core::v5::idb`)

| Source | What becomes v6 data |
|---|---|
| `inhouse-offline-v1` (per account): `documents` with `dirty` or an `outbox` row; `local-` documents; folder outbox ops | snapshot (plain or packed `__inkCheckpoint` xor-v1/raw-v1) → import as a replica state merged over the base (`baseMeta` revision of the Drive PDF, imported with §9); `originalBlob` → source asset; `local-` docs → new local v6 docs to create in Drive; pending folders → Drive folder creates |
| `notebook-data-db-v1` and `background-notebook-data-db-v1`: `snapshots/latest`, `pages`, `stroke-ops`, `timeline-history`; localStorage `notebook-data-v3`, `drive-session-v1`, `drive-pending-exit-upload-v1` | the last-open document's working copy, using 5.18.0's rules: newest `savedAt` wins, adopt `MetaPayload.sessionId` before reading page rows, replay `stroke-ops` (dedupe by id), timeline rows → versions; merged with the offline snapshot of the same doc (CRDT merge, nothing lost) |
| `inhouse-original-pdfs-v1/originals` | source assets for Drive ids being migrated or cached |
| `inhouse-offline-v1/recoveries` | kept as assets + `kv` list so the Offline documents dialog still lists them |
| `inhouse-offline-v1/settings` `preferences`, `home-views`, `drive-root`; `entries` | `kv` and `catalog` (Home paints offline right after the upgrade) |

### 10.2 Algorithm

1. Runs once per account at first v6 boot, before Home lists render from cache, behind Web Lock
   `ihn6-migrate:<account>`; progress recorded in `kv.migration` per source (with the dump's fingerprint: per
   database the record count and the greatest `savedAt`/outbox id) so a crash resumes idempotently (all writes are
   id-addressed).
2. For each document with unsynced data: build the v6 doc from the Drive base (download or cached PDF, imported with
   §9.3; if offline, from the local snapshot alone), merge the local state with `v5::import::from_local` (batches of
   the migration actor, appended to `ops`, relayed like own batches, SYNC §15.4), and write it to `inhouse-notes-v6`.
   5.x `local-` documents get a pool id (§7.1; if the pool is empty and offline, that document waits for the
   network) and are created by the first export. Pending folders → Drive folder creates.
3. A document is **done** when `relay_durable_through` covers all its migration-actor batches (and, for a `local-`
   document, the Drive create succeeded). When every document is done, the 5.x data is deleted only if:
   - a fresh re-dump of the 5.x stores has the same fingerprint as the migrated one (no 5.x tab wrote since); if it
     changed, the migration re-runs on the difference first;
   - no 5.x client is open: the v6 service worker answers `clients.matchAll({type:'window', includeUncontrolled:true})`
     and every window must have announced itself as v6 (`postMessage('ihn6-hello')` at boot); otherwise retry at the
     next boot or `visibilitychange`.
   Then delete `notebook-data-db-v1`, `background-notebook-data-db-v1`, `inhouse-offline-v1`,
   `inhouse-original-pdfs-v1`, the `inhouse-media-*` caches and the read-and-removed localStorage keys. A blocked
   delete is retried at the next boot. Never delete before every document is done.

### 10.3 What is not migrated

Ink-journal records, live-preview state, leases, P2P peer caches, `drive-open-cache` rows (re-downloaded on demand),
5.x presence ids. Their content is either already in the snapshots above or transient.

### 10.4 Old clients

A 5.18.0 tab still open is reloaded into 6.0.0 by its own release watcher at the next Home/visibility check (it never
reloads an open editor). Until then:
- The fence (§8.2) prevents it from opening v6 files; anything it uploads is imported by SYNC §15.3; its local edits
  are picked up by the migration re-dump rule (§10.2 step 3).
- The v5 runtime files (`app-v5.js`, the other `*-v5.js`, `document-processing-worker-v5.js`, `rust-core-v5.js`,
  `wasm/inhouse_notes_core.wasm`, `vendor/jspdf/`, `boot-v5.js`, `bullet-template.js`, `ink-palette.js`) stay
  **deployed but unreferenced** in 6.0.0, and the v6 service worker keeps the `inhouse-shell-5.18.0` cache and
  answers requests carrying `v=5.18.0` from it first, then the network. Both are removed in 6.0.1.
- The browser's update check of the 5.x registration fetches `/service-worker.js`, now the v6 worker (same URL,
  classic), which installs and takes control; its `freshPage` rule then caches only HTML containing `v=6.0.0`.

## 11. Editor tools and the input pipeline

### 11.1 Pipeline (minimal latency, no extra hop)

```
pointerdown/move/up (capture on the page canvas; document-level release recovery as 5.18.0)
  host: getCoalescedEvents() → pack samples into a Float64Array view of wasm memory (no objects) → ihn_event(POINTER)
  rust: accept/reject (pen always; touch/mouse only with "Draw with finger", touch also 2 s after pen; palm rejection;
        second finger → pinch handover), page lookup with geometry cached at pointerdown,
        append samples (dedupe identical batches → zero work), request ONE frame
rAF → ihn_frame: live segments to the page bitmap + newest segment on the cursor overlay (one path, one stroke call)
pointerup: remaining segments; Tx.add_stroke → Batch → apply → journal append effect → SyncEngine.local_batch
           → undo entry; heavy work waits for 180 ms idle / 1200 ms burst quiet
```

Per event Rust does O(samples) work with no allocation (pre-reserved point vectors), no layout reads, no IDB or
network; the out-buffer for a pointer event is empty unless a frame is requested. Live preview packets for remote
viewers are produced on the frame, rate-limited by SYNC.md. Remote ink never blocks local ink: during contact,
remote batches are applied within a ~0.5 ms per-frame budget and the rest is queued for idle slices (§4.4); the perf
check includes a remote 20k-op batch arriving mid-stroke (no long task).

### 11.2 Tools (5.18.0 behaviour, model-backed)

Pen and highlighter presets/palette (`ink-palette.js` values ported), stroke eraser (whole strokes within
eraserWidth/2 of the centre-line, grid-prefiltered exact test), area eraser (split into new-id fragments at original
z), lasso (any point inside), selection box (move / scale about centre / rotate from originals, duplicate +18/+18,
palette-only recolour, delete, cross-page move at ±50 px), images (insert/paste/drop, place/transform), pen
double-tap on background, keyboard Ctrl+z/Ctrl+y, toolbar drag & snap, eraser menu, read/edit toggle (every session
starts in Read mode), "Draw with finger". Erasing repaints the damage region before lift. Selection, image placement
and cross-page drag use the page's real size (BEHAVIOUR-CHANGES B-E1/B-E2). Erasers test the swept segment between samples
(B-E6); single-point pen dots persist in repaints (B-E3); page metrics follow the DOM alignment (B-E5).

### 11.3 Viewport

Pan/zoom 0.5–3, wheel/trackpad, pinch with axis lock, >100 px teleport drop, fling decay 0.935 per 16.67 ms,
overscroll pull-to-add-page, fast-scroll thumb, view state persisted to `notebook-view-v1` (250 ms debounce), CSS
transform during gestures and re-raster after 90 ms idle — all 5.18.0 constants (`spec/editor.md` §3).

---

## 12. Workers, service worker, headless runs, lifecycle

- **pdf.js worker** (vendored `pdf.worker.min.js`): parsing and rendering, as 5.18.0.
- **Export worker** (`host/export-worker.js`): module received from the page, own core instance, pdf-lib; one job at a
  time; cancelled on accepted pen contact. Also runs large 5.x imports (§9.1) and the migration's parsing (§10); it
  is terminated and recreated after any job that grew its memory beyond 64 MiB.
- No other workers. Thumbnails, compaction and small v5 imports run on the main thread in idle slices (≤ 4 ms),
  gated by the input-priority scheduler (port of `document_work.rs`).
- **Service worker** (`/service-worker.js`, the URL every 5.x page registered; **classic** script, because an update
  keeps the registration's type): `const VERSION='6.0.0'`; cache `inhouse-shell-<VERSION>`, atomic `addAll` +
  `skipWaiting` + `clients.claim`, delete other `inhouse-shell-*` except `inhouse-shell-5.18.0` (§10.4), HTML
  cached-first with `no-cache` revalidation stored only if it contains `v=<VERSION>`, `?v=` mismatch bypass (a
  `v=5.18.0` request is answered from the 5.18.0 cache first), `/oauth-callback*` bypass. It also answers the
  migration's "which windows are v6" query (§10.2). It has **no background sync**: 5.x's `sync`/`periodicsync`
  outbox run is dropped (a service worker cannot start the export worker, and the relay keepalive flush on
  `pagehide` already makes edits durable); pending exports complete in the Android headless page or the next session
  (BEHAVIOUR-CHANGES N-5).
- **Headless mode** (`?inhouse_app=1&offline-worker=1`, Android WorkManager only): same page, no visible UI; for each
  document with `relay_pending`, `export_pending` or not created, take `ihn6-net`, flush the relay, create/export
  (forced export, SYNC §14.1); report `{busy, pending, errors}` to `InhouseBackground.complete`.
- **Lifecycle** (`export/lifecycle.rs`): Home button waits only for local durability (`'Saving on this device…'`
  overlay while a batch is uncommitted; in degraded mode, until sealed), then shows Home and lets sync/export
  continue; `pagehide`/hidden → flush journal + view state + keepalive relay flush; `beforeunload` prompts
  (`returnValue=''`) only while an exit export is in flight; hidden 60 s → suspend live transports (SYNC §9.3) and
  release canvases; visible → resume.

---

## 13. Home / Drive browsing

`home/` reproduces `spec/home.md` with 5.18.0's markup: topbar, FAB, tabs and bottom navigation, Recents/Starred/
Shared/Folders lists (Drive `files.list` queries and `fields` exactly as 5.18.0, built by `drive/home.rs`), A4 sheet
cards with Drive thumbnails (cached blobs offline), manila folder cards with the palette algorithm and the
`--folder-grain` art, opened folder view with breadcrumbs and deferred card hydration (`rows.rs`), folder zoom
(WAAPI plan from `motion.rs`), pull-to-refresh, 30 s auto-refresh (Recents + current folder), search (250 ms
debounce), fast-scroll thumb, offline decoration and sync badges (§6.6), create menu, upload, folder picker, new
folder, card ⋮ menu (rename, star, colour, copy, move, bin), move-to-bin slider, Bin (restore / delete forever),
Download-app modal (web), Offline documents dialog, shared/public link handling (`?drive=…&role=…&mode=edit&
resourcekey=…`), viewer mode. Catalogue state lives in `ihn-core::catalog` so Home paints from `catalog` before the
network answers.

Sync files never appear in Home: `catalog` and every list/search/offline-listing result drop the sync folder id
(`kv.relay-folder`) and any entry whose `appProperties.ihn6Kind` is set (the list `fields` include
`appProperties`). Card actions that touch sync state (W5.5): **Copy** sends `files.copy` with `properties` mapping
every `ihn6_*` key of the source to `null` (and the reader's doc check, SYNC §8.6, covers a copy made elsewhere);
**rename** PATCHes `name` only (SYNC §14.7 turns it into a `DocName` op on the next poll/open); **delete forever**
adds the id to `kv.relay-gc` (SYNC §8.7).

"Soon": Diary/Agenda buttons in the profile menu and `#btn-scan-page` keep their markup and call the Soon toast
(`#soon-toast`, `role=status`, text `Soon`, visible 1800 ms).

---

## 14. Size and performance budgets (CI-enforced where measurable)

| Budget | Limit | Gate |
|---|---|---|
| `wasm/ihn.wasm` | ≤ 480 KB raw, ≤ 160 KB gzip -9 (includes `jsmath`) | `build-wasm.mjs` fails above |
| First-party JS (boot, host/*, service worker, export worker) | ≤ 100 KB raw unminified | `build-wasm.mjs` |
| Critical-path bytes before Home paints from cache | ≤ 40 % of 5.18.0's (no pdf.js/pdf-lib/jsPDF at boot) | perf check |
| Compile + instantiate | ≤ 15 ms desktop; ≤ 60 ms at 4× CPU throttle | perf check (`performance.mark`) |
| Pen: script per pointer event | ≤ 0.1 ms mean, 0 allocations in steady state | perf check |
| Pen: frame work | ≤ 2 ms p95 at 4× throttle, 0 long tasks while writing, also with a remote 20k-op batch arriving mid-stroke | perf check (profile.mjs) |
| Input ack (profile.mjs `inputAckMs`) | p95 ≤ 5.18.0 | perf check |
| Dense page (2400 strokes × 100 pts) synchronous redraw | < 30 ms (5.x test) | e2e |
| Editor idle network | ≤ 10 % of 5.18.0 requests/min and KB/min (5.17.22 baseline 392 req/min, 191 KB/min at 4×) | perf check |
| Home idle network | 30 s auto-refresh only (≤ 6 req/min) | perf check |
| Editor idle CPU; JS heap + `wasm memory.buffer.byteLength` (main + export worker) | ≤ 50 % of 5.18.0's CPU and JS heap; memory measured also after opening the largest 5.x fixture | perf check |
| Pen-up → visible on a second device | P2P ≤ 150 ms p95; relay-only < 4 s p95 (SYNC §1) | two-device test |

### 14.1 Build (`scripts/build-wasm.mjs`)

`cargo build --manifest-path rust/Cargo.toml -p ihn-wasm --release --target wasm32-unknown-unknown --locked` with
`RUSTFLAGS="--remap-path-prefix=$CARGO_HOME=/cargo --remap-path-prefix=<root>=."`; profile `opt-level="s"`, `lto=true`,
`codegen-units=1`, `panic="abort"`, `strip=true`; no `build.rs`; no wasm-opt; no wasm-bindgen. Then: assert imports ==
§3.2 allowlist; assert `ihn_abi()==6` in Node; the `jsmath` grep gate (§6.5); enforce §14 sizes; write
`build/wasm/ihn.wasm`, `build/offline-assets.js`, `build/native-assets.json`, `build/index.html` (native map and
`native=` hashes filled); print sizes and SHA-256. Toolchain: `rust-toolchain.toml` 1.90.0. Boot gate: `host/main.js`
validates a tiny module using reference-types + multivalue and, if unsupported (WebView < 96), shows a static "update
Android System WebView" message in `#drive-status` instead of failing silently.

### 14.2 CI and generated files

- **Every unit / PR to `v6`** (`.github/workflows/production-checks.yml`, v6 version): `npm ci` → toolchain →
  `cargo test --locked --workspace` (native) → `npm run build` twice from two different paths, asserting identical
  SHA-256 of everything in `build/` (reproducibility without committing) → Playwright Chromium → `npm run test:e2e`
  → `npm run test:parity -- --quick` for the states the PR touches. Generated files are gitignored on `v6`, so
  parallel units never conflict on them.
- **Nightly on `v6` and the release PR to `main`**: the same plus the full parity matrix (5.18.0 served from
  `git worktree add <tmp> 1d6e8a0`) and the perf check. The release PR copies `build/` over the tree, commits it, and
  CI asserts `git diff --exit-code -- wasm/ihn.wasm offline-assets.js native-assets.json index.html
  vendor/fonts/ui.css` after a clean rebuild (byte-for-byte reproducibility, AGENTS.md).
- `deploy-pages.yml` required-file checks become `index.html, CNAME, boot.js, host/main.js, wasm/ihn.wasm,
  service-worker.js`; excludes add `rust/`, `build/`; `verify_live_release.py` file list updated to v6 files.

### 14.3 Release 6.0.0

1. `APP_VERSION = '6.0.0'` in `index.html`; both `[data-app-version]` show `v6.0.0`; `service-worker.js`
   `VERSION='6.0.0'`; `package.json` `6.0.0`; every `?v=` query `6.0.0`; `RELEASE_NOTES_v6.0.0.md` (first line
   `# v6.0.0`, mentions that the Android APK and frozen installer are unchanged).
2. Delete the v5 **tests** and `rust-core/` sources; keep the v5 runtime files deployed but unreferenced (§10.4) —
   they are deleted in 6.0.1 together with the `inhouse-shell-5.18.0` exception. `offline-v5.css` stays (byte-identical
   UI asset, name kept). Commit `build/` outputs (§14.2).
3. Full suite green (cargo, e2e, full parity with only approved BEHAVIOUR-CHANGES, perf check report).
4. The user reviews BEHAVIOUR-CHANGES and the parity report and **explicitly approves** the push.
5. Push to `main` with the version bump in the same commit; verify GitHub `main` and uncached
   `https://inhousenotes.com/` serve `v6.0.0` (AGENTS.md post-push checks) before reporting.
6. No APK build, no catalog change, no updater edit; `vendor/inhouse-services-updater/` is never touched.

---

## 15. Explicit non-goals for 6.0.0

Scanner, agenda/diary creation, Google Calendar (shown as Soon); v5 P2P/journal/preview/presence interop; writing
5.x Keywords sections; jsPDF; the hidden offline-processor iframe (the main app syncs itself; Android uses the
headless page); service-worker background sync; a Drive API key; tombstone garbage collection; wasm threads/SIMD; any
change to the Android APK or the frozen installer.
