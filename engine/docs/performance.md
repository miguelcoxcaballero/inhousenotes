# Performance and graphics presets

How the renderer spends a frame, what each quality preset changes, and the
budget it is designed for. Target: **1080p at 60 FPS or more on a mid-range
GPU (GTX 1660 / RX 580 class) with the High preset**, at the window's
native resolution (render scale 100% on every preset; lower scales and
dynamic resolution are player options).

## Tools

| Tool | What it does |
|---|---|
| **F11** in the game | Frame profiler: FPS, frame-time graph (CPU white, GPU green, lines at 16.7 and 33.3 ms), internal resolution, render scale and window size, whether the frame is CPU-bound or GPU-bound (CPU work vs GPU timestamps), draw calls, triangles and instances (camera and shadows), lights, static grid size, cascades refreshed this frame, VRAM estimate, CPU time per game system and per renderer scope, GPU time per pass, the heaviest scenes by triangles, and a CSV logger. |
| **F9** in the game | Graphics options (preset, auto, every cost knob below). Applies at once except the texture cap (next map); *Save* writes `data/graphics.toml`. |
| `sandbox --bench N [--preset P] [--csv f.csv]` | Headless: N real-time frames after warm-up; prints mean/p50/p95 frame time, per-system CPU, renderer scopes, GPU passes, counts and the heaviest scenes (camera and shadow). |
| `FORJA_LEGACY_STATICS=1` | Sandbox: resubmit the whole world every frame as before the static grid (before/after comparisons). |
| `FORJA_PRESET=low\|medium\|high\|ultra` | Any tool or example: render with that preset. |
| `sandbox --smoke N [--window] [--adapter warp] [--preset P]` | The real game loop for N frames (headless offscreen, or in a window through the swapchain); prints per-frame times and GPU passes, saves `--screenshot` (default `smoke.png`) and exits non-zero on any wgpu validation error, lost device or blank image. The Windows release workflow runs it on D3D12 WARP. |
| `--adapter auto\|warp\|dx12\|vulkan\|<name>` / `FORJA_ADAPTER` | Adapter choice (see `forja_render::gpu`): best GPU first, falling back adapter by adapter. |
| `FORJA_PROFILE_CSV=f.csv` | Game: log every frame to a CSV from startup. |
| `sandbox --frame-trace N` / `FORJA_FRAME_TRACE=N` | Game, smoke or bench: every N frames, one line with the main thread's CPU timeline (sim per system, extract, renderer scopes, GPU wait per pass, present/acquire, loop wait), main-thread and process CPU, fixed steps per frame, log lines, upload flushes and streaming. `FORJA_TRACE_FILE=f` also writes the lines to a file. First thing to ask for when a machine is slow. |
| `city` example: `CITY_STATIC=1`, `CITY_BENCH=N` | Static grid path and CPU timing per shot. |
| `cargo run --release -p sandbox --example worldstats [map]` | Geometry and texture budget of a map without a GPU. |

GPU pass times come from timestamp queries (`TIMESTAMP_QUERY`, requested
when the adapter has it; DX12 and Vulkan desktop drivers do). They are read
back a few frames late through a ring of buffers, so profiling never stalls
the GPU. Passes with the same name are summed (the bloom chain, the shadowed
spots).

## Frame structure (High, daytime)

| # | Pass | Resolution | Cost drivers |
|---|---|---|---|
| 1 | Shadow cascades c0..c3 | 2048² each | Caster triangles. c2 and c3 are refreshed on alternate frames (texel-snapped, padded 4%), so ~3 cascades render per frame. |
| 1b | Spot shadows (night) | 1024² each, up to 4 | Casters inside each spot cone. |
| 2 | Depth + normal prepass | internal | Camera triangles. |
| 3 | GTAO + denoise + upsample | half res | 24 depth taps per half-res pixel; depth-aware 3x3 upsample. |
| 3b | Light clusters (compute) | 16 px tiles x 24 slices | Lights after culling. |
| 4 | Main PBR + sky + sprites | internal | Per pixel: PCSS (near 2 cascades), PCF (far), contact shadow (10 steps), SSR (32 steps), clustered lights, fog. |
| 5 | TAA | internal | 1 pass + history copy. |
| 5b | Eye adaptation (night) | 1/32 histogram | Two tiny compute passes. |
| 6 | Bloom | 6 mips | 11 small passes. |
| 3c | Sky-view LUT (+ transmittance LUT once) | 192x108 | 30 steps per texel. |
| 3d | Froxel fog: inject + integrate (compute) | 1/12 res x 64 slices | 1 shadow tap + cluster lights per froxel; 64-step column integration. |
| 5a | Motion blur (tile max, neighbour max, gather) | internal | K^2 loads per tile, 2 x 10 samples per moving pixel; skipped when nothing moves. |
| 7 | Tonemap, CAS/FXAA, camera stage + FSR 1 EASU upscale | internal -> output | The last pass upsamples with EASU when the internal resolution is below the output. |

## What was slow (measured)

Measured headless on the real default map (*Madrid y Valencia*, 89 738
placements, 168 665 static mesh parts, 455 M instanced triangles) and on the
procedural city example, with lavapipe (CPU rasterizer). CPU numbers are
real; lavapipe GPU numbers are only meaningful as proportions between passes
and as triangle counts.

1. **The whole city was resubmitted, culled, LOD-selected and sorted five
   times per frame** (camera + 4 cascades), for every one of ~170 k parts:
   `cull` alone was ~29 ms of CPU per frame on the default map (and 17-28 ms
   in the city example aerial view). Now the static world is uploaded once
   into a 32 m XZ grid of *runs* (same mesh+material, sorted by radius); a
   view skips whole cells, copies cells fully inside run by run, and
   small-detail culling is a binary search. CPU cull: **5-10 ms on the
   default map, ~1 ms at street level in the city example**, independent of
   city size.
2. **Shadows are the largest GPU cost**: ~50% of lavapipe GPU time, 10-12 M
   caster triangles per frame against 3-6 M in the camera. The heaviest
   casters are facade windows (75 k instances, 5.7 M triangles) and cars.
   Fixes: far cascades refreshed every other frame (High) or less (Low:
   every 4th), shadow LOD from the texel size (one triangle per two shadow
   texels, never finer than the camera LOD plus a bias), casters smaller than
   N texels skipped (N per preset), PCSS only in the near cascades.
3. **A 1.6 M triangle tree** planted 267 times was 94% of the map's
   instanced triangles. Its LOD chain stopped at 4-6 k triangles (8
   halvings); it now goes down to a few hundred (12 levels). Cars and other
   loaded models only got LODs above 20 k triangles per part (a car part
   has ~1.7 k), now above 1 k. Static items in short
   runs (trees, big props) pick their LOD per instance from their own
   distance instead of their grid cell's nearest point (which had made every
   tree of the camera's block use its finest level).
4. **GTAO at full resolution** with 24 taps per pixel: now half resolution
   (a quarter of the pixels, rotating the evaluated pixel of each 2x2 block
   every frame so TAA integrates all four) with a depth-aware upsample.
5. **Textures**: 104 unique textures, 1158 MB as RGBA8 with mips (48 of
   them 2048²). They are now capped per preset (Low 1024 -> ~355 MB,
   Medium/High 2048-4096 -> 1158 MB). Block compression (BC7/BC5) is not done
   yet: it needs an offline encoder in the asset pipeline (encoding at load
   time would add minutes) and is the next step for VRAM and bandwidth.
6. **Present mode**: was AutoVsync with the default frame latency; now FIFO
   (v-sync) or Mailbox, with `frame_latency` 2 (configurable 1-3), and the
   adapter is requested as HighPerformance (laptops with two GPUs).

## Measured before / after

Headless, lavapipe, 960x540, 12 real-time frames after warm-up, on a
shared 4-core VM (other jobs running: CPU numbers vary by ±30% between
runs). *Before* = the whole world resubmitted per frame
(`FORJA_LEGACY_STATICS=1`, same build otherwise).

Default map (*Madrid y Valencia*), High, street-level spawn:

| | Before | After |
|---|---|---|
| Renderer CPU: cull + LOD + sort | 28.8 ms | 5-10 ms |
| Renderer CPU total (incl. lavapipe `submit` 18-35 ms) | 60.6 ms | 30-46 ms |
| Camera triangles | 2.86 M (6.05 M with grid LOD per cell) | 2.56 M |
| Shadow triangles | 11.1 M | 9.2 M |
| Trees in the camera / shadows | 0.27 M / 0.47 M | 0.30 M / 0.33 M |
| Cars in the camera / shadows | 0.38 M / 1.27 M | 0.22 M / 0.45 M |
| Camera / shadow draw calls | 2946 / 6656 | 2882 / 6082 |
| Image | `game_before_high.png` | `game_after_high.png` (identical) |

In v0.1.1 the tree's LOD chain stopped at ~4-6 k triangles per variant
(8 halvings), so every tree beyond ~30 m and every tree in the shadow maps
cost 10-20x more than now; that is not in the *before* column above (the
LOD change is in both builds).

Procedural city example (`city small`, 28 k static parts):

| Shot | Cull before -> after | Renderer CPU before -> after |
|---|---|---|
| Street (madrid_street) | 4.4-8.6 -> 1.1-1.3 ms | 19-24 -> 8.5-11 ms |
| Aerial | 17-28 -> 4.6 ms | 43-59 -> 30 ms |

Presets on the default map (same session, fixed 960x540 so the render
scale is not applied; lavapipe GPU sum is only a proportion):

| | Low | Medium | High | Ultra |
|---|---|---|---|---|
| Shadow triangles | 2.6 M | 6.3 M | 10.3 M | 16.2 M |
| Shadow draws | 2299 | 4334 | 6078 | 9484 |
| lavapipe GPU sum (relative) | 54% | 75% | 100% | 178% |
| Static cull CPU | 2.7 ms | 4.6 ms | 6.2 ms | 11.9 ms |

The images of every preset (game and city example, day and night) are in
`engine/screenshots/agents/perf/`.

## Presets

Presets change only **costs**: resolution, sample and step counts, update
rates, caps. Exposure, tonemapping, light colours, fog, materials and the
night look are identical in all of them. High is the default and has the
same image as before this work (plus dynamic resolution that only engages
when the GPU is over budget).

| Knob | Low | Medium | High | Ultra |
|---|---|---|---|---|
| Render scale (default) / dynamic floor when enabled | 1.0 / 0.5 | 1.0 / 0.6 | 1.0 / 0.75 | 1.0 / 0.85 |
| Shadow map per cascade | 1024 | 1536 | 2048 | 4096 |
| PCSS cascades (rest PCF) | 0 | 1 | 2 | 4 |
| Far cascade refresh | every 4 frames | every 3 | every 2 | every frame |
| Shadow distance | 60% | 80% | 100% | 100% |
| Small caster cut (texels) | 6 | 4 | 2.5 | 1.5 |
| GTAO | half res | half res | half res | full res |
| SSR | off | 16 steps | 32 steps | 48 steps |
| Contact shadows | off | 8 steps | 10 steps | 10 steps |
| Lights per frame (closest) | 512 | 2048 | 4096 | all |
| Shadowed spot lights | 0 | 2 | 4 | 4 |
| Texture cap (normal/ORM maps: half, below Ultra) | 1024 | 2048 | 2048 | 4096 |
| LOD detail | 0.5 | 0.75 | 1.0 | 1.5 |
| Depth of field (cutscenes, photo mode) | off | off | on | on |
| TAA, bloom, camera stage (grade, grain, aberration) | on | on | on | on |
| Motion blur (samples) | off | 6 | 10 | 16 |
| Volumetric fog (froxels at 1080p, range) | off (analytic) | 120x68x48, 120 m | 160x90x64, 160 m | 240x135x96, 200 m |
| GTAO bent normals (cone specular occlusion) | off | on | on | on |

**Auto** (default `auto = true`, `GraphicsConfig::for_adapter`): discrete
GPUs get High, except old, entry-level or small-VRAM ones (GTX 9xx/10xx/1650,
RX 4xx/5xx, MX, RX 6400/6500, RTX 3050 laptop...) which get Medium; strong
integrated GPUs (Radeon 780M/880M/890M, Apple M) get Medium, other
integrated GPUs, WARP / lavapipe and unknown devices Low. **Intel integrated
graphics** (vendor 0x8086: UHD, Iris Xe, the Arc Xe-LPG of Meteor/Lunar
Lake laptops) get their own profile: the Medium look at the panel's native
resolution (no pixel budget, dynamic resolution off), 1536 shadow maps with one PCSS
cascade and the far cascades every 4th frame, 12-step SSR, 2048 lights, one
shadowed spot, no motion blur, no froxel fog (the analytic fog and lamp
glow stay), bent normals on, FSR 1 on.

**Resolution (v0.1.4)**: every preset and adapter profile renders at 100%
of the window's physical pixels, with TAA and no upscaling. The window
opens borderless at the monitor's native resolution (or windowed /
exclusive fullscreen, options menu) and the process is per-monitor DPI
aware, so a 2880x1800 panel at 200% scaling gets a 2880x1800 swapchain
(the UI is drawn at the same scale factor and stays crisp). The startup
log prints the monitor, its scaling and the window's physical size, and
the internal resolution is logged at startup and on every change.

v0.1.3 capped the internal image per preset (Low 1.0 MP ... Ultra 8.3 MP,
Intel iGPUs 1.2 MP) and ran dynamic resolution from the frame time when
GPU timestamps were missing: on the Galaxy Book4 Pro (Core Ultra 5 125H,
Arc 7 Xe-cores, 2880x1800) that rendered ~0.48 x native and could drop to
0.35, which looked blurrier than v0.1.2 and gave no frame rate, because
the frame was not limited by pixels. That cap is gone; old
`graphics.toml` files (no `version`) have their resolution settings reset
to native when loaded.

**Render scale** (options: 100 / 85 / 75 / 67 / 50%): below 100% the last
pass upsamples with FSR 1 EASU. Presets do not change it.

**Dynamic resolution** (off by default): it only lowers the scale when the
**GPU is the bottleneck**: the smoothed GPU time from timestamp queries
over 90% of the `target_fps` budget *and* the frame's CPU work (game +
extraction + renderer + UI, without the swapchain wait) under it, for
~0.25 s; then towards the pixel-count estimate, at most 15% at a time, in
5% steps, never under the preset's floor. It rises 5% after ~1 s under
68% of the budget. CPU-bound frames never lower it (fewer pixels would
only blur them), and without timestamps it stays at the render scale
(the frame time is not used as a stand-in). F11 shows `CPU-bound` /
`GPU-bound` with both times, so the bottleneck is visible on the machine.

## Budget on a mid-range GPU (1080p, GTX 1660 / RX 580)

Estimates in ms from the measured triangle and pixel counts above, the pass
proportions measured on lavapipe and typical throughput of this class of GPU
(~1.5-2 G depth-only triangles/s, ~5 TFLOPs). Check them with F11 on real
hardware; the DRS keeps the frame on budget if they are optimistic.

| Pass | Low (0.75) | Medium (0.9) | High (1.0) | Ultra (1.0) | Intel Arc iGPU, 2880x1800 (1.2 MP) |
|---|---|---|---|---|---|
| Sun shadows (amortized) | 0.8 | 1.6 | 2.6 | 5.5 | 2.2 |
| Spot shadows (night only) | 0 | 0.3 | 0.6 | 0.6 | 0.4 |
| Prepass | 0.6 | 0.9 | 1.2 | 1.6 | 1.0 |
| GTAO | 0.2 | 0.25 | 0.3 | 1.0 | 0.3 |
| Light clusters | 0.05 | 0.05 | 0.05 | 0.1 | 0.05 |
| Main + sky + sprites | 1.8 | 3.2 | 4.5 | 5.5 | 3.2 |
| TAA | 0.15 | 0.25 | 0.3 | 0.3 | 0.25 |
| Eye adaptation (night) | 0.05 | 0.05 | 0.05 | 0.05 | 0.05 |
| Bloom | 0.2 | 0.3 | 0.35 | 0.35 | 0.3 |
| Tonemap + final + camera stage | 0.3 | 0.4 | 0.45 | 0.45 | 0.3 + 0.9 (camera stage + EASU at 5.2 MP output) |
| Depth of field (only when a shot sets it) | 0 | 0 | 0.8 | 0.8 | 0 |
| Sky-view LUT (replaces the per-pixel sky raymarch) | 0.02 | 0.02 | 0.02 | 0.02 | 0.03 |
| Volumetric fog (inject + integrate) | 0 | 0.25 | 0.5 | 1.1 | 0 |
| Motion blur (moving frames) | 0 | 0.3 | 0.45 | 0.7 | 0 |
| UI (egui at native resolution) | 0.1 | 0.1 | 0.1 | 0.1 | 0.3 |
| **Total day / night** (no DOF) | **~4.2 / 4.3** | **~7.8 / 8.2** | **~10.9 / 11.6** | **~16.7 / 17.4** | **~9.1 / 9.5** |
| FPS (GPU bound) | 200+ | 120+ | ~90 (60 with headroom) | ~60 on this class; for 3070+ | ~100 GPU-bound; 60 with DRS headroom |

The **Intel Arc iGPU column** (Meteor Lake Core Ultra 5/7, 7-8 Xe-LPG
cores, ~4.5 FP32 TFLOPs but ~90-120 GB/s of LPDDR5X shared with the CPU)
scales the per-pixel passes by the pixel count (1.2 MP vs 2.07 MP) and
the bandwidth (x1.3-1.5 against a GTX 1660), keeps the geometry passes at
Medium's triangle counts on a ~1.2 G triangles/s rasterizer, and adds the
two passes that run at the 5.2 MP output (camera stage with EASU, egui).
The ~2x margin under 16.6 ms is for the shared memory and thermal limits
of a thin laptop; dynamic resolution (GPU timestamps, which Intel's DX12
driver exposes) drops toward 0.35 x native before the frame time does.
v0.1.2 on the same machine rendered ~4.2 MP (Medium at 0.9 of 2880x1800)
with 2048 shadow-map-class costs: by the same model ~25-30 ms of GPU plus
the CPU cost of ~9 k draws, consistent with the ~10 FPS reported. Whether
the CPU side (draw submission, ~2 k camera + ~3-6 k shadow draws) also
limits it on a 15-28 W laptop CPU must be read from F11 on the machine;
world streaming / HLOD is being handled separately.

*Since v0.1.4 the Intel profile renders at native resolution* (the 1.2 MP
column above is the v0.1.3 budget): at 5.2 MP the per-pixel rows grow
~4x when the GPU is the limit, which is then visible in F11 as
`GPU-bound`, and the player can lower the render scale or enable the
GPU-only dynamic resolution. v0.1.3 showed that lowering the pixels did
not raise the frame rate on that laptop, i.e. the frame is CPU-bound
there (draw submission, culling, game systems), so the pixels are spent
on image quality instead.

Measured shares of the new passes (lavapipe, default map at dusk, 960x540
fixed, 10 frames; only proportions are meaningful): sky LUT 0.2-0.5% of
the GPU sum, volumetric fog 0.9% (Medium) / 0.7% (High) / 1.3% (Ultra),
motion blur 0.4-0.8%. With High the `main+sky` pass went from 1288 ms
(per-pixel sky raymarch) to 939 ms + 11 ms of LUT: -27% at dusk with open
sky, the sky LUT being the largest win of this pass on every GPU.

CPU per frame, measured on the shared 4-core VM (upper bounds for a
desktop CPU): game systems 3-5 ms (player movement, physics, crowd),
extraction < 0.1 ms (the static world is no longer copied per frame), and
the engine's own render work (setup, static cull, upload, encode) 10-18 ms
here; the rest of the measured renderer time is lavapipe's `submit`
(software rasterization), which a hardware driver does not have. On a
desktop CPU the engine side should be ~4-7 ms, i.e. not the bottleneck at
60 FPS.

## Organic default map budget (v0.1.2)

The organic default map (Austrias and Carmen old towns on hills, crossed by
avenues, plus a Madrid grid, a Valencian Ensanche and the boulevard)
measured headless on the shared 4-core VM (lavapipe, other jobs running).
`sandbox` now logs the world build stages, a GPU upload estimate per
texture cap, the real upload and the resident memory.

| | Before (first organic merge) | After (v0.1.2) |
|---|---|---|
| Placements / instanced triangles | 183 807 / 103.7 M | 146 237 / 85.7 M |
| Unique scenes | 10 285 (8 372 window modules) | 5 236 (3 491 window modules) |
| Mesh memory (vertex + index + LOD) | 841 MB | 693 MB |
| Textures on High | 1 344 MB world + ~700 MB people/cars = 2.05 GB | 1.45 GB total |
| VRAM estimate on High at 1080p (F11) | ~3.3 GB | ~2.3 GB |
| Resident RAM after load | 4.4 GB before upload, +2 GB staging spike, CPU copies kept | 2.8 GB after build, ~3.8 GB incl. lavapipe's GPU memory |
| Peak of a headless run (RAM + lavapipe "VRAM") | > 6.8 GB (killed) | 4.6-4.9 GB |
| World build (this VM, loaded) | 45-50 s (city) / 68-134 s total | 31-39 s (city) / 40-47 s total |
| Camera / shadow triangles (plaza spawn, High) | n/a (did not fit) | 1.4 M / 3.2 M amortized (8.3 M all cascades) |
| Camera / shadow draws | n/a | 2.1 k / 3.6-6.6 k |
| Static grid | n/a | 269 700 parts in 3 423 cells, built in 0.17 s |

What changed:

1. **Shared window modules**: opening sizes snap to 10 cm, and a shutter
   module only keys on what it draws (roller blinds have no colour, only
   alicantinas hang over the balcony): 8 372 -> 3 491 unique window
   meshes for the same placements.
2. **No CPU copies after upload**: the world's scenes lose their vertex,
   index and texture data as soon as the renderer has them (the game never
   reads them back), the renderer forgets cached texture sources nobody
   holds (`Renderer::prune_texture_cache`), and freed heap is returned to
   the OS. Uploads are flushed every 64 MB (`Renderer::flush_uploads`):
   `create_buffer_init` / `write_texture` staged the whole city in host
   memory until the first submit.
3. **Data maps at half the cap**: normal and ORM maps use half the
   preset's texture cap below Ultra (`forja_render::texture_cap`); High's
   cap is 2048 (no world texture is larger). -0.6 GB of VRAM on High.
4. **Smaller grids, same old towns**: the Madrid grid is 3x4 blocks (was
   4x5) and the Ensanche 2x3 (was 3x3); both old towns, their alleys,
   stairs, plazas and avenues are unchanged.
5. **Overlapped loading**: the kit (decoding and weathering the scans,
   ~10 s) loads while the plan and fabric are generated, and the content
   library while the city is meshed and the trees grown.
6. Rebuilding the city at dusk/dawn (F3) frees the old map first.

Budget on a GTX 1660 / RX 580 (6 GB) at 1080p High: ~2.3 GB VRAM (meshes
0.7, textures 1.45, targets and shadow maps ~0.15), ~3 GB RAM. GPU frame
estimate as in the table above (shadows 2.6 ms amortized with ~3.2 M caster
triangles, main pass 4.5 ms): ~10 ms, i.e. 60 FPS with headroom; DRS
covers the rest. Remaining risks: facade windows are still 60 % of the
shadow triangles (an HLOD facade proxy for the far cascades would remove
most), and the load is CPU-bound (city meshing and tree generation are
single-threaded per stage; a disk cache of the kit and trees would cut
~20 s).

## Shadow proxies, load caches, BC textures (after v0.1.2)

Measured headless on the default map (plaza spawn, High, 960x540,
lavapipe) on the shared 4-core VM; other agents were building and
rendering at the same time, so wall-clock times vary by up to 2x between
runs (each figure is one run; the *before* column is the v0.1.2 binary).

| | v0.1.2 | Now |
|---|---|---|
| Shadow triangles, all 4 cascades (plaza) | 8.22 M | 1.80 M |
| Shadow triangles, all 4 cascades (aerial over the old town) | 8.46 M | 1.41 M |
| Shadow draws (plaza / aerial) | 6 656 / 6 609 | 2 253 / 2 228 |
| lavapipe `shadow c3` (plaza / aerial) | 637 / 469 ms | 95-127 / 112 ms |
| Heaviest shadow caster | `window` x60 596, 4.90 M | `block` x279, 0.46 M |
| Scanned materials + procedural kit textures | 11.5-27.4 s | 3.1-5.8 s (cache) |
| Trees (18 models) | 11.4-12.7 s | 0.3-0.6 s (cache) |
| Fabric: blocks and parcels / fillets / surfaces | 4.25 / 2.56 / 4.13 s | 0.47 / 0.23 / 1.35 s |
| City built (`city: built`) | 37-54 s | 18-21 s (warm cache) |
| World built incl. people, cars, traffic | 44-69 s | 26-27 s (warm cache) |
| GPU upload of the city | 11.3-13.9 s | 5.6-6.7 s (warm), 16 s (first run: BC encoding) |
| Textures in GPU memory (High) | 1 450 MB | 542 MB |
| VRAM estimate (F11, High, 960x540) | 2 243 MB | 1 333 MB |
| Resident after upload (lavapipe keeps "VRAM" in RAM) | 3.75-3.80 GB | 2.7-2.8 GB |
| Peak of a headless run | 4.5-4.6 GB | 3.7 GB |
| Window during the load | none (console only) | loading screen at once |
| F3 at dusk/dawn | full freeze: build + upload (~45-80 s here) | game keeps running; stall = upload (~6 s here) |

Images: day and aerial views are unchanged apart from BC1 block noise
(mean absolute difference 1.1-1.5 of 255 against v0.1.2). Screenshots in
`engine/screenshots/agents/perf2/`: plaza by day and night, the whole
city by day and night (HLOD tiles with lit windows), detail vs HLOD from
650 m, the aerial before/after the shadow proxies, a facade RGBA8 vs
BC1, and the Carmen map after a dusk switch.

What changed:

1. **Shadow proxies.** `forja_assets::ShadowRole` marks a primitive
   `All`, `Near` (camera and near cascades) or `Proxy` (far cascades only,
   never in the camera). The city marks every facade module (windows,
   shutters, balconies, shops, portals, lanterns: whatever `ModuleLib`
   makes inside `build_into`/`medianeras`) and the block walls, mouldings,
   plinths and party walls `Near`; roofs stay `All`; and each 64 m tile
   gets a `block_shadow` scene: per building a closed prism of its
   footprint from below the plinth to the eave (plus the ático's setback
   block), ~20-60 triangles instead of thousands. The renderer skips
   `Near` runs from the first proxy cascade on and `Proxy` runs before it
   (static grid: one flag per run, so no per-instance work; dynamic
   instances per part). `GraphicsConfig::shadow_proxy_cascade`: Low and
   Medium from c2, High and Ultra from c3 (F9: "Building shadow proxies
   from c1/c2/c3/off"). Small casters were already skipped by size in
   texels; the proxies remove the rest of the facade detail where it is
   sub-texel.
2. **Versioned disk cache** (`forja_assets::cache`, folder `cache/` next to
   `data/`, `FORJA_CACHE_DIR`, `FORJA_NO_CACHE=1`). Keys are SHA-256 of the
   generating code itself (`include_str!` of the sources), the parameters
   and the name/size/mtime of every input file, so a code or asset change
   regenerates the entry; files carry a format version, their key, length
   and a checksum, are written to a temporary file and renamed, and
   anything that does not match is ignored (a damaged file is deleted).
   Kinds: `kit` (all scanned sets after decoding and weathering plus the
   procedural kit textures, 430 MB: pixel buffers delta-filtered and
   deflated in parallel), `trees` (the 18 generated tree models with
   impostors, 76 MB), `textures` (BC mip chains, ~170 MB). Shared pixel
   buffers stay shared after loading. City meshes are not cached (the
   layout, traffic, colliders and spawns would all need a format); their
   build was parallelised instead.
3. **Parallel city build.** Fabric: blocks are parcelled in parallel
   (numbered afterwards in the same order), setbacks and lot clipping per
   lot, kerb fillets per junction, ground surfaces per class and street
   group; a repeated region difference is computed once. Realize: block
   scenes are assembled and module LOD chains simplified in parallel at
   the end of the city (same results, same order). Texture upload
   prepares a scene's textures (mips, BC) in parallel, a few at a time.
4. **Loading screen.** `forja_core::App` is now `Send` (resources and
   systems must be `Send`), so the game builds the world on a worker
   thread while the window shows the stage and a progress bar
   (`forja_assets::progress`); the GPU upload then runs in 40 ms slices
   under the same screen.
5. **Dusk and dawn without a freeze.** The night content still changes the
   geometry (lit flats, closed shutters, lamps), so the other city is
   built on the worker thread while the current one keeps running; when it
   is ready the old world is freed, the new one uploaded under the loading
   screen, and the clock, the camera and the player's position carry over.
   Peak RAM during that build is the running world plus one build (~1.5 +
   2.7 GB on a real GPU; on lavapipe the default map does not fit this
   shared VM, `--night-switch` measures it on smaller maps). The build
   runs in a rayon pool of `cores - 2` threads so the game keeps cores.
   Measured on the Carmen map (lavapipe, 640x360): the night city was
   built in the background in 31 s while the day city kept running (on
   lavapipe the 4-core VM shared the CPU between the build and the
   software rasteriser, so only one frame finished meanwhile; with a GPU
   the frame loop does not rasterise on the CPU); the main-thread stall
   at the swap was the upload, 6.9 s here (BC chains from the cache), plus
   the first frame. Before, the same crossing froze the game for the whole
   build plus upload (45-80 s on the default map here).
6. **BC textures.** Colour maps and packed ORM/occlusion maps are uploaded
   BC1 (BC3 when they have alpha) when the adapter has
   `TEXTURE_COMPRESSION_BC` (requested at device creation; DX12/Vulkan
   desktop GPUs and lavapipe do); normal maps stay RGBA8 (the shader's
   specular anti-aliasing reads the unnormalised filtered length). Encoder:
   `texpresso` (pure Rust, range fit), cached on disk, so later loads also
   skip the CPU mip generation. `FORJA_FX=-bc` keeps RGBA8.

Tools: `sandbox --night-switch` (headless: after the screenshot, cross
dusk as the game does and report the background build, the frames
rendered meanwhile and the main-thread stall; saves `<shot>_switched.png`),
`FORJA_TIMING=1` (fabric stage laps).

## Shadow stability, city HLOD and streaming (integrated GPUs)

Target: the user's Galaxy Book4 Pro (Meteor Lake, integrated Arc, shared
memory) ran v0.1.2 at ~10 FPS, and the shadows of the iron window grilles
flickered. Render scale and upscaling are handled separately (gfx3); this
part cuts what the city costs and makes the shadows stable.

### Shadow flicker

Causes found in the code (the cascades themselves were already
texel-snapped, with rotation-invariant bounding spheres, receiver-plane
bias, normal offset and a rotated Poisson PCF resolved by TAA):

1. **Shadow LOD followed the camera.** Casters picked their shadow LOD from
   the camera distance of their grid cell (one level coarser than the
   camera), so while walking, facade modules switched LOD *inside* the
   same cascade and their shadows popped (grilles and railings lose bars
   at coarse levels). Now the shadow LOD comes only from the cascade's
   texel size (one triangle per two texels), which does not change while
   the camera moves (`View::stable_lod`; dynamic instances the same).
2. **Thin casters in coarse cascades.** Grille and railing bars (2-3 cm)
   are thinner than a texel beyond the first cascade (c1 ~1.2 cm, c2
   ~3.5 cm, c3 ~7 cm on High), where they only alias. The facade's iron
   (`Mat::Iron` inside `build_into`) is now `ShadowRole::Fine`: camera,
   first cascade and spot shadows only (the GTA V approach of simplified
   casters). Beyond ~10 m their shadow is a sub-texel pattern anyway; the
   balcony slabs, the walls and the building masses still cast.
3. Far cascades use the building masses (above), so window modules no
   longer rasterise at 7 cm texels.

`FORJA_FX=-stableshadow` restores the old behaviour for A/B comparisons.
Measurement: `sandbox --stability N` (8-frame means remove the TAA,
GTAO and PCF noise; then N steps of the sun creeping 0.03° per step, and
the camera walking 3 m away and back). Plaza spawn, High, 480x270:

| 960x540, 4 steps | Noise floor (changed px) | Sun creep: mean / p99.9 / changed px | Walk 3 m away and back: changed px |
|---|---|---|---|
| Before (`-stableshadow`) | 0.14 % | 0.96 / 23 / 0.64 % | 0.16 % |
| After | 0.14 % | 0.92 / 16 / 0.39 % | 0.15 % |
| 480x270, 6 steps, before / after | 0.16 % | 0.43 % / 0.41 % changed | 0.07 % / 0.07 % |

The sun-creep flicker (pixels that change by more than 8/255 between
steps beyond the noise floor) drops by ~40 % at 960x540; at 480x270 the
grille bars are sub-pixel and both builds read the same. Walking away
and back already returned to the same image at this pose (texel-snapped
cascades); the LOD fix matters where casters change level within a
cascade while walking (street-level runs of facade modules). After these
runs the shadow maps also follow the sun in 0.1° steps (`shadow_sun`):
with a moving clock the maps stay identical for several frames instead of
re-rasterising every frame (the lighting keeps the exact sun).

### HLOD per city tile

Every 64 m city tile gets an HLOD scene (`forja_city::hlod`): each
building as one box (footprint to the eave, plus its ático), one mesh,
one material and a 512² atlas baked at build time with each street
facade's wall colour (mean of the scanned material x tint) and its real
grid of storeys and bays, shop fronts on the ground floor, roof colours,
and at night an emissive copy with lit windows. The renderer draws the
tile's detail (its block scene, window/shop/portal modules, shadow
masses) near the camera and the HLOD beyond `hlod_distance` (Low 150 m,
Medium 200, High 250, Ultra 400; F9 "City detail distance"), per tile with
10 % hysteresis, in the camera and in every shadow cascade
(`Renderer::set_static_instances_lod`, `StaticLod`; one flag per run, so
no per-instance cost). 70 tiles on the default map, 9 MB of HLOD meshes
and 70 atlases.

### Streaming

`sandbox::stream`: at the first upload every tile's block scene (walls,
mouldings, roofs: 313 MB of the 710 MB of meshes) is written to
`cache/stream/<process>/` by a writer thread; beyond 1.4x the HLOD
distance a tile's blocks leave the GPU (`Renderer::release_model`) and
the tile is held at its HLOD (`set_static_group_ready`); within 1.2x a
worker reads and decodes them and the main thread re-uploads at most two
tiles per frame, so a tile is resident before the 1.0x / 0.9x switch and
nothing pops or stalls. Window and shop modules are shared by all tiles
(223 MB) and stay resident. `FORJA_NO_STREAM=1` keeps everything.

Measured (lavapipe, 960x540, High, same build with `FORJA_FX=-hlod`):

| View | | Detail everywhere | HLOD + streaming |
|---|---|---|---|
| Plaza spawn | camera draws / tris | 2 074 / 1.22 M | 2 074 / 1.22 M |
| | shadow draws / tris | 2 130 / 1.85 M | 2 130 / 1.85 M |
| | tiles at HLOD / blocks streamed out | 0 / 0 | 32 / 29 (153 MB) |
| | resident meshes, VRAM estimate | 710 MB, ~1.37 GB | 556 MB, ~1.21 GB |
| Old-town aerial (60 m up) | camera draws / tris | 4 943 / 4.69 M | 4 828 / 4.53 M |
| | resident meshes, VRAM estimate | 710 MB, ~1.37 GB | 569 MB, ~1.23 GB |
| Whole city (650 m away, 180 m up) | camera draws / tris | 12 226 / 14.88 M | 1 357 / 1.07 M |
| | shadow draws / tris | 10 576 / 9.62 M | 3 062 / 1.84 M |
| | lavapipe GPU per frame (proportion) | 7.1 s | 1.5 s |
| | resident meshes, VRAM estimate | 710 MB, ~1.40 GB | 406 MB, ~1.07 GB |

Budget for the integrated GPU (draws < 1.5 k, < 2 M triangles camera +
shadows): met by the far views; near the ground the detail within the
HLOD distance (window modules: 3 491 unique meshes, one draw each when
visible) keeps ~2 k camera draws. Next steps there: fewer module
variants (paint colours as instance tints instead of materials), merged
per-tile batches of the modules at mid distance, and a shorter HLOD
distance plus proxies from c2 in the integrated-GPU preset.

## Moving and changing things

The static culling grid holds only what never changes. Traffic AI cars,
parked cars, the player, pedestrians and props are dynamic instances
(culled per instance every frame). Traffic signal heads are static
placements whose model switches with the AI's aspect: the game lists them
in `RenderScene::volatile_statics`, so they stay out of the grid and are
submitted every frame. Night lights (`FrameDesc::local_lights`, city
content lights within 600 m) are capped per preset ("lights per frame",
closest first) before clustering.

## Frame-loop stalls (v0.1.3 at ~10 FPS)

v0.1.2 and v0.1.3 ran at about 10 FPS on a Core Ultra 5 125H with an Arc
iGPU (2880x1800) and just as slowly on a desktop RTX 3070, and a lower
internal resolution did not help. A cost that does not move with the GPU
or the resolution is CPU work or waiting on the frame path, so the frame
loop was audited stage by stage with `--frame-trace` (Linux lavapipe and
Windows D3D12 WARP in CI, at tiny resolutions such as 64x36 and 160x90 so
the software rasteriser does not hide the CPU side).

### What was found and fixed

| Cost (before) | Fix |
|---|---|
| `CommandEncoder::finish` 9–10 ms and pass recording on the main thread | Shadow, prepass and main passes are recorded and finished in parallel on worker threads (the main pass split into chunks); indirect-draw validation off in release. |
| Static culling 6–12 ms per frame (3.5 k cells, 270 k parts, per view) | Camera and shadow views culled in parallel, dynamic instances culled on a worker at the same time; all views' instances uploaded in one staging copy. |
| One D3D12 staging resource created and freed per small `queue.write_buffer` (~20 per frame) | `FrameWrites`: the frame's small writes go through one staging buffer and one copy command buffer submitted first. |
| `device.poll(wait)` on the frame path | Removed; uploads after a world swap run under the loading screen in 40 ms slices; 0 upload flushes during play. |
| Fixed-step "spiral" (up to 8 steps per slow frame) | 10 ms budget for fixed steps per frame. |
| 370–490 ms first frame of play (static grid build) | The grid is built under the loading screen. |
| ~290 ms first frame after a load or a dusk/dawn swap (HDRI decode and conversion on the main thread) | The environment map is loaded under the loading screen and kept for the process: a swap to the same photo only uploads it. |
| Pipelines recompiled at every dusk/dawn swap (seconds of FXC) | `clear_scene` keeps them. |
| Dusk/dawn build competing with the frame | Builds and tile loaders run below normal priority on a pool of cores − 2. Threads they spawn inherit it (`forja_assets::threads::inherit`; Windows does not do it by itself), rayon's global pool runs below normal, and the frame's parallel work (culling, pass recording, texture preparation) runs on a pool of its own (`frame_pool`), so a frame job never queues behind a build job. |
| Streaming reload of a large tile in one frame (the 8 MB budget accepted a whole tile) | Tiles are re-uploaded model by model within the 8 MB per-frame budget; the tile shows its detail when all are in. A dropped streamer no longer joins its writer thread on the main thread. |
| Allocator and per-frame allocations | mimalloc; reused scratch lists. |

Also checked and not a cost: the only sleep is the optional frame cap (off
by default, high-resolution timer); present takes 0.1–0.2 ms; logging is
2 lines per frame trace; there are no map_async readbacks or error-scope
pops on the frame path (timestamps are read back a few frames late).

### Measured (CPU of the main thread per frame, frames 40–59)

| | Before 83453c6 | 83453c6 | 9d1e18a | Final |
|---|---|---|---|---|
| VM plaza High 160x90 (lavapipe): main thread | ~50 ms | 16.7–21 ms | 4.6–5.5 ms | 5.05 ms |
| render CPU / `finish` | 49.7 / 31.9 ms | 15–17 / 10–11 ms | 7.6 / 1.0 ms | 6.5 / 0.9 ms |
| sim | 7.4 ms | 1.0–1.3 ms | 1.0 ms | 0.95 ms |

Windows WARP (CI, same runner A/B), CPU without WARP's own submit: 4.1 →
2.6 ms headless and 4.1 → 3.3 ms windowed (Spain, Low, 64x36). Images are
bit-identical before and after on the VM (plaza luma 0.414), 0 GPU errors.

### What remains

Wall-clock frame time on WARP and lavapipe did not change: those frames
are limited by GPU work that does not depend on resolution, mostly the
shadow cascades (WARP High: c2 and c3 ~1 s each of a 2.6 s frame; VM plaza
High: c0–c3 ~430 of ~770 ms; 3.1 M shadow triangles vs 0.9 M for the
camera). That matches "a lower resolution did not help". Shadow work is
owned by the shadow-flicker and draw-call teams: caching static casters
per cascade, refreshing far cascades every N frames on every preset, HLOD
only in c3 and shadow-map size per preset are the candidates.

Not confirmed on real hardware yet: run `sandbox.exe --frame-trace 60` on
the slow machines and read the acquire/render/finish split and the GPU
time per pass before calling it fixed. Other open items: each dynamic-
resolution step recreates ~20 render targets (5–40 ms, only on GPU-bound
frames); the dusk/dawn swap holds two worlds (>7 GB peak on the VM; the
`ns` CI job logs the peak working set); the Low preset's upload takes
37 s on the VM and ~105 s on WARP and is repeated at every swap
(`clear_scene` drops the texture cache).

## Windows smoke test (D3D12 WARP)

The release workflow (`.github/workflows/release.yml` in the
Inhouse-Game-Engine repo; dispatch with `smoke_only = true` for a run
without packaging) builds the game, fetches the content packs (cached),
and runs `sandbox --smoke 100 --adapter warp --preset low --size 960x540`
headless and `--smoke 30 --window` through a real swapchain, both on the
D3D12 WARP software adapter of the runner; logs and screenshots are
uploaded as the `Forja-Windows-smoke` artifact. First run (this branch):
both passed, 0 GPU errors, DX12 pipeline compilation (FXC) 14 s, upload of
the default map 122 s, ~2.4 s per WARP frame.

## Remaining risks for real-GPU testing

- GPU estimates are not measurements: run the Windows build with F11 on a
  GTX 1660 class card and check `shadow c*`, `prepass` and `main+sky`.
- Draw calls: 2.9 k camera + ~6 k shadow draws on the default map (the
  procedural kit has one mesh per building module variant, so instancing
  merges less than it could). wgpu on DX12 records these in ~5 ms of CPU
  here; if it shows on real CPUs, merge static geometry per grid cell
  (HLOD) or move to indirect draws.
- Shader compilation: pipelines are created at startup (no hitches during
  play), but DX12 compiles them on first load (FXC/DXC); a pipeline cache
  is not wired in yet.
- Textures are BC1/BC3 (range fit); normal maps are still RGBA8 (BC5 would
  need the specular AA to work from the two-channel normal). A BC7 encoder
  would improve the colour maps' quality.
- The near cascades still draw the block walls without LOD (`block`,
  ~0.46 M shadow triangles at the plaza).
- DX12 compiles every pipeline with FXC at startup (14 s on WARP in CI,
  a few seconds on a real driver); DXC (`Dx12Compiler::DynamicDxc`) or a
  pipeline cache would shorten it.
- Upscaling is spatial (FSR 1 EASU after TAA); a TAAU resolve at output
  resolution would be sharper at 50% scale (research list item 16).
