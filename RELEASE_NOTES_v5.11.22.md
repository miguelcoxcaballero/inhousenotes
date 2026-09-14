# Inhouse Notes v5.11.22

## Closing the PDF recovery gap from v5.11.21

- v5.11.21 fixed the main cause of destructive PDF saves, but left one gap: if a save was ever still forced onto the destructive raster pipeline (strokes flattened into page pixels), that specific output file carried no record of the true original PDF. Closing the document right after that save — before any later save had a chance to self-heal — meant reversible editing was lost permanently on reopen, with no way back.
- The raster fallback now always embeds the clean original PDF inside the file it writes, whenever one is available in memory, using the same mechanism the safe save path already used. Reopening that exact file can now recover full, traceless editing instead of being stuck on the white-cover approximation.
- Found and fixed a second, independent instance of the same class of bug while verifying this: the code that reads a clean original back out of an attachment had its own hardcoded 60MB cap, 4MB stricter than the app's real 64MB upload limit — it would have silently refused to recover an original that was legitimately captured and attached under the real limit. Both caps now reference the same constant so they can't drift apart again.
- Confirmed with a real end-to-end test: a document is forced through the destructive path, and the resulting file is independently reopened and verified (via the same code path production uses) to actually contain a recoverable original.
- What's still true from v5.11.21: a PDF that was already saved destructively by an older build, or before this fix existed, cannot be recovered — the original pixels are genuinely gone in those specific files. This closes the gap going forward, not retroactively.

## General lag

- Profiled real drawing, erasing, page navigation, and PDF scrolling sessions with Chrome's CPU profiler instead of guessing. The stroke-rendering fix from v5.11.20 held up: freehand drawing and erasing cost well under a millisecond per input event, and scrolling a rendered PDF showed no CPU bottleneck.
- Found one real, measurable inefficiency: the "last saved" timestamp label re-ran the expensive `toLocaleTimeString` formatting on every single stroke while actively writing, instead of once per clock-minute (the label's own display granularity). It's now cached per minute, removing that repeated cost from the hot path.
- Did not find further evidence of a broader drawing/rendering bottleneck in this pass — if it's still laggy after this release, the next step is a specific repro (device, and what action feels slow) rather than more guessing.

## Tests

- Extended `tests/e2e/pdf-nondestructive-erase.spec.mjs` to verify the raster fallback's output actually carries a recoverable clean original, using a realistic failure simulation (pdf-lib failing on the specific original PDF, not a blanket stub) and independent pdf.js-based verification of the written file.
- Added `tests/e2e/save-timestamp-format.spec.mjs` covering the minute-bucket cache: same-minute calls hit the cache, a minute boundary crossing is not stale, and the "X min" / empty-timestamp paths are unaffected.

## Android

- The existing Android `1.0.10` shell loads this web release automatically; no native binary change is required.
