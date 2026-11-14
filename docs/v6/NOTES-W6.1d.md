# W6.1d PARITY — the last three causes — notes

Branch `v6-w6_1d` (from `v6-int` at `b5d432b`). The three failures left open by W6.1c
(`docs/v6/NOTES-W6.1c.md`, "Open") are fixed at their cause; no tolerance, expected-difference entry or state was
changed.

Method: a copy of the 5.18.0 tree with `console.log` probes in `app-v5.js` (scratchpad only) and temporary
`host.log` probes in v6 (`#[track_caller]` on `schedule_visible`), forwarded by a local, uncommitted console hook in
`run.mjs`, gave both apps' sequence of visible passes, preview jobs and wrapper backgrounds on the fake clock.

## 1. `home/folder-zoom-open@50` (desktop-1280, phone-360)

- **Cause**: 5.18.0 creates the folder view's fade paused in `prepareScene`, then `zoom.ready.then(joinMotion)`:
  keyframes from `zoom.currentTime / duration`, `fade.play()`, `fade.startTime = zoom.startTime` (the fade runs on
  the zoom's own timeline). v6 built the fade at its prepare timer from `now − click` with a negative delay.
- **Fix** (ABI addition, as W6.1c proposed):
  - `host/dom.js`: listen type `ready` on an animation handle → one dispatch when its `ready` promise resolves
    (nothing if it rejects or was unlistened); DOM op 30 `anim_set(anim, keep|play|pause, keyframes|null, with)`:
    `effect.setKeyframes`, `play()`/`pause()`, `startTime = with.startTime`.
  - `host/main.js`: metric 23 `AnimCurrentTime` (`animation.currentTime`, 0 when unresolved).
  - `abi.rs` (`dom_op::ANIM_SET`, `ety::READY`), `dom.rs` (`OutBuf::anim_set`, `AnimPlay`), `host.rs`
    (`Metric::AnimCurrentTime`), self-test golden in `testapi.rs`.
  - `home/folders.rs`: `prepare_scene` creates the held fade (`[{opacity:0},{opacity:0}]`, paused) and listens to the
    zoom's `ready`; `join_motion` ports `joinMotion` exactly. A closing retarget now sets the zoom's keyframes
    (`zoom.effect.setKeyframes`) instead of cancelling and recreating it with a negative delay. `home/mod.rs` routes
    the zoom listener's `ready` to `join_motion`, its `finish` to `cleanup` as before.
- **Result**: passes, `--runs 2` on both viewports (folder-zoom-open and -close, 48/48 comparisons).

## 2. `pages/photo-modal@added` (desktop-1280, phone-360)

- **Cause**: 5.18.0's `insertPhotoPageAt` takes `acquireLocalPageStructureMutation()`, which polls every 32 ms (12 s
  deadline) while `hasSmoothInteraction()` is true. The tap on "Add Page" is itself such an interaction (the
  capture `pointerdown` starts a gesture), so 5.18.0 inserts only after the gesture settles (+128 ms in the harness:
  settle at +100, next poll at +128). Its `goToPage` resync then runs its forced visible pass (+180) before the new
  page's scroll preview exists, and the preview's completion does not touch a drawn active page: the wrapper keeps
  the photo URL. v6 inserted in the tap's instant; the gesture settle 100 ms later re-armed the resync timer, whose
  forced pass then ran after the preview and showed it (page-corner pixels).
- **Fix**: `editor/viewport.rs` `smooth()` (v5 `hasSmoothInteraction`: pen writing, gesture, panning, pointers,
  inertia, a Pages-panel drag `pages_panel::dragging`, a selection transform `selection::transforming`);
  `panels/photo_page.rs` `Step::Acquire` + `T_ACQUIRE`: the insert waits as 5.18.0 does (32 ms polls, 12 s deadline →
  "Failed to add photo page").
- **Result**: passes, `--runs 1` both viewports and in the quick matrix (only the existing N-1 save-dot tag remains,
  the same box as `pages/drag-reorder` and `pages/delete-slider`).

## 3. `canvas/dense-page@repaint` (desktop-1280), intermittent on the v6 side

- **Cause**: not the image decode (`images_pending` was false in every traced run). 5.18.0's
  `processScrollPagePreview` picks a page, draws background and strokes and encodes the JPEG in **one task** (its
  only yields are an image decode and a stroke slice past 3 ms; under the frozen clock it never slices). v6 created
  the job in one timer task and drew it in a second, 0 ms timer task. On the fake clock the editor frame due at the
  same instant (the open's resync-retry visible pass, requested earlier than the 0 ms timer) ran in between, so the
  preview arrived after the last visible pass: wrapper without preview, Δ2 at the page corner. Whether this happened
  depended on where v6's open placed its first render (0 or 16 ms, varying between runs), which moved the first
  preview attempt by one frame.
- **Fix**: `editor/pages.rs` `preview_step` starts and processes the job in the same step (no hop between
  `ScrollPreview::begin` and the drawing); the encode's result already arrives in the same host call.
- **Result**: passes, `--runs 3` (8/8 + no B-vs-B stability warning; before: runs 1 and 2 failed, B run 2 ≠ run 3).

## Verification (after the last code change)

See the report at the end of this file.

## Open

- 5.18.0's other page-structure edits also take `acquireLocalPageStructureMutation()` (`addPage`, `movePage`,
  `confirmDeletePage`, `applyRestoredPages`, `restoreVersion`); v6 still runs those at once. Their parity states pass
  today (the tapped control is not followed by a visible-pass race there); porting the same wait to them is a
  follow-up, not done here to keep this round to the three causes.
- v6's open places its first page render at fake 0 or 16 ms from run to run (5.18.0: always 16). After fix 3 this no
  longer changes any pixel of the traced states; not investigated further.
