# Performance and graphics presets

How the renderer spends a frame, what each quality preset changes, and the
budget it is designed for. Target: **1080p at 60 FPS or more on a mid-range
GPU (GTX 1660 / RX 580 class) with the High preset**, with dynamic
resolution absorbing the heavy views.

## Tools

| Tool | What it does |
|---|---|
| **F11** in the game | Frame profiler: FPS, frame-time graph (CPU white, GPU green, lines at 16.7 and 33.3 ms), internal resolution and render scale, draw calls, triangles and instances (camera and shadows), lights, static grid size, cascades refreshed this frame, VRAM estimate, CPU time per game system and per renderer scope, GPU time per pass, the heaviest scenes by triangles, and a CSV logger. |
| **F9** in the game | Graphics options (preset, auto, every cost knob below). Applies at once except the texture cap (next map); *Save* writes `data/graphics.toml`. |
| `sandbox --bench N [--preset P] [--csv f.csv]` | Headless: N real-time frames after warm-up; prints mean/p50/p95 frame time, per-system CPU, renderer scopes, GPU passes, counts and the heaviest scenes (camera and shadow). |
| `FORJA_LEGACY_STATICS=1` | Sandbox: resubmit the whole world every frame as before the static grid (before/after comparisons). |
| `FORJA_PRESET=low\|medium\|high\|ultra` | Any tool or example: render with that preset. |
| `FORJA_PROFILE_CSV=f.csv` | Game: log every frame to a CSV from startup. |
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
| 7 | Tonemap, CAS/FXAA + upscale | internal -> output | The final pass upsamples when render scale < 1. |

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
| Render scale (max / min, dynamic) | 0.75 / 0.5 | 0.9 / 0.6 | 1.0 / 0.75 | 1.0 fixed |
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

**Auto** (default `auto = true`): discrete GPUs get High, except old or
entry-level ones (GTX 9xx/10xx, RX 4xx/5xx, MX, RX 6400/6500...) which get
Medium; strong integrated GPUs (Radeon 780M/880M/890M, Arc, Apple M) get
Medium, other integrated GPUs and software rasterizers Low.

**Dynamic resolution**: the GPU frame time (timestamps; frame time when
unsupported) is smoothed, and the internal scale drops (towards the
pixel-count estimate, at most 15% at a time, in 5% steps) after ~0.25 s over
90% of the frame budget, and rises 5% after ~1 s under 68% of it. TAA
accumulates at the internal resolution and the final CAS pass upsamples, so
0.75 still looks sharp.

## Budget on a mid-range GPU (1080p, GTX 1660 / RX 580)

Estimates in ms from the measured triangle and pixel counts above, the pass
proportions measured on lavapipe and typical throughput of this class of GPU
(~1.5-2 G depth-only triangles/s, ~5 TFLOPs). Check them with F11 on real
hardware; the DRS keeps the frame on budget if they are optimistic.

| Pass | Low (0.75) | Medium (0.9) | High (1.0) | Ultra (1.0) |
|---|---|---|---|---|
| Sun shadows (amortized) | 0.8 | 1.6 | 2.6 | 5.5 |
| Spot shadows (night only) | 0 | 0.3 | 0.6 | 0.6 |
| Prepass | 0.6 | 0.9 | 1.2 | 1.6 |
| GTAO | 0.2 | 0.25 | 0.3 | 1.0 |
| Light clusters | 0.05 | 0.05 | 0.05 | 0.1 |
| Main + sky + sprites | 1.8 | 3.2 | 4.5 | 5.5 |
| TAA | 0.15 | 0.25 | 0.3 | 0.3 |
| Eye adaptation (night) | 0.05 | 0.05 | 0.05 | 0.05 |
| Bloom | 0.2 | 0.3 | 0.35 | 0.35 |
| Tonemap + final + camera stage | 0.3 | 0.4 | 0.45 | 0.45 |
| Depth of field (only when a shot sets it) | 0 | 0 | 0.8 | 0.8 |
| **Total day / night** (no DOF) | **~4.1 / 4.2** | **~7.2 / 7.6** | **~9.8 / 10.5** | **~14.8 / 15.5** |
| FPS (GPU bound) | 200+ | 130+ | ~100 (60 with headroom) | ~65 on this class; for 3070+ |

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

## Moving and changing things

The static culling grid holds only what never changes. Traffic AI cars,
parked cars, the player, pedestrians and props are dynamic instances
(culled per instance every frame). Traffic signal heads are static
placements whose model switches with the AI's aspect: the game lists them
in `RenderScene::volatile_statics`, so they stay out of the grid and are
submitted every frame. Night lights (`FrameDesc::local_lights`, city
content lights within 600 m) are capped per preset ("lights per frame",
closest first) before clustering.

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
- No BC7/BC5 texture compression yet (see above).
- Facade windows are now the largest shadow cost (75 k instances,
  5.6 M triangles on High). Medium and Low cut them with the small-caster
  threshold; on High a merged per-building facade proxy for the far
  cascades (HLOD) would be the next step.
