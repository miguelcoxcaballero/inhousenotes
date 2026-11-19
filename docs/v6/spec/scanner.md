# Scanner: behavioural specification (v5.17.22, reference for v6.0.0)

Scope: the document scanner that turns a photographed Inhouse bullet-journal page into editable native pen strokes inside a notebook. It covers the live camera, capture readiness, stencil (page template) detection, the curved-page geometry, colour and illumination correction, ink extraction and vectorisation, the parallel backup photos and their review, the hand-off to the editor, workers, UI states and the tests that pin each of these.

All `file:line` references are to the v5.17.22 checkout. "Rust" means `rust-core/src/*.rs`, compiled to `wasm/inhouse_notes_core.wasm` and bridged by `rust-core-v5.js`. Where Rust and JS both implement a kernel, the JS version is the fallback and must stay numerically identical (see §11).

---

## 0. File map and runtime status

| File | Role | Loaded by |
|---|---|---|
| `scanner/index.html` | Scanner document. It holds the legacy "Scanner Pro" workspace and the live scanner dialog `#liveScanner`, plus the script order (`scanner/index.html:264-281`). | iframe / standalone |
| `scanner/live-camera.js` | The live camera, readiness, burst capture, parallel review and add/retake/close. **This is the primary UI.** | page |
| `scanner/live-camera.css` | Styles for `#liveScanner` | page |
| `scanner/processing/liveDetection.js` | Synchronous frame detector at ≤480 px, plus illumination normalisation and colour-reference finder (`SP.LiveDetection`) | page and both workers |
| `scanner/processing/capture-quality.js` (+ `capture-quality-worker.js`) | Sharpness and dot-lattice preflight on a frozen still (`SP.CaptureQuality`) | page, plus one readiness worker |
| `scanner/processing/lightweight.js` | Still pipeline: detection, warp, colours, encode (`SP.Lightweight`) | page and detection worker |
| `scanner/processing/detection-worker.js` | Runs the complete per-photo pipeline off the UI thread | one worker per photo |
| `scanner/processing/surface-parameterization.js` | Perspective/anchor re-parameterisation of the frame curves (`SP.SurfaceParameterization`) | page and worker |
| `scanner/processing/metric-surface.js` | Physical 210×297 mm developable surface plus pinhole fit, measured-lattice fallback, dot registration and the raster certificate (`SP.MetricSurface`) | page and worker |
| `scanner/processing/ink-vectorizer.js` | Ink axes to native pen strokes (`SP.InkVectorizer`) | page and worker |
| `scanner/processing/scan-template.js` | Template geometry constants, the printable SVG and a wrapper around the native renderer (`SP.ScanTemplate`) | page and worker |
| `bullet-template.js` | Native editor template renderer (`InhouseBulletTemplate.render`). The editor uses the same file. | page and worker |
| `ink-palette.js` | `InhouseInkPalette = ['#4D4D4D','#002FD9','#E81010']` and `InhouseInkDefaults = {width:2,pageWidth:794,pageHeight:1123}` (`ink-palette.js:4-5`) | page, worker, editor |
| `rust-core-v5.js` | WASM loader and the scanner kernels `warp`, `scanColors`, `balancePaper`, `separateInk`, `surfaceFit`, `inkAxes` (`rust-core-v5.js:31-186`) | page and worker |
| `scanner/script.js` | Legacy multi-page "Scanner Pro" workspace (gallery, crop, Ref overlay, PDF export), embed bridge `ihn-scanner-ready`, config bootstrap | page |
| `scanner/configLoader.js`, `scanner/values.config` | Converts 0–100 user values to technical config. **Only `CALIBRATION_TARGETS` reaches the live pipeline** (`lightweight.js:5359`), and its values equal the built-in fallbacks. | page |
| `scanner/processing/{colorTransform,colourValues,mesh,preMesh,yellowMask,yellowRectangle,edgeDetectionFallback}.js` | **Dead legacy.** No page or worker loads them. They appear only in `offline-assets.js` (precache) and the native-asset hash map in `index.html:94`. They are not needed for identical behaviour. | none |
| `app-v5.js` (editor) | Opens the iframe, validates and inserts pages (`app-v5.js:19497-19745, 20453-20560`) | editor |

Every asset URL carries `?v=<APP_VERSION>&native=<sha256>`. Workers inherit their parent script's `location.search` (`detection-worker.js:10`, `capture-quality.js:227`, `lightweight.js:3560-3561`), and the WASM URL drops `native` (`rust-core-v5.js:5-9`). The scanner works offline because its assets are precached; `tests/e2e/app-resilience.spec.mjs:310` covers this.

External network dependencies (optional): Google Fonts and Material Symbols (`scanner/index.html:9-12`), Sortable 1.15.0 and jsPDF 2.5.1 from cdnjs for the legacy workspace only (`script.js:1510`, `script.js:841`). jsPDF is borrowed from `window.parent.jspdf` when embedded.

---

## 1. Embedding in the editor (entry, protocol, insertion)

### 1.1 Opening
* The button is `#btn-scan-page` ("Scan pages") in the Manage-Pages panel (`index.html:9993`), wired at `app-v5.js:16400`. It is disabled when the document is read-only (`openPagesPanel`).
* `openScannerEditor()` (`app-v5.js:19515`) works as follows:
  * It does nothing if the document is read-only, an import is in progress, or the overlay or frame is missing.
  * It closes the pages panel and the cover modal, calls `ensurePageIds`, and creates a session id with `crypto.randomUUID()`. The fallback is `scan-<base36 time>-<random>` (`app-v5.js:19497`).
  * It sets the iframe `#scanner-editor-frame` (inside `#scanner-editor-overlay`) to `scanner/index.html?embed=1&session=<id>&v=<APP_VERSION>`, adds `.visible` and `aria-hidden=false` to the overlay and `body.scanner-editor-open`, then focuses the frame.
* Overlay markup: `<iframe id="scanner-editor-frame" title="Inhouse Scanner" allow="camera" referrerpolicy="same-origin">` (`index.html:10019-10020`).
* Overlay styles: fixed, z-index 12000, background `#131314`, 160 ms opacity fade, no transition under reduced motion (`index.html:3216-3257`).
* `closeScannerEditor({force})` (`app-v5.js:19535`) is refused while an import is running unless `force` is set. It clears the session id, hides the overlay, and on the next tick sets the iframe `src` to `about:blank`, which releases the camera and workers. Leaving the editor, going to Drive home, and similar transitions call it with `force` (`app-v5.js:2184, 6120, 24983`).

### 1.2 postMessage protocol (same origin only; `sessionId` must match)
Scanner → app messages carry `{source:'inhouse-scanner', type, sessionId, ...}`. App → scanner messages carry `{source:'inhouse-notes', type, sessionId, ...}`. Both sides check `event.origin === location.origin` and the expected `event.source` (`app-v5.js:19711`, `live-camera.js:1063-1069`, `script.js:1068-1077`).

| type | Direction | Payload | Behaviour |
|---|---|---|---|
| `ihn-scanner-ready` | scanner → app | none | Sent by `script.js:1518` after config loads |
| `ihn-scanner-init` | app → scanner | `documentName` (`state.exportName` or 'Untitled'), `theme`, `maxPages: 24`, `openSource: true` | Live camera stores `documentName` (≤160 chars, `live-camera.js:1067`). The legacy workspace applies the theme and clamps `maxPages` to 1–48. |
| `ihn-scanner-close` | scanner → app | none | Sent by the close button or Escape. The app calls `closeScannerEditor()`. |
| `ihn-scanner-pages` | scanner → app | `pages:[result]`, `documentName`, `stencil:false` | The app runs `importScannedPagesFromBridge` |
| `ihn-scanner-result` | app → scanner | `ok`, `count` or `message` | The scanner finishes the submission (§3.9) |

### 1.3 App-side validation (`validateScannerPages`, `app-v5.js:19576`)
* There must be 1–`SCANNER_MAX_PAGES`=24 pages. Each blob must be `image/*`, between 1 byte and 18 MiB, and the total must not exceed 120 MiB (`app-v5.js:406-408`).
* `inkLayer === true` requires `blob.type === 'image/png'` and `templateKind === 'default'`. `width` and `height` must be finite and positive; they are rounded and clamped to 1–12000.
* `editableInk` must be absent unless `inkLayer` is set. `validateScannedInk` (`app-v5.js:19553`) applies these checks:
  * `version === 1`, with dimensions equal to the page's.
  * At most 60000 strokes and 500000 points in total.
  * Each stroke has `tool === 'pen'`, a colour in the palette (`SELECTION_ALLOWED_COLORS = InhouseInkPalette`, `app-v5.js:357`), `0 < width ≤ max(w,h)`, at least 2 points, and every point inside `[0,w]×[0,h]`.
  * Each point gets pressure `p: .5`.
* `importScannedPagesFromBridge` (`app-v5.js:19632`) then runs these steps:
  1. It decodes each blob to a data URL. For `inkLayer` pages it decodes the PNG and requires the natural size to equal the declared size and the image to be portrait (`width < height`).
  2. It scales the strokes to the logical page size (`getScannedPageLogicalSize`: portrait 794×1123, landscape 1123×794; `A4_WIDTH/HEIGHT` at `app-v5.js:294-295`). Points are multiplied by `sx = pageW/w` and `sy = pageH/h`; widths by `sqrt(sx·sy)`.
  3. It yields every 2 pages and aborts if the session or document changed.
  4. It inserts the pages after `state.activePageIndex`, re-anchored to that page's `pageId`.

### 1.4 Insertion format (`insertScannedPagesAt`, `app-v5.js:20453`)
The function acquires the local page-structure mutation (12 s), loads all pages, clears the selection and pending stroke ops, and resets the dirty, access and sync maps. It then builds each page with `createBlankPageData`:
* **Native ink page** (the normal camera result: `inkLayer && templateKind==='default'`):
  * `backgroundImage: null`, `backgroundSource: 'template'`, `templateKind: 'default'`.
  * `strokes` = the scaled pens, each with a new `generateStrokeId()` and a collaboration stamp.
  * No image item.
* Native ink layer without `editableInk` (legacy integrations): the PNG becomes one `images[]` item centred on the page at full page size, rotation 0.
* Uncorrected or other images: `backgroundImage: dataUrl`, `backgroundSource: 'custom'`.

The pages are spliced in and `noteCollabPageAdded` is called for each. The function sets the active page to the first inserted page, clears history, reindexes, re-renders, marks everything dirty, calls `scheduleSave(true)` and `goToPage`. Afterwards the app posts `ihn-scanner-result{ok:true,count}`, shows "Scanned page added" (or "N scanned pages added") and force-closes the scanner. On failure it posts `{ok:false,message}` and shows "Failed to add scanned pages". An `AbortError` is silent.

---

## 2. The printed page template (shared by printing, detection and output)

The physical units are cm on a 21×29.7 cm A4 sheet, defined in `bullet-template.js:48-124` and `scan-template.js:194-201`. Drawing scale is `u = canvasWidth/21`.

| Element | Geometry | Style |
|---|---|---|
| Outer frame | `roundRect(1.5u, 1.43u, 18u, 27u, r .03u)`, so it spans x 1.5–19.5 and y 1.43–28.43 | stroke `#f0db4c`, width .06u |
| Bullet dots | 35 columns × 51 rows at `(2.0 + 0.5c, 1.93 + 0.5r)` cm, a 5 mm pitch | radial gradient `#a8a8a8`, core radius .035u, blur σ .02u, stamp cached for up to 4 sizes |
| Calibration boxes | Two `roundRect`s: x 2.0–10.0 and 11.0–19.0, y 27.43–27.93 | yellow stroke |
| Box ticks | Vertical lines every 0.5 cm: x 2.5…9.5 and 11.5…18.5 | yellow stroke |
| Colour markers | Circles of radius .125u at y 27.68 and x 10.125, 10.375, 10.625, 10.875, filled red `#ff0000`, black `#000000`, blue `#0000ff`, green `#6eff12` | each with a yellow outline |

In mm (`scan-template.js` `geometry`): frame `{x:15, y:14.3, w:180, h:270}`, bullets start at `{x:20, y:19.3}` with pitch 5, boxes start at x 20 and 110 at y 274.3, markers centred at x 101.25/103.75/106.25/108.75 and y 276.8. Most geometry code works in normalised page coordinates (u = x/210, v = y/297) or in frame coordinates ((x−15)/180, (y−14.3)/270).

* `render(ctx,w,h,{background,dots})` resets the context state (identity transform, `source-over`, no filter, no shadow, `butt`/`miter` caps and joins), fills white unless `background === false`, and draws the elements above. `SP.ScanTemplate.render` forces `background: options.background === true`.
* `SP.ScanTemplate.svg({dots,background})` returns the printable SVG: a dot `pattern` from (1.75, 1.68) with pitch .5 and a `feGaussianBlur` of .02. `dots:false` gives the overlay-only SVG.
* Stencil downloads (legacy UI and `#downloadEmbedStencilBtn`, `script.js:844-899`):
  * `scanner_stencil.svg` is the raw SVG.
  * `scanner_stencil.png` is the SVG drawn on a white canvas of `SP.Dims.A4_W×A4_H`.
  * `scanner_stencil.pdf` is the same canvas as a JPEG at quality .95, placed at 0,0,210,297 mm with jsPDF (portrait A4).

---

## 3. Live camera UI (`scanner/live-camera.js`, `scanner/index.html:194-262`)

### 3.1 DOM

```
section#liveScanner[role=dialog aria-modal=true aria-labelledby=liveTitle hidden]
  data-state = closed|requesting|starting|searching|stable|capturing|processing|review|error
  data-phase = camera|processing|review|error        data-camera-ready = true|false
 header.live-header: h1#liveTitle "Scan", button#liveCloseBtn (aria-label "Close scanner")
 div#liveCameraView
   div#liveStage: video#liveVideo[autoplay muted playsinline], canvas#liveOverlay[data-detected]
   .live-camera-controls: p#liveStatus[role=status aria-live=polite], p#liveHint[hidden],
     progress#liveProgress[max=1] ("Automatic capture readiness"),
     .live-actions: #liveImportBtn "Photos", #liveRetryBtn "Open camera"[hidden], #liveCaptureBtn "Capture"[disabled]
     input#liveFileInput[type=file accept=image/* hidden]
 div#liveReview[hidden]
   .live-review-image > .live-review-canvas: img#livePreview[hidden], canvas#liveProcessingOverlay[hidden data-mode data-point-count]
   .live-review-controls: p#liveReviewStatus[role=status], ol#liveProcessingSteps[hidden]
       li[data-step=dots|surface|colors|template][data-complete][aria-current=step],
     div#liveAlternatives[role=group] (one button per candidate, aria-pressed), #liveRetakeBtn "Retake", #liveAddBtn "Add page"
```

Styling (`live-camera.css`):
* Fixed full-screen dialog, z-index 10000, dark (`#111213`), safe-area padding, yellow accent `--live-yellow: #f4d84b`.
* The video and overlay share one `object-fit: contain` box.
* `[data-camera-ready=false] #liveVideo{visibility:hidden}` hides Android's placeholder.
* `data-phase=error` hides the stage, the Capture button and the progress bar, and centres the controls.
* A spinner shows before `#liveReviewStatus` in the `processing` and `capturing` states. The current step pulses (1100 ms); completed steps are yellow.
* `.scan-reveal` plays a 180 ms scale-in animation.
* Thumbnails are 44×56 px; the selected one has a yellow border.
* Responsive rules apply at ≤420 px width and in short landscape (row layout with a 270 px side panel).
* Reduced motion disables all animations.

### 3.2 State machine
`setState` (`live-camera.js:61`) maps `review` to phase review, `capturing`/`processing` to processing, `error` to error, and everything else to camera. `#liveCaptureBtn` is enabled only in `searching` and `stable`.

```
closed --open()--> requesting --stream--> starting --first fresh frame+1--> searching <--> stable
   ^                   |  (error)                                             | ready  |
   |                   v                                                      v        |
   +--close()------ error <--- track ended / watchdog / encode failure ---- capturing   |
                       ^                                                      |        |
                       +--- visibility hidden: pauseCamera ---                 v        |
                                                        processing <--> review (Retake→open)
```

### 3.3 Opening the camera (`open`, `live-camera.js:153`)
1. If submitting, do nothing. Increment `generation` (the token for all async work), stop any previous camera and clear the review.
2. On first show:
   * Remember `document.activeElement`.
   * Set every other `body` child to `inert`, remembering each one's previous value.
   * Add `body.live-scanner-open` and remove `#sourceModal.open`.
   * Show the camera view, hide the review and the retry button, set state `requesting`, focus `#liveCloseBtn` and set the status to "Opening camera…".
3. Call `getUserMedia({audio:false, video:{facingMode:{ideal:'environment'}, width:{ideal:2560}, height:{ideal:1920}}})`. On `OverconstrainedError`, retry with `{video:true}`. If `navigator.mediaDevices.getUserMedia` is missing, report "Camera unavailable".
4. If the token is stale, stop the acquired tracks. If the document is hidden (Android permission sheet), stop the tracks, run `pauseCamera(true)` and set `resumeAfterPermission`; `visibilitychange` reopens the camera when visible.
5. Apply `{advanced:[{focusMode, exposureMode, whiteBalanceMode: 'continuous'}]}` for each mode the track advertises as supported. Failures are ignored.
6. If a track `ended` while `starting`/`searching`/`stable`/`capturing`, call `cameraError(null, true)`, which shows "Camera interrupted".
7. Set state `starting`. Count frames with `requestVideoFrameCallback` (a frame counts only while the track is not muted), start the overlay rAF loop, call `video.play()` (a rejection becomes a camera error) and call `tick(token)`.

Camera errors (`cameraError`, `live-camera.js:141`) set state `error` and show `#liveRetryBtn`. The title is one of "Camera interrupted", "Camera permission needed" (`NotAllowedError`/`SecurityError`), "No camera found" (`NotFoundError`/`DevicesNotFoundError`) or "Camera unavailable". The hint is "Allow camera access in your browser or device settings, then try again. You can also choose a photo." when permission was denied, and "Try opening the camera again, or choose a photo to scan." otherwise.

Pausing (`pauseCamera`, `live-camera.js:1037`):
* It is triggered by `visibilitychange` to hidden, or by `pagehide` (which uses `includePermission=true`).
* It runs only in `requesting`, `starting`, `searching`, `stable` or `capturing`. In `requesting` it is ignored unless `includePermission` is set.
* It stops the camera, **clears the review**, sets state `error` and shows "Camera paused" / "Open the camera to continue scanning."

Camera permission platform notes: HTTPS or localhost is required; the parent iframe grants `allow="camera"`. The Android APK declares `android.permission.CAMERA` with `uses-feature camera` and `camera.autofocus` set to `required=false` (`android app/html_to_apk_builder.py:1449-1460`). On older APKs the camera fails, but import still works.

### 3.4 Analysis loop (`tick`, `live-camera.js:373`)
`tick` runs only in `starting`, `searching` or `stable`. A hidden document triggers `pauseCamera()`.
* **Fresh frame**: `readyState ≥ 2`, `videoWidth > 0`, a frame clock `> 0` that differs from the previous one, and a live, unmuted track. The frame clock is the rVFC count, or else `getVideoPlaybackQuality().totalVideoFrames`, or else `currentTime`.
* **Watchdog**: with no fresh frame for 12 s from start (before any frame) or 5 s since the last frame, call `cameraError(null, true)`. If the track is muted or no frame arrived for >750 ms, reset readiness, clear the overlay, set `cameraReady=false` and state `starting`, and show "Waiting for camera…".
* The **first** fresh frame is not analysed: record it and re-tick after 120 ms.
* On each later fresh frame:
  * Set `cameraReady=true`.
  * Run `detect()`: draw the video into a canvas scaled so the longer side is ≤480 px, then call `SP.LiveDetection.detect(imageData, lastDetection)`.
  * On a hit, call `observeVisual` (§3.5) and update `lastDetection` and `lastObservedAt`. A miss clears `lastDetection` only after 350 ms.
  * Dispatch `window` event `scanner-live-detection` with the detection (or null).
  * If `captureReady(detection)`: set `lastQualifiedLiveAt`, state `stable` and "Checking page…". If no readiness check is pending and ≥160 ms have passed since the last one, call `checkReadiness(detection, token, true)`.
  * Otherwise: reset readiness if no qualified frame for >500 ms, set state `searching`, and show "Focusing…" (when the detection exists with `focusScore < 1`) or "Fit the yellow frame".
* An exception resets everything and shows "Could not check the page" / "Try again or choose a photo.".
* Next tick: `setTimeout(max(25, 80 − analysisTime))`, giving an 80 ms cadence.

`focusScore(d) = max(d.sharpness/32, d.edgeSharpness/160)` (`live-camera.js:287`).

`captureReady(d)` (`live-camera.js:289-295`) requires all of:
* `source==='yellow'`
* `confidence ≥ .86`
* `focusScore ≥ 1`
* `edgeSupport ≥ .7`
* `markerSupport ≥ .55`
* at least 3 `frameEdgeSupport` values ≥ .7
* `75 ≤ brightness ≤ 252`
* `coverage ≥ .16`
* every quad corner within [.008, .992] in both axes

### 3.5 Overlay drawing and motion filter (`live-camera.js:237-370`)
* Canvas size is the video box × `min(devicePixelRatio, 2)`. Geometry is normalised (0–1) and mapped through the contain letterbox. Strokes are `#ffde00`.
* For `source==='yellow'`, a dashed (`[6,5]`·dpr) 1 px page outline (`pageEdges` or `pageQuad`) is drawn first. Then the frame contour at 3 px: `contour` if it has more than 4 points, else the flattened `frameEdges`, else `quad`. Then the markers at 1.4 px (each `markerEdges` group flattened, else the `markers` quads).
* `overlay.dataset.detected` is 'true' or 'false'. Only observed geometry is drawn, never a fixed frame.
* `observeVisual(next, now)`:
  * **Reset** (snap to the new measurement, `visualResponse = 100`) if there is no target, the source changed, the last observation is more than 350 ms old, or the max corner movement is more than .16.
  * Otherwise, fit a common similarity transform (translation + rotation + scale from the four corners: `a = Σ(x·u+y·v)/Σ|x|²`, `b = Σ(x·v−y·u)/Σ|x|²` about the centroids). Its max corner displacement is `commonMotion`.
  * `visualResponse = clamp(110 − (commonMotion/max(16,dt))·350000, 35, 110)` ms.
  * `motionWeight = min(1, .35 + commonMotion/.012)`.
  * New target for each point: `p' = p + (T(p)−p)·motionWeight`, then `target = p' + (measured − p')·.32`.
  * `contour` is always rebuilt from `frameEdges.flat()`, so the outline and its samples stay identical.
* `renderOverlay` (rAF): if the target is fresh (≤350 ms) and the camera is ready, interpolate `alpha = 1 − exp(−min(dt,64)/visualResponse)` toward the target and draw with opacity `min(1, (350−age)/190)`. Otherwise clear.
* A `ResizeObserver` on the video redraws.

### 3.6 Readiness preflight (`checkReadiness`, `live-camera.js:502`)
Only one check runs at a time. A manual request made during a pending automatic check sets `manualReadinessRequested`.
1. Take a `snapshot()` still: a canvas scaled so the longer side is ≤2560 px. It throws "The camera image stopped. Open the camera and try again." if the track is not live, is muted, or has had no frame for >750 ms, and "Wait for the camera image, then try again." if the video has no dimensions.
2. Re-detect on that still, seeded with the live detection. For an automatic check that fails `captureReady`, the result is `{valid:false, reason:'page-not-ready'}`; otherwise call `CaptureQuality.inspectReadinessAsync(still, observed, {signal})` (§6).
3. Dispatch `scanner-capture-quality` (the quality object).
4. Evidence window:
   * Invalid: reset.
   * Valid: if this frozen frame was taken >650 ms after the previous completion, reset first. Then start the window if empty, set `readinessCompletedAt = now` and increment `samples`.
   * `ready = valid && (!automatic || (samples ≥ 3 && frozenAt − windowStart ≥ 300 ms))`. A manual request accepts the first valid still.
   * `#liveProgress.value = ready ? 1 : min(.95, samples/3)`. Dispatch `scanner-capture-readiness` with `{...quality, samples, elapsedMs, ready}`.
5. If ready, call `capture(automatic, observed, {canvas, quality})`; the verified still becomes burst frame 0. Otherwise set state `stable` or `searching` and show "Checking page…", "Focusing…" (reason `focus-or-motion-blur`) or "Finding page dots…".
6. Errors other than `AbortError` reset readiness and show "Checking page…". Stale results (token, request id or abort changed) are dropped at every await.

> The README says "five frames and 800 ms"; the **code** requires 3 valid preflights spanning ≥300 ms (pinned by `scanner-live-resilience`, `scanner-moving-capture` and `scanner-slow-readiness`). v6 must follow the code.

### 3.7 Burst capture (`capture`, `live-camera.js:567`)
* Pressing Capture with no prepared still calls `checkReadiness(lastDetection, token, false)` instead (a manual preflight).
* With a prepared still: set state `capturing`, show "Scanning…", stop the tick timer and capture up to **3** frames:
  * Frame 0 is the verified still with its trigger detection and quality.
  * Frames 1 and 2 each wait (polling every 12 ms, up to 180 ms) for a new video frame, then snapshot and detect (seeded). A frame needs its own `inspectReadiness` when `source==='yellow'`. It is skipped if the quality is invalid, or (automatic only) if it fails `captureReady` or any corner moved >.03 from the trigger quad.
  * Each kept frame is encoded immediately and asynchronously as JPEG quality .94 with a 6 s timeout ("Image encoding took too long" / "Could not encode this scan"). Its score is `focusScore(detection) × min(1.5, clarity/.5)`. Each carries `requireGrid: true`.
  * The first kept frame is shown at once as a frozen thumbnail (≤720 px JPEG .85 data URL in `#livePreview`). The review view opens with "Scanning…" and Add disabled.
* If no frame is kept: rethrow the capture error if there was one; otherwise reset to `searching`, show "Focusing…" and resume `tick`.
* Otherwise, `stopCamera()` (no more frames are needed) and await the encodings. Sort the candidates by score (descending), create object URLs, call `showAlternatives()` (buttons only when there are 2 or more candidates, labelled `Review frame N`) and then `reviewCandidates`.
* Errors set state `error`, show the retry button and display "Could not capture the page" with the error message.

### 3.8 Review and parallel processing (`reviewCandidates`, `live-camera.js:901`)
* Abort any previous batch and create a new `AbortController`. Set `pendingCandidates = n`, show candidate 0, wait one rAF plus a macrotask, then start **all candidates concurrently**:
  `SP.Lightweight.processPageAsync(blob, {signal, liveDetection: candidate.detection, previewProgress: true, requireGrid: candidate.requireGrid===true, onProgress})`. Each call runs in its own worker (§8.0).
* Acceptance per candidate:
  * Throw "The page geometry could not be verified" if `detection.quality.valid===false && corrected`.
  * Throw "The corrected page dots could not be verified" if `requireGrid && corrected && dotAlignment.status !== 'verified'`.
  * Decode the display blob (`previewBlob || blob`) with `Image` (6 s timeout; errors "Image decode timed out", "This image could not be opened", "This image is empty").
  * When there is a `previewBlob`, also decode the ink blob and require equal dimensions ("The scan layers do not match").
  * On success: `candidate.result = {blob, width, height, name:'Scanned page', corrected, inkLayer, templateKind, editableInk}`, `analysis = detection`, `error = corrected ? '' : (failureReason || "Couldn’t correct this photo. Try again.")`, `outputQuality`, `resultUrl`.
  * On exception: `error = message`, and the result falls back to the original blob with `corrected:false` when it is decodable.
* On completion:
  * The thumbnail image switches to `resultUrl`. The button is disabled when uncorrected, with aria-label "Photo could not be corrected".
  * Unless the user made an explicit choice that is still valid, select the best-ranked corrected candidate.
  * Revoke the progress preview URL and drop the processing geometry.
* After all candidates settle, if there was no explicit choice, select the argmax of `candidateRank` over all candidates. Failed captures stay on screen; **the scanner never reopens the camera automatically** (only Retake does).
* `reviewCandidate(i)`:
  * `waiting = pending > 0 && !(complete && corrected)`. `result = corrected && !waiting ? candidate.result : null`. State is `processing` or `review`. `#liveAddBtn.disabled = !result`, and its `textContent` becomes 'Add page' (embedded) or 'Save' (standalone). This replaces the icon span.
  * Status text: while waiting, the stage text; then 'Ready'; then the error.
  * Image URL priority is `resultUrl`, then `processingPreviewUrl`, then `url`. Images are decoded off-screen and swapped atomically. A corrected result triggers the `scan-reveal` class.
* Stage texts (`processingStatus`):
  * decoding, detection, dots → "Finding dots…"
  * surface, warp → "Straightening…"
  * colors → "Adjusting colour…"
  * vectors → "Tracing ink…"
  * template, encoding → "Finishing…"
  * when the result or an error exists → "Comparing scans…"
* `#liveProcessingSteps`: the active step is dots (decoding/detection/dots), surface (surface/warp), colors (colors/vectors) or template (template/encoding). Earlier steps get `data-complete=true`; the active one gets `aria-current=step`.
* `candidateRank` (`live-camera.js:866-900`):
  * Not processed: −1.
  * Uncorrected or `quality.valid===false`: `.1·f/(f+1)`, where `f = max(0, score)`.
  * Otherwise, `verifiedGeometry` requires `quality.valid===true`, a surface type of `developable-pinhole` or `measured-lattice`, `validation > 0`, a finite `pitch > 0`, and rms ≤ .14p, p90 ≤ .22p, p99 ≤ .4p and borderP95 ≤ 1.15p.
  * `registered` additionally requires `surface.registration` and (`rasterRegistration || registration`) with `anchors ≥ 350` and `maximumAnchorError < .01`.
  * Rank = `tier(2|1) + .25·registered + .4·cleanPaper + .17·geometry + .08·f/(f+1) + .03·clamp(confidence) + .02·exposure`, where:
    * `geometry` = `1 − .45·b(rms/p/.14) − .25·b(p99/p/.4) − .3·b(borderP95/p/1.15)` if verified, else .55.
    * `exposure` = `1 − b(|brightness−210|/150)`, or .5 when brightness is unknown.
    * `cleanPaper` = `1 − b(paperResidualFraction/.08)` when `paperSamples ≥ 100` and `confidence ≥ .5`, else .5.
* Processing overlay (`drawProcessingOverlay`, `live-camera.js:723`):
  * It draws only for the selected candidate, only while it is pending, only after the `dots` stage, and only on the image the geometry belongs to.
  * Before warp: measured dot centres ("observed" mode, from progress `dots` or `surface` details with `points`/`rows`/`columns`).
  * After warp: the corrected preview with warped observations ("corrected" mode). Faint dashed straight target runs are drawn in white at 60/40 alpha.
  * Row runs are drawn in `#f4d84b99` and column runs in `#70e0e680`, .8 px wide; dots are 1.4 px circles in `#fff4a7`.
  * Runs break at missing points; nothing is invented. `data-mode` and `data-point-count` are set. A `ResizeObserver` on the preview redraws.

### 3.9 Add, Retake, Close, Import
* **Add** (`addPage`, `live-camera.js:1006`): requires `result`, `state==='review'` and not submitting.
  * Standalone: download `previewUrl` as `scan.png` (ink layer) or `scan.jpg`.
  * Embedded:
    1. Set `submitting`, abort processing and disable Add, Retake and Close. Show "Adding…".
    2. Post `ihn-scanner-pages {pages:[result], documentName, stencil:false}`. If posting fails, show "The document is no longer available.".
    3. `ihn-scanner-result` resolves the submission: ok closes the scanner; otherwise show the message (default "Could not add the page. Your scan is still here; try again.") and re-enable.
  * A verified scan can be added while slower backups are still processing (pending work is aborted).
* **Retake** and **Open camera** both call `open()`.
* **Close** (button, or Escape in the capture phase with `stopImmediatePropagation`): `close()` returns false while submitting. Otherwise it stops the camera, clears the review (revoking every object URL), hides the dialog, restores inert flags and focus, sets state `closed` and then posts `ihn-scanner-close`.
* **Tab** is trapped within the visible, enabled buttons of `#liveScanner`.
* **Import** (`#liveImportBtn` → `#liveFileInput`, one file): stop the camera and clear the review. A non-image file gives `cameraError()` ("Camera unavailable"). An image becomes a single candidate `{blob:file, detection:null, score:0}` with no `requireGrid`, so the camera-only grid rule does not apply (§8.3).
* `#btnSourceCamera` and `#btnLandingStart` are rebound to `open` (`live-camera.js:1061-1062`). In embed mode the camera opens immediately (`live-camera.js:1099`). `SP.LiveCamera = {open, close}`.

---

## 4. Live detector (`scanner/processing/liveDetection.js`)

The detector is synchronous and has no OpenCV. Input is an RGBA `ImageData` of at least 40×40; the caller keeps it at ≤480 px. Output is normalised (0–1) geometry or null (`detect`, `liveDetection.js:975`).

### 4.1 Colour references (`findColourReferences`, `:117-261`)
This finds the four-disc footer independently of the yellow ink.
1. Downsample (nearest) so the longer side is ≤480 px.
2. For each pixel, compute a "white" as the channel-wise max of itself and 4 neighbours at `whiteReach = max(4, round(.017·min(w,h)))`. Skip it if that white's min channel is <15.
3. Compute ratios `r/wr, g/wg, b/wb`. Require `peak ≥ .12` and `peak − low ≥ .24`. Label the pixel red if `r > 1.3g, 1.3b`, blue if `b > 1.3r, 1.2g`, green if `g > 1.25r, 1.25b`.
4. Find 8-connected components of each label. Keep those with count ≥2, size ≤ `max(5, .025·min)` and aspect ≤3. Keep the top 32 per colour by compactness × count.
5. For each red–blue pair: `pitch = |rb|/2` must lie in [1.4, .025·min]. Green must sit within `.6·pitch` of `red + 1.5·(blue−red)`, and no disc may exceed `2.4·pitch²` px. Then:
   * **White**: sample 15×4 points across ±7 pitches and ±3–4 pitches away, skipping strongly coloured samples. Need ≥24; average the brighter half. Require light ≥32, min channel ≥15, and spread ≤13 % of light.
   * **Paper support**: 15 points at across ∈ {0, ±12, ±24} and away ∈ {8, 20, 36} pitches on the page-interior side. Each needs level .5–1.65 relative to the white and cast `max(|log rRatio|, |log bRatio|) ≤ .8`; when the cast is >.25, it also needs local smoothness (±18 % light, ±.12 log-ratio at ±1.5 pitch). Require at least 4 hits at |across| = 24 and `hits/15 ≥ .67`.
   * **Black**: the darkest pixel within `.35·pitch` of the red–blue midpoint must have light ≤ .45·white and each channel ≤ .65·white.
   * **Each disc**: after black/white normalisation (`(v−black)/max(12, white−black)`), the primary channel must be ≥ .18 and exceed the others by ≥ .12.
   * Score = `Σ compactness − error + min(1, pitch/4)`.
6. The best result is `{points:[red, black, blue, green] in source px, white, black, pitch, error, paperSupport}`.

### 4.2 Illumination normalisation (`estimateIllumination`, `:263`; `applyIlluminationRows`, `:371`)
* An optional `region` (normalised x0/y0/x1/y1) restricts the reference to the physical interior; rectified scans pass `{x0:.12, x1:.88, y0:.08, y1:.9}`. The sampling stride is `max(3, floor(max(w,h)/160))`.
* Candidate pixels need light ≥75 (≥32 when anchored by a region or references), min channel ≥38 (≥15), min ≥ .36·max (≥ .18·max), and a 4-neighbour (±1 px) gradient sum ≤ 42.
* Build a 64×64 histogram of `(round(24·r/g), round(24·b/g))` weighted by `(light/255)²` and take the peak over a 3×3 window. With references, the peak is instead `white` (`r/g·24`, `b/g·24`).
* Average the samples within ±.08 of the peak (weighted).
* Return null (no change) if:
  * the selected count is < 4.5 % of the reference sample count, or
  * the result is neutral (`|red−1|, |blue−1| < .085`), or
  * (no references) neutral pixels (both ratios within .075) number ≥ 8 % and are ≥12 brighter than the selected material.
* Otherwise build a coarse field of 48 px cells. Use samples within |log ratio| ≤ .17 of the global estimate, smooth with a 3×3 kernel (weights 4/2/1) plus the global estimate at weight 3, and interpolate bilinearly.
* Per pixel: `level = wr·.2126 + .7152 + wb·.0722`; `R = r·level/wr`, `G = g·level`, `B = b·level/wb`, keeping alpha.
* The result is `{width, height, data: corrected, luminanceData: original, illumination:{red, blue, coverage, reference:'colour-markers'|'paper'}}`.
* `normalizeIlluminationAsync` does the same in 48-row strips, yielding with `scheduler.yield` or `setTimeout 0` and honouring `signal` (AbortError "Scan cancelled"). Its output must be **byte-identical** to the synchronous version.

### 4.3 Masks (`analyse`, `:426`)
* Luminance always comes from the **unnormalised** pixels; `chroma = ((r+g)/2 − b − .34|r−g|)·gain`, where gain = `min(3, 255/max(50, refLight))` with references and 1 otherwise.
* `neutral = saturation < .28 && paperRatioError < .35`, where the error is the log-ratio distance to the reference white and 0 without references.
* `yellow = max > (refs ? 25 : 65) && g ≥ .70r && r ≥ .82g && b < .94g && chroma > 10 && sat > .055`.
* Paper thresholds:
  * `paperThreshold` = refs ? `max(32, .72·refLight)` : `max(95, P84(light) − 38)`.
  * `paperCeiling` = refs ? `min(255, 1.5·refLight)` : 255.
  * `scenePaperThreshold` = `min(250, max(95, P84 − 12))`.
* `layoutState` (`:793`) is the colour-independent mode, used only with the footer:
  * `chroma = (255 − L)·min(4, 255/max(40, refLight))·1.5`.
  * `neutral = L ∈ [.5, 1.65]·refLight`.
  * `yellow` = a thin luminance ridge (gap 2/3/4/6 px, contrast >3 horizontally or vertically) with `L ≥ .42·refLight`.

### 4.4 Candidates and scoring (`detectState`, `:843`)
* The search radius is `R = max(3, .065·min(w,h))`.
* Candidates:
  1. **Tracking**: the previous yellow quad and, if each of its 4 frame edges has 33 finite points, those edges as guides.
  2. **Yellow components**: if there are more than 60 yellow pixels, take the 4-connected components of the 3×3-dilated mask with ≥55 px (top 5 by size, boundary points from the undilated mask). Each gives a `corners()` quad (convex hull reduced to 4 by removing the smallest-area vertices, top-left first). With several groups, also add their union.
  3. **Paper**: for thresholds `[paperThreshold, min(250, +26), scenePaperThreshold if refs and > +8]`, take components of `neutral && L ∈ [threshold, ceiling]` with ≥12 % of the area. Each plausible quad becomes a paper candidate, plus 2 yellow candidates (paper rotations 0 and 1) inset to the template frame `rectangle(paper, 1.5/21, 1.43/29.7, 19.5/21, 28.43/29.7)`.
* `plausible` (`:59`): every point within [2, w−3]×[2, h−3]; area 12–93 % of the frame; min side ≥ .18·min(w,h); every corner sine ≥ .38; each pair of opposite sides with ratio ≥ .4; and (top+bottom)/(left+right) in (.38, 2.6).
* `paperEvidence` (24×32 interior samples at u .08–.92, v .07–.93): require neutral ≥ .64, yellow ≤ .2 and brightness ≥ (refs ? 32 : 75). Sharpness is the variance of a 4-neighbour Laplacian over the same samples.
* **Yellow candidate**:
  1. `edgeEvidence`: 32 samples per side, a hit if the ridge contrast is >5 within R (gap 3 in layout mode, else `max(3, .8R)`). Need ≥3 sides ≥ .4 and a max ≥ .7.
  2. `traceEdge` with 33 samples (`:569`): dynamic programming over ±`ceil(R)` normal offsets.
     * Observation = `signal − .16·|offset|/reach − (outer ? .015·offset : 0)`, where `signal = min(3, contrast/12)` (layout mode: `min(1.5, contrast/6)`). Contrast is the ridge contrast at separations {3, 6} (layout: {2, 3, 4, 6}).
     * "Narrow" means both sides are lower by >1 at separation 6.
     * On outer edges in layout mode, or with references plus illumination correction, a non-narrow sample's contrast becomes 0.
     * Transitions are limited to ±4 bands at a cost of .14 per band.
     * The chosen offsets are smoothed by `smoothOffsets` (`:527`): 3 robust passes of a second-derivative-penalised fit (λ = 18 if more than 20 samples, else 5), solved by banded Cholesky, with evidence weights `clamp(strength/18, .3, 1)`, or .03 when there is no evidence. Corners are anchored for outer non-tracking traces.
     * Outputs per side: `points`, `observedPoints`, `support` (hits/31), `narrowSupport`, `cornerOffset`.
  3. In tracking or layout mode, `trackedCorners` (`:654`) moves each corner by intersecting the fitted end displacements of its two adjacent traces. The move is rejected if the normals are near-parallel (|det| < .2) or the move exceeds 1.5R. Then retrace from the new corners.
  4. Gates:
     * ≥3 traces with narrowSupport ≥ .4.
     * `geometryPenalty = Σ cornerOffset/(4·min) ≤ .025`.
     * `edgeSupport` (mean) ≥ .59 (.5 when tracking), with ≥3 sides ≥ .5.
  5. `calibration` (`:701`): for each of the 4 rotations, trace the two box strips (x ∈ [.5/18, 8.5/18] and [9.5/18, 17.5/18], rails at v = 26/27 and 26.5/27 of the frame) along the curved frame surface (Coons-style `surfacePoint`), using 17 samples (34 in layout mode).
     * The two rails must be distinct: separation ≥ max(1.3, .45·expected), and in layout mode the gap to the frame bottom must be ≥ .7·separation.
     * Support = the minimum over both strips. The best rotation wins, and markers are reported when support ≥ .5.
     * In layout mode the footer must match (`footerOffset`, `:821`): the 4 reference discs must sit at frame coordinates `((8.625 + .25i)/18, 26.25/27)` within a common shift ≤ 3 pitches, each within `max(1.5, .7·pitch)`, and the footer must lie .65–5 pitches above the bottom rail. Layout mode also needs markerSupport ≥ .65 and min side support ≥ .45.
  6. `pageQuad` is the paper quad, or else the frame extrapolated to the A4 edge: `rectangle(oriented, −1.5/18, −1.43/27, 19.5/18, 28.27/27)`.
  7. `confidence = clamp(.49 + .31·edgeSupport + .13·markerSupport + .05·neutral, 0, .98)`, adjusted as follows:
     * Cap at .76 if min side < .2 and markerSupport < .5.
     * If markerSupport < .5, subtract `clamp((aspectError − .15)·.5, 0, .18)`, where `aspectError = min(|ln(aspect/(18/27))|, |ln(aspect/(27/18))|)`.
     * Cap at .76 if illumination was corrected and markerSupport < .5.
* **Paper candidate**: `paperBoundary` checks 15 points per side at a ±`max(3, .013·min)` gap. Require support (inside brighter by >12) ≥ .65 and mean contrast ≥ 15. Then `confidence = clamp(.38 + .21·support + .09·neutral, 0, .69)`.
* For all candidates, cap confidence at .76 if brightness < 75. Score = `confidence + min(.04, .06·coverage) − 2·geometryPenalty`; the highest score wins.
* In layout mode, a tracked candidate with conf ≥ .94, edge ≥ .9 and marker ≥ .85 returns early.
* Result fields:
  * Geometry (normalised, top-left first; frame edges rotated to match): `quad`, `pageQuad`, `markers[2 quads]`, `markerEdges`, `frameEdges[4×33]`, `contour`.
  * Scores: `confidence`, `source` ('yellow'|'paper'), `frameEdgeSupport[4]`, `edgeSupport`, `markerSupport`.
  * Mode: `tracking`, `detectionMode` ('colour'|'layout'), `illumination?`.
  * Image measures: `sharpness`, `edgeSharpness` (Laplacian variance in 3×3 around the frame edge points, `railSharpness`, `:750`), `brightness`, `coverage` (quad area fraction).
* `detect` (`:975`): if the previous detection was in layout mode and references exist, try layout first and return it if it is yellow with ≥ .86. Otherwise run colour mode and return it if ≥ .86 or there are no references. Otherwise try layout and return it if it is yellow with higher confidence than the colour result.

Exports: `SP.LiveDetection = {detect, normalizeIllumination, normalizeIlluminationAsync, findColourReferences}`.

---

## 5. Overlay

The live overlay and its motion filter are specified in §3.5; the review-time processing overlay is specified in §3.8.

---

## 6. Capture quality preflight (`scanner/processing/capture-quality.js`)

* `readSource`: grey (Rec.709 luma) of the still scaled so the longer side is ≤1100 px.
* **Sharpness** (`inspect`, `:23`):
  * At each point, fine = (±1 px central difference)² and broad = (±3 px)², used only if `fine + broad ≥ 144`.
  * Interior: a 2-px grid inside the quad (6 px inset), excluding a band within `.045·(left+right side length)` of the bottom edge (the footer). Points are binned into 3×3 zones over the quad's bounding box. A zone is measured if it has ≥12 samples and `fine > .02·broad`; its ratio is fine/broad.
  * `interior` = the 25th percentile of the zone ratios when ≥3 zones are measured.
  * `border` = Σfine/Σbroad over 5×5 neighbourhoods of all `frameEdges` points, requiring ≥60 samples.
  * `clarity = min(border, interior)`. `valid = border ≥ .32 && (interior null || interior ≥ .32)`.
* **Grid** (`inspectGrid`, `:75`):
  * Rectify the page to 540×810 (810×540 if the quad is landscape) through a Coons patch of `frameEdges`, or the quad if edges are missing. More than 1 % of samples outside the image gives `page-clipped`.
  * Use a 9×9 local mean from an integral image (11 px margin). A pixel is dark if `mean − v ≥ max(4, .022·mean)`.
  * Take 8-connected components with ≤45 px, bounding box ≤8 px, aspect ≤2.5 and peak deficit ≥6; their weighted centroids are the dots. More than 5500 dots gives `page-texture`.
  * Link each dot to its best neighbour in 4 directions at pitch 15: major axis 65–135 % of the pitch, minor ≤ .32·pitch, error `|major − pitch| + 1.5·minor`. Keep reciprocal links that form complete cells with opposite-side mismatch ≤ .25·pitch.
  * `valid = cells ≥ 160 && matched ≥ 400 && all 9 zones have ≥12 matched && rowBands ≥ (32 portrait | 21 landscape) && columnBands ≥ (21 | 32)`.
* `inspectReadiness` (`:185`) requires sharpness valid **and** `clarity ≥ .42` **and** `interiorMinimum ≥ .32`; otherwise the reason is `focus-or-motion-blur`. Then the grid: reason `sharp-grid`, or `grid.reason || 'grid-not-found'`. The result object is `{valid, reason, clarity, border, interior, zones, edges, interiorMinimum, grid}`.
* `inspectReadinessAsync` (`:209`) runs in one shared worker (`capture-quality-worker.js`):
  * The frame is sent as an `ImageBitmap` (transferred).
  * One request at a time; a second concurrent request returns `{valid:false, reason:'readiness-busy'}`.
  * A 2000 ms timeout terminates the worker. Abort terminates the worker and rejects with AbortError.
  * When the worker is unavailable, it falls back permanently to the synchronous `inspectReadiness`, which must give the same numbers.
  * `disposeReadiness` bumps the generation and terminates the worker.

---

## 7. Detection-worker contract

`processing/detection-worker.js`: `self.document.createElement('canvas')` returns `new OffscreenCanvas(1,1)`, and all processing scripts are imported. Message `{operation, bitmap|blob, config, liveDetection, maxDimension, timeoutMs, previewProgress, reportProgress, surfaceBudgetMs, requireGrid}`:
1. Set `ScannerPro.Config = config` and await `InhouseRust.ready` (never start the synchronous 3D solver before WASM resolves).
2. `operation === 'process'`:
   * If `OffscreenCanvas.convertToBlob` or a 2D context is unavailable, reply `{unsupported:true}`.
   * Otherwise reply `{progress, detail}` messages followed by `{result}`.
3. Otherwise run `detectPage(bitmap, …)` and reply `{detection}`.
4. Errors reply `{error, name, needsImageDecoder}`. The bitmap is always closed.

`Light.processPageAsync` (`lightweight.js:3929`) creates **one worker per photo**:
* It falls back to the in-page `processScanBlob` when Worker, `createImageBitmap` or `OffscreenCanvas` is missing, on worker error, on `unsupported`, or on `needsImageDecoder` (for example SVG).
* Timeout: `clamp(timeoutMs || 20000, 1000, 60000)`, with "Page correction took too long. Try another frame.".
* An external abort rejects with `DOMException('Scan processing cancelled', 'AbortError')` and terminates the worker.
* `Light.detectPageAsync` (`:3550`) is the detection-only equivalent ("Page analysis took too long. Try another frame.").

---

## 8. Still pipeline (`processScanBlob`, `lightweight.js:3745`)

Constants (`lightweight.js:6-12`): `A4_RATIO = 21/29.7`, `ANALYSIS_FAST_MAX = 608`, `ANALYSIS_MAX = 800`, `OUTPUT_LONG_EDGE = 2546`, giving a default output of **1800×2546 px** (≈85.7 px/cm, 8.571 px/mm). `yieldToBrowser` is a no-op inside a worker; on the UI thread it is `scheduler.yield` or `setTimeout 0`.

| # | Stage (progress name) | Input → output | Notes |
|---|---|---|---|
| 1 | `decoding` | Blob → `ImageBitmap` (or `Image` on the main thread) | In a worker without an image decoder for the format, the error carries `needsImageDecoder`. An empty image throws "This image is empty". |
| 2 | — | bitmap → `source` canvas, longer side ≤2560 | `createScaledCanvas` uses `imageSmoothingQuality` "medium". The bitmap is closed. |
| 3 | `detection` | source → detection (§8.1) | Via `detectPageAsync` with `surfaceBudgetMs = 7800` in a worker (otherwise the 2600 ms default). Progress `dots` and `surface` details pass through. |
| 4 | gate | — | `precise = method ∈ {stencil, marker-guided} && confidence ≥ .55`. Quad = `detection.pageQuad` if precise, else the live seed's `pageQuad`. If `quality.valid===false` or there is no quad, return the original with `corrected:false, failureReason: quality.reason || "Page edges could not be verified"`. |
| 5 | grid gate (`requireGrid`) | — | §8.3 |
| 6 | `surface` | — | `{verified, diagnostics}` |
| 7 | `warp` | source + frame → output A4 canvas (§8.4) | `onRegistration` emits `surface {registered:true, …}` |
| 8 | certificate | output → `dotAlignment` (§8.5) | Only when precise |
| 9 | `warp` (preview) | output → 720 px JPEG .9 preview with the native template overlaid (`background:false`) when precise; detail = warped observations (`space:'corrected'`) plus `previewBlob` | Only with `previewProgress`. A failure here is ignored. |
| 10 | `colors` | output → cleaned RGB, `inkCoverage`, `outputQuality`, `stencilCleanup` (§8.6) | Precise only |
| 11 | — | `separateScanInk` (§8.7) | Precise only |
| 12 | `vectors` | ink RGBA + coverage → `editableInk` (§9). The output is then **cleared and re-rendered with exactly those native pens**. | Precise only |
| 13 | `template` | preview = native template (white) + output drawn on top | Precise only |
| 14 | `encoding` | Output as PNG (precise) or JPEG .94; preview as PNG. The encoded output is decoded again and its dimensions must match ("The scan could not be verified"). | |

The result is `{blob, width, height, corrected:true, previewBlob, editableInk, inkLayer: precise, templateKind: precise ? 'default' : undefined, templateApplied, stencilCleanup, outputQuality, dotAlignment, detection, timings}`. `timings` contains `{worker, decodeMs, detectionMs, warpMs, alignmentMs, previewMs, colorMs, colors{backend, pixelsMs, cleanupMs}, vectorMs, templateMs, encodeMs, totalMs, recoveryMs?}`. Every cancellation checkpoint throws AbortError, and all canvases are zeroed in `finally`.

### 8.1 Page detection (`Light.detectPage`, `lightweight.js:3434`)
1. If there is no `liveDetection` (imports), compute one with `LiveDetection.detect` on a ≤480 px copy.
2. **Quick pass**: `detectPageAtScale(source, 608)`; with `forceDetailed`, 800.
3. A detailed pass is needed if any of these holds:
   * the quad is not finite;
   * the method is `fallback`;
   * the method is `paper` with conf > 0;
   * the method is `marker-guided` and any of: conf < .96, frame support < .42, box support < .42, curvature > .03, `blackSupport` < .05;
   * the method is `stencil` and any of: conf < .72, support < .78, (box < .72 and support < .9).
4. **Shortcut**: if a detailed pass is needed but the quick result is valid, marker-guided, conf ≥ .98, support ≥ .88, box ≥ .72 and all sides ≥ .7, finalise a deep copy without the lattice fallback. If it yields a metric surface, use it (`scaleSelection.reason = 'verified-full-source-surface'`).
5. **Detailed pass** at 800. Keep the coarse result instead ("degraded") when the coarse result had complete evidence (marker-guided, support ≥ .9, box ≥ .85, and either coarse quality not invalid or detailed quality not valid) and any of these holds:
   * the detailed result is not marker-guided and its support ≤ coarse + .03;
   * the coarse weakest side ≥ .94 and the detailed weakest side < coarse − .04;
   * `measureFrameEvidence` (both pages re-measured at 1200 px, 61 samples per side, ridge test with ±3 px shift) scores the detailed result < coarse − .06;
   * the coarse result is valid and the detailed result invalid.
6. `finalize(selected)`:
   1. `refineObservedFrame` (§8.1.3).
   2. `SurfaceParameterization.refine` (§8.2.1).
   3. Propagate layout mode.
   4. **Seed agreement**: compare the refined frame paths with the live `frameEdges` using `contourDistance` (point-to-polyline distance, normalised by the image diagonal). If max > .025 or mean > .009, mark invalid with `refined-contour-disagreement`.
   5. If valid, fit the metric surface (§8.2). In `forceDetailed` mode try `fitLattice` first; otherwise `fit`, then `fitLattice`. On success set `frame.metricSurface = surface`; otherwise record `frame.surfaceDiagnostics`.

#### 8.1.1 `detectPageAtScale` (`:2745`)
1. Scale the source to the analysis size and build the **detection image** (`detectionImage`, `:38`): illumination-normalised (§4.2). If the 90th-percentile luminance is <112 and a footer white of 32–111 exists, also apply a gain of `min(3, 152/paperLight)`. This disposable copy is used only for geometry.
2. `observedLiveFrame` (`:2662`) re-measures the live seed's 4 edges on this photo:
   * The seed must be yellow with conf ≥ .72 and edges of ≥8 points.
   * At each point, search ±6 px (0.5 px steps) along the normal for a two-sided yellow-chroma ridge (centre ≥9, contrast ≥3) **or** a luminance ridge `≥ max(2.5, .012·paper)`. The score is penalised by `.65·|offset|`.
   * Require min side support ≥ .58 and ≥3 sides ≥ .78. Corners are averaged from the adjacent arms, and the quad must be plausible (≥12 % area).
3. `detectPaperQuad` (`:506`): candidate pixels have L ≥ 72, sat ≤ .27, g/r ≥ .76 and b/r ≥ .67. Dilate separably with radius 3, then flood-fill from seeds near the centre (radius up to 24, up to 6 seeds). Keep the largest component (>8 % of the area) with ≥40 boundary points. Take its convex hull and extreme corners (by x+y and similar). It must be plausible with area ≥22 % and opposite-side ratio ≤ 2.25.
4. `estimatePaperYellowBaseline` (`:115`): a 13×13 interior sample (u .17–.83, v .13–.81) with L ≥ 92 and sat ≤ .42; take P55 chroma, P55 sat and P62 L. Defaults are `{chroma 3, sat .025, L 232}` when there are fewer than 18 samples.
5. **Yellow mask**:
   * Strict: `isYellow` (hue 39–68°, max ≥82, sat ≥ .16, g/r ≥ .70, b/g ≤ .84, (r+g)/2 − b ≥ 20) and `hasLocalYellowRidge` (a chroma drop of ≥2 at radius 3 or 5 on both sides, horizontally or vertically).
   * Faint (only inside the paper): `isFaintYellow` (hue 27–88°, max ≥54, sat ≥ max(.032, baseline − .012), chroma ≥ baseline + 3.2, opponent ≥6, |r−g| ≤ 1.45·opponent + 9, g/r ≥ .58, b/g ≤ .97) plus `hasLocalStencilContrast`, which only applies inside the template reference bands.
   * The trace mask additionally needs paper on both sides.
   * When the live seed edges are under-covered (<70 % within 3×3), add pixels along them (±2 px) that form a two-sided luminance ridge as strength 3.
6. **Marker-anchored candidates** (`markerAnchoredPaperCandidates`, `:1724`):
   * Find chromatic components (`chromaticMarkerKind`, `:1635`): red `r−g > 10, r−b > 8, r > 1.22g, r > 1.16b`; blue `b−r > 7, b−g > 4, b > 1.05r`; green `g−r > 6, g−b > 10, g > 1.04r`.
   * Build ordered red/(black)/blue/green sequences at the canonical x 10.125/10.625/10.875 of 21 and y 27.68/29.7, which yield affine paper quads.
   * For the first candidate, run a second narrow-band pass: count chromatic evidence in the reference bands (threshold `max(420, .0011·area)`). If it is sparse and the neutral strip support is ≥ .1, enable **neutral recovery**, which promotes grey ridges (`isNeutralStencilTone`) to strength 3. With strong chromatic evidence but ≤2 visible rails (rail support ≥ .34 over 48 samples), skip the band pass. Recompute the candidates.
7. **Axis-aligned fallback**: take the first and last mask pixels per row and column inside the paper (u, v ∈ [.008, .992]). Fit `ransacLine` per side (tolerance `max(2.1, .0055·min)`, span ≥30 %) and intersect them. If plausible (top and bottom widths >35 % of w, side heights >42 % of h, area ≥16 %), `confidence = min(1, yellowPixels/(.004·area))`.
8. For each marker attempt (marker-affine quads, plus the paper quad), run `detectMarkerGuidedStencil` (`:2218`):
   * Choose the rotation by strip score (yellow points with v .72–1.12) plus 7 per marker hit (up to 6 per component) plus 45 per distinct marker kind.
   * Fit the strip, marker axes and side, top and bottom lines with RANSAC.
   * Result: `{stencilQuad, neutralGuides, confidence: max(neutral ? .94 : 0, clamp(.58 + support/max(900, 2.2·N) + min(.14, .01·markerHits) + (black ? .04 : 0)))…, rotation, calibration{…markerAxes, topLine.source 'chromatic'|'neutral-recovery'}}`.
   * Then `traceYellowFrame` (§8.1.2) is retraced up to twice while it moves ≥ .7 px and its support does not drop by > .08 (not in neutral recovery).
   * Score = `2·support + 1.25·boxSupport + .45·conf (+ .02 for marker-affine)`.
9. With a live seed, add an `observed-contour` attempt: rotate the seed so the strip is at the bottom, trace, resample each seed edge to 25 points, set confidence = min(.99, support) and score it the same way. **Every** attempt whose seed agreement exceeds max .025 or mean .009 loses 4 points.
10. `reliableAttempt`:
    * The seed must agree, and the extrapolated page must lie within the image with an 8.5 % margin.
    * **Ordinary frame**: ≥3 sides ≥ .42, support ≥ .54, box ≥ .42 and ≥40 evidence points.
    * **Chromatic occlusion exception**: marker-affine, top line chromatic, a complete red/blue/green axis, blackSupport ≥ .004, conf ≥ .72, ≥2 sides, support ≥ .42, box ≥ .58 and ≥40 evidence points.
11. Result precedence:
    1. **marker-guided**. `pageQuad` = the extrapolated stencil if all 4 sides are supported, else the observed paper. Quality is valid with reason `observed-contour-agreement` or `frame-and-calibration`.
    2. **stencil** (conf ≥ .24) traced with refinement. Quality is valid if the extrapolated page fits the image and (with a seed) the contour agrees.
    3. **paper** (conf .35 if yellow pixels ≥ .045 % of the area, else 0).
    4. **fallback**: the largest centred A4-ratio rectangle.
    * In `finish`, if a yellow seed existed but no live frame was re-observed, the result is valid only as `independently-observed-contour` (marker-guided, agreeing contour, re-measured side supports with min ≥ .58 and ≥3 ≥ .78); otherwise `live-contour-not-observed`. A missing quality object becomes `page-frame-not-verified` (invalid). `analysisTimings` holds the per-phase milliseconds.

#### 8.1.2 Frame tracing (`traceYellowFrame`, `:1144`)
* Map the mask pixels into stencil `(u, v)` with `invertBilinear` (7 Newton steps), keeping −.22…1.22. At least 48 points are required.
* With refinement allowed, `refineStencilQuad` (`:900`) runs a 196² accumulator arm search. If the quad moves ≥1.25 px, retrace from the refined quad.
* Each side uses `traceFrameSide` (`:676`), 25 samples:
  * Collect points within `maxDeviation` of the expected line in windows of `.72/(n−1)`.
  * Candidates are clusters (gap ≤ .009, ≥2 points; median value, strength ≤36) plus a local guided RANSAC line (tolerance .0044, slope ≤ .58), then de-duplicated within .004.
  * Choose one continuous ridge by dynamic programming with node cost `80·|v − expected| − strength`.
* Deviations: border .07 (.09 trusted); sides also try a wide corridor of .15 (.17 trusted), accepted only for a coherent inward shift (support ≥ .55, median > .022, ≥62 % inward, ≤16 % outward). A "safe top" alternative is used when it is shifted inward. In neutral recovery the top deviation is ≤ .045.
* Lower rails (`traceLowerStencilRails`, `:999`): the box top (v = 1 − 1/27), box bottom (1 − .5/27) and frame bottom (1) are traced jointly. Box rails exist only for u inside [(2−1.5)/18, (10−1.5)/18] or [(11−1.5)/18, (19−1.5)/18]. Deviation is .032 for box rails and .052 for the bottom.
* Rejected if fewer than 2 sides have support ≥ .26 or the mean support is < .2 (`:1321`).
* Output (source px; paths top and bottom run left→right, left and right run top→bottom; 25 points each):
  * `paths{top, right, bottom, left}`, `corners`, `refinedStencilQuad`.
  * `box{top, bottom, outer = paths.bottom, points, normalized, support = mean(boxTop, boxBottom)}`.
  * `evidence{points, normalized}`: real mask pixels within .014 of a fitted rail, at most 2200, used by the legacy animation.
  * `sideSupports[4]`, `support`, `confidence = clamp((support − .16)/.68, 0, 1)`, `curvature` (max deviation from the chord), `samples`, `interiorTrace`.

#### 8.1.3 Source-ridge refinement (`refineObservedFrame`, `:3307`)
This runs only if `frame.confidence ≥ .6`, on a ≤1200 px detection image.
* For each interior path point (skipping 2 at each end), test normal shifts of ±6 px in 0.5 px steps (±4 for box rails when box support ≥ .5). The score is the sum over ±5 tangent samples of `max(0, s(c) − (s(c+4n) + s(c−4n))/2)`, where `s` = yellow chroma for yellow pixels, `.22·(255 − mean)` for pixels with sat < .16, and 0 otherwise.
* If the best score is ≥20, use the centroid of the shifts within 2.5 of the best, weighted by `score − .35·best`.
* Sets `frame.refinement = 'source-ridge'`.

### 8.2 Surface model
#### 8.2.1 Perspective and anchor parameterisation (`surface-parameterization.js:136`)
* Requires `box.support ≥ .5`, finite paths of ≥4 points, and a perspective model `(g, h)` from the 4 frame corners with `min(1+g, 1+h, 1+g+h) ≥ .3`.
* On a ≤1200 px illumination-normalised copy, compare linear vs projective parameterisation using:
  * the 5 mm tick contrast inside the boxes;
  * the rail contrast at v = 26/27 and 26.5/27;
  * the rail-to-border **gap error**.
* `usePerspective` if any of:
  * the projective gap < linear − .0005 and < .65·linear;
  * tick gain > 7 with projective ticks > 8 and rail gain > −8;
  * rail gain > 7 with projective rails > 6 and tick gain > −5.
* `markerAnchors` (`:25`, only when both side borders are straight within .4 % — `verticalRuling`):
  * Sample 2049 points across the boxes and find the red/blue/green centres (strength ≥18) in order, with spacing .012–.05 and b/g ratio near 2/3 (±.16).
  * Find the tick peaks and number them outward from the markers (cells 16/36 and 20/36), stepping ±1 cell with ±28 % tolerance.
  * Require ≥29 anchors spanning u < .09 … > .91.
  * Each anchor's ruling (toward the vanishing point of the two side borders) is intersected with every guide curve; the curves are rebuilt piecewise-linearly through the anchors at 65 samples.
* Result `frame.parameterization = {model: 'measured-surface'|'projective-guides'|'observed-guides', g, h, linear, projected, anchors}`. Paths and box rails are resampled to 65 points.

#### 8.2.2 Dot observation (`MetricSurface.observe`, `metric-surface.js:366`)
1. Work on a ≤1000 px copy. Response = broad blur − σ.55 blur of luminance.
2. Estimate the pitch from the frame quad (frame/36 horizontally, /54 vertically). It must be 5–30 px.
3. **Candidate dots**:
   * Response ≥5 (faint pass ≥1.7), a 5×5 local maximum, channel max ≥32.
   * Relative darkening versus a 7×7 background (`(bg − v)/max(40, bg)`): strongest ≥ .014, weakest ≥ .36·strongest − .012, spread ≤ .56·strongest + .016.
   * ≥6 of 8 ring samples at 3 px brighter by > .3·response.
   * Inside the frame polygon, above the **visible** calibration-top rail (a rail is used to exclude only if ≥35 % of its segments show colour or layout evidence), and ≥ .42·pitch from every frame path.
   * Weighted 3×3 centroid. Drop points with strength >2.3× the median.
4. Need 350–4500 candidates. With fewer than 350, retry once in **faint mode**.
5. Build a nearest-neighbour lattice graph (both direction-field hypotheses), bridge single gaps, take the largest component, assign `(col, row)` and choose the **phase** `(c0, r0)` by distance to the 4 frame borders.
   * Each side's evidence must be ≥12 (≥3 faint); both lateral sides plus ≥3 or 4 sides must be supported; the total must be ≥80.
   * The best phase score must be ≤ .65 and beat the second by ≥ .25 (unless faint mode measured a complete 35×51 extent).
   * Extent ≥ 31 columns and ≥ 46 rows (40 in faint mode).
6. Remove pencil impostors (duplicates; points >.22·pitch off a quadratic through ≥4 same-row/column neighbours that themselves fit within .08·pitch). At least 350 must remain, and ≥85 % of measured (60 % faint).
7. `refineDotCenters` (`:969`): on a ≤2600 px raster, compute the weighted centroid inside radius `clamp(.2·pitch, 2.8, 6)` against a 16-sample ring. In faint mode only confirmed centres train the fit.
8. The output is `{points[{x, y, u, v, row, col, centerRefined, dotContrast}], raster, pitch (source px), phaseScore, coverage = n/1785}`. Each dot sits at `u = (20 + 5·col)/210`, `v = (19.3 + 5·row)/297`.

#### 8.2.3 Developable pinhole fit (`MetricSurface.fit`, `:1701`)
* **Model** (`prepare`, `:70`; `projectPhysical`, `:115`): parameters `p = [rx, ry, rz (axis-angle), tx/100, ty/100, ln depth, ln focal, β (bend direction), k knot angles]`, with k = 8 (coarse) or 12 (fine). A fixed 0 knot is inserted in the middle.
  * The sheet's curve tangent angle is interpolated across the knots. Arc length `L = (|cos β|·210 + |sin β|·297)/2` is integrated in 128 steps (trapezoid on cos and sin), centred at the middle.
  * A page point `(x, y)` in mm becomes `s = x·cosβ + y·sinβ` (along the bend), mapped to `(curve x, curve z)`; then rotated, translated and projected with the focal length around the image centre.
* **Initial guess** (`initial`, `:188`): a homography from the 4 frame corners (15/195 × 14.3/284.3 mm), focal `1.4·max(w,h)`, decomposed into a rotation and translation.
* **Data split**: training = rows with `row % 4 === 1` or columns with `col % 4 === 2`; everything else is validation. Positional samples are ≤260 evenly spread training dots. Parallel guides (`traceParallels`, `:1062`) are local quadratic tangent samples along measured rows and columns, ≤40 per family, from training dots; held-out guides come from validation dots.
* **Residual** (`:1170`):
  * dot position errors;
  * a knot second-difference penalty ×.5;
  * 4×25 frame border samples to the nearest path point ×.35;
  * guides: position ×.65, tangent ×1.7.
  * Loss = `Σ 2(√(1+r²) − 1)`.
* **Optimiser** (`optimize`, `:1314`): Levenberg–Marquardt with IRLS weight `1/√(1+r²)` and a forward-difference Jacobian (ε 1e−4), λ starting at .01 (×.35 on success, ×5 on failure).
  * Each step is clamped to ±.4. Translation is clamped to ±5; depth to ln[120, 4000]; focal to ln[.5, 5]·max(w,h); knots to ±1.5.
  * Up to 48 iterations (20 for the fine profile); stop when the improvement is <1e−5. `preserveEnds` freezes the first and last knot.
  * **Rust** `surfaceFit` is used when available (`rust-core-v5.js:111`; `surface.rs`), stepping one iteration per call until the deadline.
* **Starts**: β ∈ {0, π/2, π/4, −π/4}, with knots initialised to `((i or i+1) − 4)/4·.25`. A valid start beats an invalid one, then lower cost wins. Stop early when the validation rms < .025p, p99 < .065p, tangent P90 < .035p and the model is valid and visible.
* **Validation**: `validErrors` (`:1284`): rms ≤ .14p, P90 ≤ .22p, P99 ≤ .4p, border median ≤ .6p, border P95 ≤ 1.15p. `visibleSurface` (`:1293`): the Jacobian sign is constant and non-degenerate on a 13×17 grid. Held-out tangent P90 must be ≤ .22p.
* **Fine profile**: with ≥32 guides and ≥180 ms left, interpolate 12 knots and optimise with preserveEnds false, then true. Accept a candidate that validates and (if the coarse fit is valid) costs <98.5 %. Also try 25/50/75 % blends of the two in parameter space.
* **Measured refinement** (`fitMeasuredRefinement`, `:1464`): a smooth cubic B-spline 2D residual field with 9×13 controls, fitted on the training dots plus rail anchors and bounded by `boundRefinement` (displacement ≤ .22p; the global gain is backed off, rejected below .35). It is accepted only if validation rms < .88×, P90 < .93×, P99 ≤ 1.03× and tangent no worse than ×1.03.
* **Missing-dot recovery and registration**:
  * Predict the missing sites and re-measure them. A recovered dot needs contrast < 2.3× typical and must lie within `max(.25, min(.08p, 1.2·P90))`.
  * `refineProjectedCenters` (`:1540`) then `registerMeasuredDots` (`:1598`) build an exact-anchor displacement field on a (35+4)×(51+4) knot grid: page edge, frame edge, 35/51 dot positions, frame edge, page edge.
  * Candidates must lie within .3p of the model and agree with both dot families (±max(.35, .06p)). Missing interior controls are Laplace-filled; the 2 outer rings are fixed.
  * Up to 5 attempts remove the worst control per bad cell, where a bad cell has a local scale shift >12 % or a fold. The minimum is `max(350, 35 % of points)`.
* **Result**: `{type:'developable-pinhole', parameters, refinement?, registration?{colPositions, rowPositions, dx, dy, points}, width, height, diagnostics{observations, registration{anchors, rejected, interpolated, localScaleLimit, maximumAnchorError, recovered}, training, positionalSamples, validation, parallels, parallelValidation, measuredRefinement, profileKnots, coverage, phaseScore, rms, p90, p99, borderP95, pitch, elapsedMs, iterations, backend:'rust'|'javascript', starts}, profile{arcLengthMm, depthRangeMm, bendAxisRadians}}`. The budget defaults to 2600 ms and is 7800 ms in a worker.
* **`fitLattice`** (`:1976`) handles creases that one bend cannot explain:
  * Model = Coons patch of the frame paths (`coons`, `:337`) plus a recovery refinement field (13×19 controls, weaker smoothing, bound 3p).
  * Needs ≥500 observations, ≥200 training and ≥300 validation points, `validErrors` and tangent P90 ≤ .22p.
  * Recovers missing dots with a 3×3 offset search (±.18p), refits on all points if rms does not worsen, then registers.
  * Type `measured-lattice`.
* **`map(surface, u, v)`** (`:2445`) evaluates `project`: Coons or physical, plus refinement, plus registration. `preserveLatticePerimeter` slides the result tangentially onto the measured rails inside the blank half-cell margin, tapered at the corners.

### 8.3 Camera grid requirement (`processScanBlob:3788-3814`)
With `requireGrid` (every camera burst frame), all of the following must hold: `precise`, a surface type of `developable-pinhole` or `measured-lattice`, `observations ≥ 350` and `validation > 0`. If they fail but the pass was precise, not `forceDetailed`, not already at 800 px, and `surfaceDiagnostics.candidates ≥ 500`, **re-run the whole pipeline once with `forceDetailed`** (progress `detection {retry:true}`). Otherwise return the original with reason `page-dot-grid-unverified`: "The page dots are not clear enough to straighten. Try another frame.". Imports skip this gate.

### 8.4 Warp (`Light.warp`, `lightweight.js:4003`)
* Output: height `clamp(maxDimension || 2546, 256, 2546)` and width `round(height·21/29.7)`, prefilled with white.
* `curved = frame.paths && frame.confidence ≥ .18`. The mesh has 64 columns × 80 rows of fractions plus extra knots:
  * u: frame edges 1.5/21 and 19.5/21, plus the registration column positions;
  * v: 1.43, 27.43, 27.93 and 28.43 /29.7, plus the registration row positions.
  Every registered bullet is therefore a mesh vertex.
* With registration, `MetricSurface.refineForRaster` (`:2310`) first re-measures each anchor's glyph centroid **in the exact sampling of the warp** and adjusts the anchors. It emits `surface {registered:true}` detail and caches the result per surface and output size.
* The mapping is `mapFramePoint` (`:1548`): the metric surface if present, else the guide-blend interpolation of the frame curves, using the box rails (when box support ≥ .25) tapered with a smoothstep over the last 0.5 cm at the sides. When not curved, the mapping is `projectivePageMap(pageQuad)` (`:3985`).
* **Inverse mapping**: each output pixel is sampled once. Within each cell, the left and right edges are interpolated in v and the sample steps linearly in u. Sampling is bilinear at pixel-centre conventions (`sx = … − .5`); out-of-image samples become white.
  * When curved, cells outside the frame (u < 1.5/21, u > 19.5/21, v < 1.43/29.7, v > 28.43/29.7) are **left white**.
  * A non-finite node throws "The page surface could not be reconstructed. Try another frame.". A folded cell (cross-product sign versus the quad orientation) throws "The page surface overlaps itself. Keep the whole sheet visible and try another frame.".
* **Rust** `warp_cell` (`rust-core/src/lib.rs:41`) performs the per-cell loop; JS falls back to the identical loop. Rounding is `round_ties_even` in Rust and the Canvas `Uint8ClampedArray` rounding in JS, which is also ties-to-even.
* `output.scanSurface = {model: 'curved-guided-surface'|'projective', columns, rows, registered}`. `onMesh({us, vs, grid})` feeds `warpedObservations` (`:3646`), which inverts each observed dot into output pixels by Newton iteration over the exact cell (7 iterations, error ≤ .1 px); runs break at missing points.

### 8.5 Raster certificate (`MetricSurface.certifyRaster`, `metric-surface.js:2062`)
* This runs on the **warped photo before colour correction**, at ≤1100 px, with `pitch = width·5/210`.
* For each of the 1785 template sites, search ±.55p for a compact dark glyph (radius .18p, background P65 of a .72p ring, a neutral-darkening test like observe's). Record `dx, dy, error, contrast`, deduplicated at .18p.
* Drop points with contrast > `max(.12, 2.5·median)`. Bin into 5×6 tiles; a tile is supported with ≥6 points.
* **Regularity**: neighbour pairs at .55–1.45p along with < .75p across; "regular" if across < .35p.
* `evidence = points ≥ 240 && supported tiles ≥ 10 && pairs ≥ 160 && regularity ≥ .6`.
* `failed = evidence && (P90 of per-tile median offsets > .06p || P90 of errors > .16p || fraction with error > .14p > .2)`.
* Status: `verified`, `failed` or `not-applicable` (reasons `photographed-dots-aligned`, `photographed-dots-misaligned`, `insufficient-visible-dot-pattern`).
* **Consequences** (`processScanBlob:3827-3856`):
  * `failed`, or anything other than `verified` under `requireGrid`: if the detection came from the `verified-full-source-surface` shortcut and the pass is not `forceDetailed`, re-run once with `forceDetailed`. Otherwise return the original:
    * `failed`: reason `interior-dots-misaligned`, "The page dots could not be aligned. Try another frame."
    * other: reason `interior-dot-grid-unverified`, "The corrected page dots could not be verified. Try another frame."

### 8.6 Colour and illumination (`Light.correctColors`, `lightweight.js:5283`)
Called with `{useStencil:true, preciseStencil:true, dotObservations: warped dots, captureInkGeometry:true}`.
1. **Global cast**: `LiveDetection.normalizeIllumination` with region `{x0:.12, x1:.88, y0:.08, y1:.9}`. The async strip version runs on the main thread; the sync version runs in a worker.
2. **Observed paper illumination** (`observedPaperIllumination`, `:4955`):
   * Around each verified dot (and recovered sites in the rectified raster), sample paper rings at 1 and 1.8 mm (24 samples each) and keep the lighter half, giving luminance, red/green and blue/green ratios.
   * Grey-dot absorption ratios give neutral anchors; a local majority of isolated dots is required.
   * Needs ≥80 anchors, otherwise null and the fallback field is used.
   * Builds a ≤512 px-wide field, smoothed (closing radius 2, no blur) and continued across drawings. It also returns a `drawing` protection mask (0–255), `neutralConfidence`, `white` (when the max cast > .085 or a neutral calibration exists), `paperPoints`, and `noiseSigma`/`noiseFloor = clamp(2.2σ, 2, 6)`.
3. If a `white` field exists, `balanceObservedPaper` (`:5241`) applies per-pixel bilinear white ratios as in §4.2. With neutral grids it also rescales each channel's ink depth about the paper (`paper − (paper − c)·k_c`), preserves luminance, and gamut-fits the chroma about the luminance. **Rust** `balance_paper_rows` (`lib.rs:130`) does this in bands of 96 rows on the main thread or all at once in a worker.
4. **Fallback illumination** (no observed field): the canvas scaled to ≤192 px wide, luminance clamped to the field interior (1.78–19.22 × 1.71–28.15 cm), `smoothPaperIllumination` (`:4902`): a separable max-then-min closing with radius `max(2, round(w/16))` followed by a 3-radius box blur ×2.
5. **Paper profile** (`estimatePaperProfile`, `:4266`): sample the interior (12 % / 8 % margins, to 90 % of the height) at stride `max(3, ceil(max/520))`. Neutral pixels (L ≥ 72, sat ≤ .19) give the per-channel P78 = `sourcePaper` (225 grey when there are fewer than 80). `gains = clamp(255/max(104, c), .92, 1.3)`, then within ±.13 of the mean. `targetPaper = 255`.
6. **References**:
   * 4 markers sampled with radius `max(2, .115·pxPerCm)` at `(x cm, 27.68 cm)`, each normalised by its local paper (P80 of a 9×7 grid at y 28.15 cm).
   * 9 yellow rail measurements (sides at y 6/14.8/23 cm and the top at x 6/10.5/15 cm) whose median gives "yellow".
   * `markerLooksValid` (`:4361`) per marker: black `max < .72 && L < .58`; red `r > .18, r−b > .04, r−g ≥ max(.025, .4(r−b))`; blue `b > .16, b ≥ r + .008, b ≥ g + .008`; green `g > .18, g ≥ r + .008, g ≥ b + .015`.
   * Calibration requires all 4 markers. Fit a 3×4 affine colour matrix by weighted ridge least squares (`buildColorMatrix`, `:4224`; ridge .012 toward identity; any coefficient beyond ±3.5 falls back to identity). Markers have weight 2 and paper (1,1,1)→(1,1,1) weight 8; yellow is added at 1.2 when its prediction error is < .2.
   * Accept the matrix if the RMS error is < .14.
   * Targets: red (255,0,0), black (0,0,0), blue (0,0,255), green (110,255,18), yellow (240,219,76) (`CALIBRATION_TARGETS`).
7. **Per-pixel classification** (`:5498-5690`; **Rust** `correct_scan_rows`, `lib.rs:198`, takes a 61-float parameter block, `:5473-5480`):
   * **Margin artefacts**: outside the frame (±.12 cm band), a pixel that is synthetic white (≥254 in the original), has L < 105, or has L < 178 with sat > .22 becomes paper.
   * **Printed yellow** on the frame band or calibration band (strict yellow), or warm yellow on the printed rails or ticks, or saturated pixels (sat > .24) in the marker wells (x 9.92–11.08, y 27.48–27.90 cm) become white.
   * **Ink coverage** = `round(255·clamp((localBright − L)/max(40, localBright)))`. This is the geometry input to vectorisation, measured **before** pigment enhancement.
   * **Coloured stroke** test: has ink contrast (`deficit > max(1.5·noiseFloor, .025·paper)` when observed); `max − min > max(2·noiseFloor, deficit·(.22 − .12·neutralConfidence))`; and (sat > .18, or sat > .105 with deficit > max(6, .035·paper)).
   * **Neutral output**: `reflectance = clamp(1 − (paper − L − noise)/paper)`; transition = `smoothstep((reflectance − .12)/.45)`; strength = `(1.25 − .6·drawing)·transition` when observed, else .65; `neutral = 255·clamp(refl − strength·refl·(1 − refl))`. Deep black is nudged toward the calibrated black reference.
   * **Coloured output**: the calibrated matrix on paper-relative RGB; or else projection onto the nearest reference ink vector (`calibrateFromReference`, `:4336`; residual ratio ≤ .5, mix ≥ .58); or else ×1.08 vibrance. Thin strokes get extra vibrance (`1 + .6·(1 − drawing)·dilution·edge`) and coverage (`1 + .38·…`) toward the hue. Clipped dark warm pixels are limited to their automatic luminance ×1.1 + 3.
8. **Template removal** (`removePhotographedStencil`, `:4374`):
   * Clear the outer rails with a generous band, the marker wells, and the yellow/neutral print in the footer boxes, keeping dark or chromatic handwriting there.
   * Remove the dot lattice: find a global phase vote from the repeated dots, then a per-block local phase. Learn the dot glyph as a robust low quantile across all 1785 cells. Subtract the learned darkness (capped), or, for compact, radially isolated, neutral glyphs, remove them entirely.
   * Keep pixels with dark neighbours on both sides of an axis (strokes), punctuation, chromatic ink and elongated marks. Observed dot positions from the warp are used when they agree with the corrected lattice.
9. **Output quality**: around each `paperPoint`, 16 samples on a 1.5 mm ring (skipping coloured ones, chroma > 26); take P80 per point. The result is `{method:'verified-dot-surrounds', paperSamples, paperResidualFraction = share < 235, paperP10, paperMedian, confidence = min(1, n/600)·min(1, regions/50)}`.
10. The return value also carries `calibration{method: 'paper-relative-six-reference'|'paper-relative-four-markers'|'bounded-reference-fallback', …}`, `illumination{method: 'observed-paper'|'local-paper', …}` and `references`.

### 8.7 Ink separation (`separateScanInk`, `lightweight.js:3725`; **Rust** `separate_ink`, `lib.rs:295`)
Per pixel: `paper = min(r, g, b)` and `alpha = 255 − paper`. If alpha is 0 the pixel becomes fully transparent; otherwise `c' = round((c − paper)·255/alpha)`. The result is the cleaned scan over white reproduced exactly, with paper transparent.

---

## 9. Ink vectorisation (`scanner/processing/ink-vectorizer.js`)

`fromPixels(imageData, {signal, coverage})` (`:659`):
* Requires 2 ≤ w, h ≤ 4096, w·h ≤ 12 M, the palette present, and `coverage` a `Uint8Array(w·h)` when given. Otherwise it throws "Invalid scan ink dimensions" or "Invalid ink coverage dimensions".
* It awaits Rust readiness.

1. **Axes** (Rust `ink_axes`, `ink.rs:83`, geometry version 2; JS `fallback`, `:20`, with identical topology tests):
   * `alpha(i)` is 0 if the RGBA alpha is <16, otherwise the coverage value (or the alpha).
   * Initial mask: alpha ≥ 8 (≥16 without coverage), labelled by `colour()` (`:8`):
     * 1 (neutral) if the chroma `hi − lo` is <18, or < .2·hi, or `chroma·alpha < 255·12`;
     * otherwise the nearest of black (0,0,0), blue (0,49,225) or red (225,0,0) after scaling the chroma to 225, giving labels 1/2/3.
   * **Core** within a `reach = clamp(ceil(2.5·w/794), 3, 8)` window: alpha ≥ `max(.55·peak, min(.65·peak, mean + .05·peak))` (with coverage; otherwise .6·peak). Non-core pixels survive only if a transverse profile in 8 directions shows a narrow ridge (`cross = max(1, round(.75·w/794))`, continuation ≥ .4·α on both tangent sides, a peak ≥ α/.65, and both flanks < .45·peak with peak − flank ≥ 6).
   * **Components** (8-connected): drop if peak < 24, or a single pixel with peak < 90, or no core with extent < `max(4, 3·w/794)`. Small components (≤32 px) get a centre anchor.
   * Chamfer 3-4 distance transform → `radii`.
   * Anchored tips: degree-2 pixels whose continuation at `max(2, 2·w/794)` keeps ≥50 % alpha.
   * Ordered **Zhang–Suen style thinning**: two sub-phases using explicit products, budget 40 M ("Ink could not be separated. Try a clearer photo."). With coverage, the degree ≤1 endpoints of this guide are fixed, the silhouette is restored, and a priority queue (`4·radius + alpha`, weakest first) thins toward the photographed ink ridge.
   * Final label per axis pixel: alpha²-weighted votes within `clamp(floor((r+2)/3), 2, 8)`. Blue or red needs ≥22 % of the total and ≥1.5× the other colour; otherwise neutral.
   * Rust status 2 (budget exceeded) becomes `{rejected:true}` and an error.
2. **Trace** (`:325`):
   * Graph edges skip a diagonal that sits beside a cardinal neighbour. Walk paths from non-degree-2 nodes first, then loops. Limit 60000 paths ("This scan contains too much detail. Try a clearer photo.").
   * **Path colours** (`coherentPathColours`, `:254`): join opposite branches at junctions (dot < −.82) with union-find. Per group, colour label L is supported if votes ≥ 24 %, longest run ≥ `min(total, max(3, 3·scale))` and "strong" pigment samples ≥ `max(1, 8 %)`, where strong means a cross-section chroma ≥24 and (≥45 or ≥ .28·hi) and `chroma·alpha ≥ 255·14`. Blue and red both supported gives mixed (per-run labels); otherwise the supported one; otherwise neutral.
   * **Width**: standard pen = `2·√(w/794 · h/1123)`, about 4.53 px at 1800×2546. The measured cross-section is `min(2r/3 + 1, mass/peak·.5)` along the normal. A group snaps to the standard width if its median is in [.75, 1.6]·std and ≥72 % of its samples lie in [.65, 1.65]·std. Short junction branches (< 4·std) with an oversized median (>1.4× reference) borrow the connected same-colour reference (P30 of long paths, snapped).
   * Paths split only at sustained radius changes (Δ ≥ 6 and ×1.8 over ≥12 samples).
   * **Emit**: `smoothAxis` (`:217`: Gaussian σ = clamp(.25·width, .35·scale, .55·scale), corners kept where cos < .5 over ±`max(2, round(3·scale))`, move ≤ min(.25·scale, .15·width), loops wrap), then `simplify` (Douglas–Peucker, tolerance .08·scale), then resample at step `clamp(min(scale, .6·width), .5, …)`. A single point gets a twin at +.001.
   * Limits: 60000 strokes and 500000 points.
3. **Silhouette restoration** (`restoreSilhouette`, `:550`, only with coverage):
   * step = `clamp(.665·scale, 1, 1.5)`; brush = 1.47·step; levels every 3·step of radius.
   * Strokes with < 90 % of points whose `radius/3 − .5 ≥ width/2 − .35` are thinned to ≤ 1.33·step.
   * `joinBranches` (`:503`) chains same colour and width ends that meet with opposite tangents (dot < −.94).
   * Labels are flood-filled through the axis region. Marching squares over the radius field at each level give contour polylines, emitted where ≥2 nodes are **not** already covered by the rendered pens (alpha < 180). Contours split at label changes; simplify tolerance `min(.8, .28·scale)`, spacing `clamp(1.4·scale, 1, 3.5)`, width = brush.
   * Rust `ink_repair_discs` (`ink_repair.rs:62`) exists and is exported (`InhouseRust.inkRepair`) but is **not used** by this pipeline.
4. With coverage, round coordinates to 0.01. A stroke whose points all coincide gets its last point moved +.05·scale in x.
5. Return `{version:1, width, height, strokes:[{tool:'pen', color: palette[label−1], width, points:[{x,y}]}], backend:'rust'|'javascript', repairCount, repairBackend, reconstruction:'measured-envelope'|'centreline'}`.

`render(ctx, ink)` (`:682`) is the editor's opaque pen renderer: round caps and joins, `source-over`, alpha 1. Two points draw a line. Longer strokes draw a line to the first midpoint, `quadraticCurveTo` through the midpoints, and a line to the last point. The scan preview uses exactly this.

---

## 10. Outputs

| Artefact | Format | Consumer |
|---|---|---|
| `result.blob` | PNG 1800×2546 (portrait): **rendered native pens on a transparent background** (`inkLayer:true, templateKind:'default'`). JPEG .94 for non-precise corrections (not reachable from the camera); the original file when uncorrected. | Editor validates its dimensions and portrait orientation; ignored when `editableInk` is present |
| `result.editableInk` | `{version:1, width:1800, height:2546, strokes}` in scan px | Editor scales to 794×1123 and stores as ordinary pen strokes (`p:.5`) on a page with the default template background |
| `previewBlob` | PNG: native template (white) + pens | Review display, thumbnails, standalone "Save" (`scan.png`) |
| progress `warp.previewBlob` | 720 px JPEG .9: warped photo + template | Review during processing |
| Detection/diagnostics | `detection` (quality, frame, metricSurface diagnostics, scaleSelection, analysisTimings), `dotAlignment`, `outputQuality`, `stencilCleanup`, `timings` | Ranking, tests |

Window events (test and diagnostic hooks): `scanner-live-detection`, `scanner-capture-quality`, `scanner-capture-readiness` (live camera) and `scanner-processing-phase` (legacy animator, `script.js:301`).

---

## 11. Rust/WASM boundary (what is already Rust)

Loader: `rust-core-v5.js:4-22` fetches `wasm/inhouse_notes_core.wasm` (4 s timeout) and instantiates it with no imports. It requires `core_abi_version() === 1`. Memory is managed with `alloc`/`free`, and allocations are limited to 128 MiB. `InhouseRust.ready` resolves to true or false. Every adapter returns null (falling back to JS) for an old core or invalid input.

| Kernel | Rust symbol (file:line) | JS adapter | JS fallback (must match) | Parity test |
|---|---|---|---|---|
| Mesh warp cell | `warp_cell` (`lib.rs:41`) | `warp()` `:31` | `lightweight.js:4082-4111` | `rust-pixels.spec`, `rust-core.test` "mesh sampling…" |
| Colour/ink classification | `correct_scan_rows` (`lib.rs:198`, 13 args incl. coverage and neutral support) | `scanColors()` `:62` | `lightweight.js:5498-5690` | `scanner-rust-colors.spec` |
| Observed paper balance | `balance_paper_rows` (`lib.rs:130`) | `balancePaper()` `:85` | `lightweight.js:5250-5276` | `rust-core.test` "local paper balance…", `scanner-neutral-colors.spec:132` |
| Ink separation | `separate_ink` (`lib.rs:295`) | `separateInk()` `:103` | `lightweight.js:3734-3741` | `rust-core.test` "Rust ink separation…" |
| Developable surface LM | `surface_fit_new/step/result/drop` (`surface.rs:309-412`, 128 steps) | `surfaceFit()` `:111` (≤4500 points, ≤160 guides, 10–32 params) | `metric-surface.js:1336-1396` | `scanner-rust-surface.spec`, `rust-core.test` |
| Ink axes (mask, radii, thinning) | `ink_axes` (`ink.rs:83`), `ink_geometry_version()=2` | `inkAxes()` `:149` | `ink-vectorizer.js:20-199` | `scanner-editable-ink.spec:39`, `scanner-ink-legibility.spec:67`, `scanner-oblique-ink.spec` |
| Ink repair discs | `ink_repair_discs` (`ink_repair.rs:62`) | `inkRepair()` `:166` | — (unused in production) | `rust-core.test` legacy-null check |

**Everything else is JavaScript today**: live detection and illumination, readiness, page detection and frame tracing, surface parameterisation, dot observation, registration and certificate, colour reference and matrix fitting, illumination fields, template removal, path tracing, colouring and silhouette restoration, template rendering and all UI.

---

## 12. Legacy "Scanner Pro" workspace (`scanner/script.js`)

The workspace is reachable standalone at `/scanner/` (not embedded):
* The landing page offers "New Scan" (rebound to the live camera) and "Download Stencil" (PDF/SVG/PNG).
* Dragging images onto the landing page or viewport opens the workspace: sidebar page list (Sortable drag reorder when the library loads), `#btnDesktopAdd`/`#btnAddMobile` (mobile opens the source sheet, whose camera button is rebound to the live camera), theme toggle (persisted in `localStorage.theme`), zoom/pan, magnifier, Crop (manual quad), "Ref" stencil overlay, Auto, and Export PDF (jsPDF A4, JPEG .82 per page, fit and centre).
* Each image is processed serially through `process()` (`script.js:1616`): `detectPageAsync`, then `warp`, then `correctColors(useStencil = preciseStencil)`, with the processing animation (`createProcessingAnimator`, phase texts "Finding yellow marks / Forming page frame / Confirming 4 corners / Mapping page curvature / Straightening page / Balancing colours"). Invalid geometry keeps the original, labelled as such.
* Memory profile `MEM` (`script.js:235`): low-memory devices (mobile, touch ≤1024 px, or ≤4 GB) use a max dimension of 1500 (≤2 GB) or 2200; otherwise 2600.
* In embed mode this workspace is hidden behind the live dialog. Its "Add N pages" path (`addScannedPagesToDocument`, `:2754`) posts JPEG .92 pages (optionally with the stencil overlay) and is unreachable in practice because the live dialog always opens and its close posts `ihn-scanner-close`.

v6 must keep the standalone landing page, stencil downloads and drag-to-workspace for identical features.

---

## 13. Performance budgets and limits

| Item | Value | Source |
|---|---|---|
| Live analysis size | longer side ≤480 px; cadence 80 ms minus analysis time, minimum gap 25 ms | `live-camera.js:273, 437` |
| Overlay | rAF, DPR ≤2, stale after 350 ms, fade over 190 ms | `:239, 359, 366` |
| Readiness | ≥160 ms between checks, one in flight, worker timeout 2000 ms, raster ≤1100 px, rectified grid 540×810 | `:417`, `capture-quality.js:8, 83, 251` |
| Auto-capture evidence | ≥3 valid stills spanning ≥300 ms; gap >650 ms resets; no qualified live frame for >500 ms resets | `live-camera.js:421, 529, 535` |
| Capture | still ≤2560 px; burst of 3; ≤180 ms wait per backup frame; corner drift ≤ .03; JPEG .94; encode timeout 6 s; frozen thumbnail ≤720 px | `:464, 492, 604, 440-445, 473` |
| Tests | capture starts <150 ms after `ready`, <3000 ms after the first qualified frame; backup encoding starts before frame 0 finishes encoding | `scanner-live-resilience.spec:348-352`, `scanner-moving-capture.spec:64-67` |
| Camera watchdog | 12 s before the first frame, 5 s after; 750 ms gives "Waiting" | `:382, 386` |
| Per-photo processing | one worker each (3 concurrently); timeout 20 s (clamped 1–60 s); surface budget 7800 ms in a worker, 2600 ms otherwise; fine profile only with ≥180 ms left | `lightweight.js:3771, 3952`, `metric-surface.js:1704, 1814` |
| Analysis sizes | 608 → 800 px detection; 1200 px refinement and evidence; 1000 px dot graph; ≤2600 px dot centres; 1100 px certificate; colour fields ≤192 or ≤512 px | as cited |
| Output | 1800×2546 PNG; preview 720 px JPEG .9 | `lightweight.js:12, 3703` |
| UI-thread yielding (fallback only) | warp every 3 mesh rows; colours every 96 rows; illumination every 48 rows; vectoriser between passes and every 2000/500/8192/256 work items | as cited |
| Vector limits | ≤4096 px side, ≤12 MP, ≤60000 strokes, ≤500000 points, ≤40 M thinning work, ≤500000 contour nodes | `ink-vectorizer.js` |
| Bridge limits | ≤24 pages, ≤18 MiB per page, ≤120 MiB total; app yields every 2 pages; structure lock 12 s | `app-v5.js:406-408, 20464` |
| Readiness worker | single queue; duplicates rejected as `readiness-busy` | `capture-quality.js:215` |

---

## 14. User-visible strings (must be reproduced verbatim)

**Live camera**
* Opening: "Opening camera…"
* Guidance: "Waiting for camera…", "Fit the yellow frame", "Focusing…", "Checking page…", "Finding page dots…", "Scanning…"
* Analysis error: "Could not check the page" / "Try again or choose a photo."
* Camera errors (§3.3): "Camera interrupted", "Camera permission needed", "No camera found", "Camera unavailable", plus the two hints given there
* Pause: "Camera paused" / "Open the camera to continue scanning."
* Capture failure: "Could not capture the page"

**Review**
* Progress: "Finding dots…", "Straightening…", "Adjusting colour…", "Tracing ink…", "Finishing…", "Comparing scans…"
* Result: "Ready", "Couldn’t correct this photo. Try again." (typographic apostrophe)
* Submission: "Adding…", "The document is no longer available.", "Could not add the page. Your scan is still here; try again."
* Labels: "Add page"/"Save", "Review frame N", "Photo could not be corrected"

**Pipeline errors**
* Pipeline errors are listed in §§3.7, 3.8, 7, 8.3, 8.4, 8.5, 9.
* `failureReason` falls back to raw reason codes (for example `refined-contour-disagreement` or `live-contour-not-observed`), which are shown verbatim as `candidate.error` (`lightweight.js:3786`). This is a quirk; keep or replace it deliberately.

**Editor**
* "Scanned page added", "N scanned pages added", "Failed to add scanned pages"
* Validation messages: `app-v5.js:19553-19623`

---

## 15. Tests pinning behaviour

Run with `npm run test:unit` (node, includes `tests/scanner-live-detection.test.mjs` and `tests/rust-core.test.mjs`) and `npx playwright test tests/e2e/live-scanner.spec.mjs tests/e2e/scanner-*.spec.mjs` (Chromium, default timeout 30 s, CI 1 retry and 1 worker; `playwright.config.mjs`).

The camera tests feed real Canvas `MediaStream`s with synthetic footage (`installCameraFixture`, `live-scanner.spec.mjs:9`). Fixtures in `tests/e2e/fixtures/` provide independent page renderers:
* `scanner-synthetic.js`: flat and perspective pages;
* `scanner-developable.js`: pinhole plus arc-length 3D sheets;
* `scanner-physical.js`;
* `scanner-scene.js`: S-bends on tables and pale books;
* `scanner-paper-contrast.js`: a reflectance and illumination field;
* `scanner-dot-metrics.js`: measures the output dots.

Manual tools: `tests/scanner-visual-validation.mjs <photo.jpg>` and `tests/scanner-synthetic-visual-validation.mjs` (write `test-results/scanner-*`).

Legend: **[slow]** = per-test timeout ≥90 s; **[timing]** = asserts or depends on wall-clock timing, concurrency or real media timing (flaky under CPU load).

### Unit tests (`tests/scanner-live-detection.test.mjs`)
- uses paper perimeter without claiming absent yellow markers (:90)
- does not draw predicted boxes when the calibration strip is absent (:98)
- does not promote a square yellow rectangle to automatic A4 capture (:105)
- finds a frame against a white desk and in dim light (:110)
- rejects uniform views and yellow surfaces (:118)
- rejects clipped sheets and invalid image data (:123)
- traces observed curved rails rather than drawing the corner chords (:130)
- keeps measured rails through a partial reflection and validates every new frame (:164)
- does not confuse the edge of a white sheet on a warm desk with yellow printed ink (:173)
- a previous seed follows a moving curved page without freezing its corners (:181)
- small changes in noisy ink do not turn a bowed edge into many little folds (:202)
- measures faint yellow rails relative to the paper under cool, warm and mixed illumination (:238)
- illumination normalization preserves neutral data and rejects coloured desks and neutral printing (:260)
- full-resolution illumination normalization yields, cancels and matches detector pixels exactly (:281)
- a larger pastel desk cannot recolour a smaller correctly exposed white sheet (:301)
- rectified-page illumination uses the physical interior rather than synthetic white margins (:315)
- four ordered colour circles select the page illuminant independently of a brighter neutral desk (:343)
- verified colour references retain the frame in low exposure without manufacturing capture quality (:363)
- bright paper with a veiling reflection keeps independently measured rails (:373)
- missing, reordered or occluded colour circles cannot become an illuminant reference (:381)
- a small ordered row of laptop icons at the image edge is not page calibration (:393)
- identifies the printed layout when the rails are grey or shifted away from yellow (:410)
- layout recovery requires the measured box strips and matching four-circle footer (:429)

### Rust unit tests (`tests/rust-core.test.mjs`, scanner-related)
- Rust core loads with no host imports (:9)
- mesh sampling matches clamped canvas bytes and is reusable after disposal (:24)
- Rust ink separation reconstructs coloured and neutral photographs over white (:46)
- Rust scan buffers reject inconsistent paper fields before allocating (:56)
- Rust surface contexts reject invalid correspondences and release independently (:63)
- Rust scan axes preserve faint pen cores in the discrete palette without mutating inputs (:79)
- Rust scan axes reject malformed and unbounded image allocations (:99)
- local paper balance removes a measured cast without changing coverage or neighbouring pigment (:107)
- a cached core without measured coverage or neutral calibration requests the safe JS fallback (:125)

### Live camera (`tests/e2e/live-scanner.spec.mjs`), all **[timing]**
- live video draws the yellow frame and captures after verifying the sharp bullet page (:110, 45 s)
- retaking releases the previous camera and only confirmation inserts the scanned page (:161, 60 s), asserting the saved page is template, default, no image, pen strokes
- a `${mode}` camera image never triggers automatic capture (:202)
- denied camera permission offers image import and retry without trapping the scanner (:215, 45 s)
- closing while permission is pending stops a camera stream granted later (:235)
- Android permission visibility changes preserve camera startup (grant while hidden: true/false) (:247)
- backgrounding an active camera stops tracks and keeps a visible retry on return (:271)
- a `${interruption}` camera feed stays in the scanner with retry instead of a frozen preview (:287)
- a short camera mute resumes live frames without capturing a stale frame (:316)
- older WebViews can capture three live frames even when image encoding is slow (:335, 45 s)

### Live robustness and readiness
- `scanner-live-resilience.spec` **[timing]**:
  - small brief-loss/soft-focus fluctuations retain tracking and complete capture promptly (:202)
  - a document that leaves the feed is cleared and never captured from retained tracking (:219)
  - a verified clear frame survives a detection miss during the backup burst (:228)
  - a crisp blank-crisp/blank-markers page never captures without a measured bullet lattice (:237)
  - a blurred blank template remains in the live feed instead of passing border focus (:256)
  - the complete displayed outline moves between analyses and damps local contour jitter (:268): shown jitter < .65× measured, final motion < .105
  - visual interpolation expires after a lost document and stops when the camera closes (:293)
  - invalid page geometry cannot be inserted and automatically returns to the camera (:304)
  - one clear video frame cannot trigger a capture after the page disappears (:311)
  - a late readiness result cannot capture or reopen a closed camera (:324)
  - sustained sharp grid evidence precedes a burst that freezes before slow JPEG encoding finishes (:338): elapsed ≥300 ms, start <150 ms, <3 s
- `scanner-live-detection.spec`:
  - live detector follows printed rails through perspective, rotation, curl and lighting (:9)
  - blur does not produce a sharp automatic capture candidate (:75)
- `scanner-capture-readiness.spec`:
  - capture preflight finds the photographed lattice on sharp perspective and curved pages (:9)
  - capture preflight rejects blur before border confidence can trigger capture (:25)
  - sharp borders and markers cannot substitute for a sufficiently covered two-axis dot lattice (:42)
  - readiness worker matches fallback evidence and leaves animation frames responsive (:83) **[timing]**
  - readiness cancellation discards old work and duplicate frames cannot queue indefinitely (:101) **[timing]**
  - worker-unavailable fallback retains the same sharpness and lattice requirements (:126)
  - camera noise and a crisp border cannot conceal a smeared or washed out interior (:141)
  - readiness uses locally visible dots under colour casts and rejects clipped-away dots (:166)
  - sharp page edges and outer regions cannot mask a defocused centre (:187)
- `scanner-moving-capture.spec`:
  - a moving sharp bullet page captures after repeated grid verification without requiring stationary corners (:6) **[timing]** (<150 ms / <3 s)
- `scanner-slow-readiness.spec`:
  - slow 400/600 ms preflight still captures three distinct verified frames (:3) **[timing]** (elapsed >1000 ms)
- `scanner-photo-focus.spec`:
  - the frozen photo focus gate rejects smeared text even with confident yellow detection (:3)

### Capture, review and parallel processing
- `scanner-capture-regression.spec`:
  - capture shows a decoded photo while correction is pending, then inserts only the decoded result (:89, 60 s)
  - undecodable/unverified captures stay available until an explicit retake (:139, 60 s)
  - invalid imported image leaves processing and a subsequent valid photo can be reviewed (:162)
  - retaking during correction cancels the old result and keeps the new preview (:177) **[timing]**
  - a verified scan can be added while slower backup photos are still processing (:203) **[timing]**
- `scanner-parallel-review.spec`:
  - all three photos run concurrently and the best corrected result wins regardless of completion order (:78) **[timing]**
  - clean paper beats a shadowed registered photo without overriding a manual choice (:99)
  - sparse or invalid paper measurements cannot promote a worse scan (:123)
  - choosing a thumbnail keeps that choice while other corrections finish and uses their cached outputs (:142)
  - registered dots/held-out physical geometry take priority over a superficially stronger correction (:159)
  - camera review rejects corrected results without independent final dot verification (:191)
  - closing aborts every pending photo and ignores late completions without retaining image URLs (:209)
- `scanner-processing-visual.spec`:
  - measured bullet points and curved runs align with the contained photo without filling missing dots (:92)
  - warp progress displays actual corrected observations and removes all guides from the finished scan (:115)
  - each capture keeps its own observed dots and closed scans ignore late progress and preview blobs (:153)
  - the automatically selected frame shows its finished scan while the remaining candidates still run (:177) **[timing]**
  - a selected backup that fails correction cannot replace a successful scan (:200)
  - dots, yellow frame and four reference circles stay visible through the final processing stages (:218)
- `scanner-worker-pipeline.spec`:
  - three photographs complete their full corrections concurrently outside the UI thread (:7) **[timing]** (all start before any finishes)
  - cancelling during worker cleanup terminates that candidate without publishing a scan (:67)
  - an unverified photograph is returned byte-for-byte without correction in workers and legacy fallback (:93)
  - a worker startup failure still completes a verified scan on older WebViews (:124)
  - an image supported only by the document decoder retains the original import (:144)
- `scanner-processing.spec`:
  - still detection runs in a worker and cancellation leaves the camera responsive (:7) **[timing]**
  - a capture contour absent from the still image cannot certify a different crop (:30)
  - surface resampling is cancellable between bands without completing a stale image (:50) **[timing]**
  - colour processing removes deep smooth shadows while retaining fine pencil and coloured ink (:70)
  - a folded detection cannot become a plausible looking but mirrored scan (:107)
  - cold and warm illumination are removed without bleaching pencil or coloured writing (:129)
  - template cleanup preserves isolated punctuation even where it crosses a printed dot (:167)
  - synthetic white warp margins do not create a grey halo or erase nearby writing (:185)
- `scanner-required-grid.spec`:
  - a sharp physical bullet grid is certified before camera output in `${mode}` (:47)
  - camera rejects a sharp border with `${pattern}` dots before warp in `${mode}` (:62)
  - camera cannot accept `${certification}` final certification in `${mode}` (:80)
  - ordinary bordered-photo imports remain supported without the camera grid requirement (:108)
- `scanner-adaptive-recovery.spec`:
  - a failed final raster check retries detailed observation before rejecting the photo (:3, 60 s)
- `scanner-import-quality.spec`:
  - photo import keeps rejected geometry original in preview and export, then allows a manual crop (:3) (legacy workspace)
- `scanner-interior-quality.spec`:
  - an invalid interior cannot hide beneath a native grid in a worker/legacy processing (:4)
  - a genuinely flat bullet page still receives the native template (:41)
- `scanner-speed-profile.spec`:
  - scan processing profile keeps geometry and reports measured stage timings (:3): only logs `SCAN_PROFILE`; asserts corrected and validation >500

### Geometry and surface
- `scanner-geometry.spec`:
  - scanner reconstructs 15 deterministic 3D, orientation and colour variants (:3) **[slow 120 s]**
- `scanner-surface.spec`:
  - perspective fallback restores interior writing using a camera homography (:8)
  - observed guides flatten interior writing on independent physical paper surfaces (:19) **[slow 90 s]**
  - dense surface interpolation preserves physical cylindrical and S-bend writing (:43)
  - missing calibration teeth cannot shift the inferred physical cell numbers (:60)
- `scanner-metric-surface.spec`:
  - physical 3D fit restores measured interiors for longitudinal and diagonal paper bends (:13) **[slow 90 s]**
  - a border without enough interior observations cannot invent a metric 3D surface (:90)
  - incompatible two-axis stretching is rejected instead of inventing a developable paper fit (:109)
  - detected curved borders and the physical surface preserve writing through the complete warp (:132) **[slow 90 s]**
- `scanner-rust-surface.spec`:
  - Rust 3D optimization preserves independently validated camera and sheet geometry (:3) **[slow 90 s]** (budget 7800 ms per fit)
- `scanner-grid-parallels.spec`:
  - ordered grid parallels recover 3D bends despite missing dots and coloured light (:5)
- `scanner-bullet-pipeline.spec`:
  - worker shows measured bullet rows before straightening and restores the printed template after preserving ink (:6)
  - a diagnostic thumbnail encoding failure does not discard a corrected worker scan (:174)
- `scanner-bullet-refinement.spec`:
  - measured bullet rows correct remaining local bow and report only observed dots before fitting (:8)
  - blank and incompatible sheets cannot invent dots or bypass physical surface rejection (:65)
- `scanner-dot-registration.spec`:
  - photographed bullet pixels register to the template in a sharp/blurred JPEG/small subpixel photograph (:8)
- `scanner-dot-registration-geometry.spec`:
  - exact bullet registration uses real source centres and leaves missing sites as interpolation only (:3)
- `scanner-dot-certification.spec`:
  - raw photographed dot alignment distinguishes coherent bow and phase errors from small local jitter (:3)
- `scanner-faint-dots.spec`:
  - faint photographed dots retain their measured lattice and independently verified 3D bend (:3)
- `scanner-lattice-recovery.spec`:
  - measured lattice preserves writing with `${variant}` (:9)
  - a plausible perimeter without real dots cannot produce a recovery surface (:39)
- `scanner-layout-detection.spec`:
  - observed footer and frame layout recover non-yellow printing on curved sheets (:3)
  - a grey-frame camera scan retains full photo and output dot-grid verification (:29)
  - still-photo frame verification survives a lamp change after the live detection (:50) **[slow 90 s]**
- `scanner-raster-regression.spec`:
  - strong-left-curl/strong-bottom-curl/faded-yellow preserves observed geometry after one-pixel motion/JPEG/subpixel motion (:12)
- `scanner-scene-regression.spec`:
  - complete processing keeps only the marked sheet: s-bend on plain table/pale book/warm pale book (:8)
  - live camera review preserves the writing of the sheet lying on a pale book (:77) **[timing]**
  - a faint scene is either reconstructed correctly or rejected without enabling insertion (:117)

### Colour and lighting
- `scanner-color-lighting.spec`:
  - four measured footer colours calibrate recoverable green, magenta, tungsten, dim and bright illumination (:100)
  - local coloured flare on black ink is not mistaken for a coloured pen (:127)
  - thin JPEG pens retain navy and dark red without colouring black under local reflections (:146)
  - four footer colours still calibrate when the thin yellow border cannot be sampled (:162)
  - missing or inconsistent footer pigment cannot be borrowed from a neighbouring disc (:174)
  - neutral frame ridges cannot override the four measured colour pigments (:192)
  - mixed ambient light keeps black neutral without erasing blue and red ink (:205)
- `scanner-lighting-pipeline.spec`:
  - complete scan recovers geometry and writing under cool-daylight-with-shadow / warm-lamp-with-shadow / cool-daylight-camera-jpeg (:13)
  - lighting correction cannot invent 3D grid geometry on a coloured card (:120)
- `scanner-marker-lighting.spec`:
  - four measured references recover cool/warm/magenta paper beside a bright neutral desk, and dim neutral paper (:13)
  - four references cannot invent a completely clipped frame and dot lattice (:70)
- `scanner-neutral-colors.spec`:
  - neutral printed dots calibrate strong local reflections without recolouring real navy or red pens (:96) **[slow 120 s]**
  - coloured handwriting crossing some reference dots cannot recolour nearby black ink (:117)
  - neutral-axis calibration keeps saturated ink luminance when colour correction reaches RGB limits (:132)
- `scanner-paper-contrast.spec`:
  - white paper and stronger writing survive neutral/cool/warm-JPEG shadows, missing/sparse/absent/partially missing dot observations, and faint blue ink (8 cases) (:15)
- `scanner-reference-colors.spec`:
  - six measured references restore consistent red and blue pen colours across lamps and camera flare (:3)
- `scanner-rust-colors.spec`:
  - Rust colour processing agrees with the JavaScript fallback under warm light (:3)
- `scanner-observed-dot-cleanup.spec`:
  - measured bullet positions clean displaced print while preserving pencil and punctuation (:7)
  - stale source-space dot coordinates do not replace photographed-grid recovery (:59)
  - verified bullet centres survive nearby camera speckles without leaving a second dot pattern (:80)
  - soft photographed dots are removed across the whole page, including sites missing from geometry (:133)
  - a faint handwritten i-dot near a missing printed site is not mistaken for recovered print (:187)

### Ink and editable output
- `scanner-editable-ink.spec`:
  - Rust makes opaque palette pens while preserving curves, closed loops, pencil and punctuation (:39)
  - scan pens survive local reload and PDF reopening with no photograph underneath (:75) **[slow 120 s]**
  - native lasso resize, recolour, partial erase and undo work on scanned pens (:98) **[slow 90 s]**
  - the scanner bridge rejects intermediate colours instead of storing them (:157)
  - blank ink and cancellation cannot introduce template dots or a partial vector result (:163)
  - weak JPEG fringes do not turn coloured handwriting into outlined black pens (:175)
  - a photographed page reaches the document as native pens through the actual Add button (:195)
  - mixed camera fringes produce one red axis, while real blue and black crossings keep their colour (:219)
  - near-standard pen widths snap to the editor width at either scan resolution, preserving thin and thick pens (:248)
- `scanner-dense-native.spec`:
  - a dense scanned page stays editable and preserves native pens through input, undo and reload (:60) **[slow 120 s]**
- `scanner-native-template.spec`:
  - scanned ink uses the native editor template through insertion, local reload and a PDF round-trip (:81) **[slow 120 s]**
  - a JPEG cannot masquerade as transparent native scan ink (:112)
- `scanner-template.spec`:
  - the native scan preview keeps canonical bullet positions, calibration boxes and solid colour markers (:12)
  - the native template has opaque colour rings and preserves caller state; separate ink stays above it (:51)
  - scanner previews match every pixel of the original native editor renderer at display and scan resolution (:103)
  - worker and document canvases produce the same complete template without an image decoder (:118)
- `scanner-handwriting-fidelity.spec`:
  - photographed cursive keeps the shape of small letters, hooks and counters (:3)
- `scanner-ink-continuity.spec`:
  - fine handwriting stays continuous beside darker ink at 1x/2x (:3)
- `scanner-ink-edges.spec`:
  - camera JPEG colour bleed is not amplified into a halo around pen strokes (:3)
- `scanner-ink-legibility.spec`:
  - small photographed letters keep their counters, thin strokes and measured widths in Rust and JS (:67)
  - pigment enhancement cannot change pen geometry or resurrect erased template pixels (:85)
- `scanner-ink-repair.spec`:
  - recovering pen lobes cannot borrow a nearby different ink colour (:3)
  - a large filled drawing keeps its hole without generating thousands of repair pens (:51)
- `scanner-oblique-ink.spec`:
  - oblique cursive curves survive darker crossings without changing Rust/JS geometry (:5)
- `scanner-pen-consistency.spec`:
  - shaded blue stays one colour across black patches and junctions, without recolouring a crossing black pen (:3)
  - light smoothing removes raster zigzags at either resolution, retaining endpoints, sharp turns and closed loops (:40)
  - a shaded near-standard pen keeps one width and smooth axis while a cold black remains black (:89)
- `scanner-variable-width.spec`:
  - photographed variable-width letters retain their silhouette, tapers and open counters (:6) **[slow 90 s]**

### Integration elsewhere
- `app-resilience.spec`:
  - scanner overlay and iframe URL (:261)
  - multi-page scanner insertion is ordered, durable, and preserves shifted pages (:267)
  - embedded scanner opens directly into the live camera workflow (:292)
  - scanner starts offline and processes a stencil without OpenCV (:310)
- `rust-pixels.spec`:
  - Rust scan warp matches the production JavaScript path on perspective and curled meshes (:3)
  - pixel benchmark includes all buffer copies and reports medians (:27) **[timing]**
- `tests/release-v5-smoke.mjs`: static checks on the scanner files, including `#scanner-editor-frame[allow="camera"]` (:75-76)
- `tests/native-assets.test.mjs`: asset hashes for `scanner/index.html` (:19)

---

## 16. Notes for the v6 rebuild (behaviour to preserve or decide explicitly)

1. **The code is authoritative over the README**: the readiness rule is 3 samples over 300 ms, not 5 over 800 ms.
2. Uncorrected originals can be **reviewed but never inserted** from the live UI, because Add requires `result.corrected`. Imports from the live UI are corrected only when `precise`.
3. Raw reason codes can surface as the review error text (§14).
4. `#liveAddBtn.textContent` assignment removes its icon after the first review render.
5. Dead legacy processing modules (`colorTransform.js` and the others in §0) do not need to be ported. `values.config` only affects calibration targets, and those equal the defaults.
6. Rust `ink_repair_discs` is unused by production. The JS marching-squares envelope (`restoreSilhouette`) is the behaviour to reproduce.
7. JS and Rust kernels must stay bit-identical (`round_ties_even` versus `Uint8ClampedArray` rounding). Parity tests exist for warp, colours, ink axes and surface fit.
8. Every async stage checks a generation token and an `AbortSignal`. Late worker messages, previews and readiness results must be ignored after Retake or Close, and all object URLs revoked (pinned by the parallel-review and processing-visual tests).
