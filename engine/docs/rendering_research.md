# Efficient cinematic rendering: research notes for Forja

These are the techniques from published papers and talks that do the most
for Forja's look per millisecond. Each one is weighed against what the
renderer already has (depth/normal prepass, GTAO, TAA, PCSS, contact
shadows, specular occlusion, SSR, AgX, bloom).

Costs are for 2560x1440 on an RTX 3070 class GPU. They are estimated from
tap and ALU counts and from the published numbers, so measure them with
timestamp queries on the target. **Status** says whether the technique is
implemented: `done` means it is now in `forja_render`, `existing` means it
was there before this pass, and `next` is a recommendation.

## Ranked list (impact / cost)

| # | Technique | Impact | GPU cost | Status |
|---|---|---|---|---|
| 1 | GGX-prefiltered environment radiance (split-sum) | Very high: removes the mirror-like sky reflections on rough masonry, which caused the "plastic" look | 0 per frame (CPU at load, ~0.3 s) | done |
| 2 | DFG LUT + multiple-scattering energy compensation | High: correct energy on rough surfaces, diffuse/specular energy conservation | 1 extra bilinear tap per pixel | done |
| 3 | Geometric + normal-map specular anti-aliasing | High in motion (no sparkling edges, stable glass and paint) | ~10 ALU per pixel | done |
| 3b | Porous-masonry specular factor + material micro-shadowing (Chan 2018) | High on facades: matte lime render and brick | ~3 ALU per pixel | done |
| 4 | 3D LUT colour grading after the tone curve | High: art-directable "film" colour without touching exposure or the tone curve | ~0.05 ms (shared camera pass) | done |
| 5 | Luminance-dependent temporal film grain | Medium-high: breaks up CG-clean gradients and hides banding | in camera pass | done |
| 6 | Optical cos^4 vignette + lateral chromatic aberration | Medium: photographic lens signature | in camera pass (2 extra taps) | done |
| 7 | Physical thin-lens depth of field (scatter-as-gather) | High for cutscenes and photo mode | ~1 ms, only when enabled | done |
| 8 | Roughness-aware SSR (stochastic GGX + rough fade) | Medium | existing (fades out at r 0.45–0.62) | existing |
| 9 | Bent normals / cone-traced (GTAO) specular occlusion | Medium: better occlusion of sky reflections in street canyons | +0.1 ms (GTAO already marches the horizons) | next |
| 10 | Camera motion blur (McGuire reconstruction filter) | Medium in fast camera moves | ~0.5–0.8 ms with tile max/neighbour max | next |
| 11 | Froxel volumetric fog with shadowed sun (Wronski / Hillaire) | High at dawn and dusk, god rays down streets | 0.6–1.2 ms | next (fog owned by the lighting team) |
| 12 | Sky-view / aerial-perspective LUTs (Hillaire 2020) | Medium: cheaper sky and correct aerial perspective | ~0.2 ms, less than today's per-pixel raymarch | next (lighting team) |
| 13 | Physically based camera exposure (EV100, Lagarde) | Medium: consistent day/night exposure | ~0 | next (lighting team) |
| 14 | Subtle anamorphic-free lens flare / ghosting (bloom-chain based) | Low-medium, easy to overdo | ~0.1 ms | next |

## Why the facades looked like plastic (diagnosis)

These measurements were taken on the `street` example with the
`wide_street_01` HDRI (see `examples/matstats.rs`):

- **The environment mips were box-filtered, not GGX-convolved.** The shader
  picked `lod = 1 + r (max - 1)` on a plain 2x2 box pyramid. At roughness
  0.5 that read a 128x64 map with ~3 degree texels. A real GGX lobe with
  alpha 0.25 is ~20–30 degrees wide and has long tails. Rough stucco and
  stone therefore reflected a sharp, bright sky band, which is the classic
  "wet plastic" sheen. Fix: Karis 2013 split-sum with importance-sampled
  GGX prefiltering (filtered importance sampling, Křivánek & Colbert 2008)
  and Filament's `r (2 - r)` mip mapping.
- **The analytic env BRDF fit has no multiple scattering.** Rough GGX
  loses 10–40 % of its energy, so the diffuse term did not account for
  what the specular layer reflects. Fix: DFG LUT integrated for the
  shader's own height-correlated Smith G, plus Fdez-Agüera 2019 for IBL
  and Kulla-Conty/Filament `1 + F0 (1/Ess - 1)` for the sun.
- **Specular aliasing.** Normal maps at distance (box-filtered mips) and
  curved mouldings produced sub-pixel highlights that TAA turned into
  shimmering. Fix: Tokuyoshi & Kaplanyan 2019 screen-space normal variance,
  plus the Toksvig/vMF variance from the shortened filtered normal.
- **F0 0.04 for every dielectric.** That is right for solid stone and
  glass (IOR ~1.5), but porous masonry (lime render, unglazed brick and
  terracotta, sandstone) traps part of the specular in its pores and
  scatters it diffusely. Fix: a per-material specular factor
  (`KHR_materials_specular`, `MaterialData::specular`) set by
  `forja_city::kit::porous_specular` (0.65-0.9, so F0 ~0.026-0.036).
  Kit roughness is mostly in the right range (0.75-0.95 for masonry);
  `examples/matstats.rs` prints the audit.
- **Smooth sun gradients on rough walls.** The material AO (brick joints,
  render pores) only darkened the ambient term, so grazing sun lit the
  wall as one smooth plastic-looking gradient. Fix: micro-shadowing (Chan
  2018, "Material Advances in Call of Duty: WWII"):
  `shadow *= saturate(|n.l| + 2 ao^2 - 1)`, ~3 ALU.

## References

- B. Karis, "Real Shading in Unreal Engine 4", SIGGRAPH 2013 Physically
  Based Shading course. Split-sum, prefiltered environment maps, analytic
  env BRDF.
- J. Křivánek, M. Colbert, "Real-time Shading with Filtered Importance
  Sampling", EGSR 2008. Pdf-driven source lod.
- R. Guy, M. Agopian, "Physically Based Rendering in Filament" (Google,
  2018–). Roughness-to-lod mapping, energy compensation, DFG LUT.
- C. J. Fdez-Agüera, "A Multiple-Scattering Microfacet Model for Real-Time
  Image-based Lighting", JCGT 8(1), 2019.
  https://www.jcgt.org/published/0008/01/03/
- C. Kulla, A. Conty, "Revisiting Physically Based Shading at Imageworks",
  SIGGRAPH 2017 course.
- D. Chan, "Material Advances in Call of Duty: WWII", SIGGRAPH 2018
  Advances in Real-Time Rendering. Micro-shadowing from material AO.
- S. Lagarde, C. de Rousiers, "Moving Frostbite to Physically Based
  Rendering 3.0", SIGGRAPH 2014 course. Specular occlusion, horizon fading,
  DFG, physical camera.
  https://blog.selfshadow.com/publications/s2014-shading-course/frostbite/s2014_pbs_frostbite_slides.pdf
- A. Kaplanyan, S. Hill, A. Patney, A. Lefohn, "Filtering Distributions of
  Normals for Shading Antialiasing", HPG 2016.
  https://research.nvidia.com/sites/default/files/pubs/2016-06_Filtering-Distributions-of/NDFFiltering.pdf
- Y. Tokuyoshi, A. Kaplanyan, "Improved Geometric Specular Antialiasing",
  I3D 2019, and "Stable Geometric Specular Antialiasing with Projected-Space
  NDF Filtering", JCGT 10(2), 2021.
  https://www.jcgt.org/published/0010/02/02/
- M. Toksvig, "Mipmapping Normal Maps", JGT 2005. C. Han et al.,
  "Frequency Domain Normal Map Filtering", SIGGRAPH 2007 (vMF fit).
- J. Jimenez, "Next Generation Post Processing in Call of Duty: Advanced
  Warfare", SIGGRAPH 2014 Advances in Real-Time Rendering. Scatter-as-gather
  DOF and motion blur, bloom chain.
  https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/
- D. Gustafsson, "Bokeh depth of field in a single pass", 2018 (Tuxedo Labs
  blog). Golden-angle spiral gather (also in Blender Workbench and bgfx).
- J. Jimenez, X. Wu, A. Pesce, A. Jarabo, "Practical Real-Time Strategies
  for Accurate Indirect Occlusion", SIGGRAPH 2016 course (GTAO, bent
  normals, specular occlusion from cones).
- M. McGuire, P. Hennessy, M. Bukowski, B. Osman, "A Reconstruction Filter
  for Plausible Motion Blur", I3D 2012. J. Guertin, M. McGuire,
  D. Nowrouzezahrai, "A Fast and Stable Feature-Aware Motion Blur Filter",
  HPG 2014. https://research.nvidia.com/publication/2013-11_fast-and-stable-feature-aware-motion-blur-filter
- B. Wronski, "Volumetric Fog: Unified Compute Shader Based Solution to
  Atmospheric Scattering", SIGGRAPH 2014 Advances.
  https://bartwronski.com/wp-content/uploads/2014/03/ac4_gdc.pdf
- S. Hillaire, "Physically Based and Unified Volumetric Rendering in
  Frostbite", SIGGRAPH 2015; "A Scalable and Production Ready Sky and
  Atmosphere Rendering Technique", EGSR 2020, CGF 39(4).
  https://diglib.eg.org/handle/10.1111/cgf14050
- S. Lagarde, "Physically based camera" (Frostbite 2014 notes, section 5):
  EV100 exposure, f-stop and shutter as camera parameters.
- J. Hable, "Filmic Tonemapping" / Uncharted 2 (GDC 2010); N. Dutré et al.
  on colour grading with 3D LUTs (GPU Gems 2, ch. 24, "Using Lookup Tables
  to Accelerate Color Transformations", J. Selan).
- Unity Post-processing Grain (luminance contribution) and film-emulation
  literature: grain is strongest in the mid-tones and must be applied after
  the LUT, because a LUT cannot create texture.
  https://docs.unity3d.com/2017.3/Documentation/Manual/PostProcessing-Grain.html

## Implemented in this pass (where to look)

| Item | Code | Settings |
|---|---|---|
| GGX prefiltered env mips | `env.rs` `prefilter_ggx`, `mip_roughness`; `common.wgsl` `env_reflection` | automatic with any HDRI |
| DFG LUT, multi-scattering | `brdf.rs`; `common.wgsl` `ibl_weights`, `spec_energy`; `pbr.wgsl` | automatic |
| Specular AA | `common.wgsl` `specular_aa`; `pbr.wgsl` | automatic, `FORJA_FX=-specaa` |
| Porous specular, micro-shadowing | `forja_city::kit::porous_specular`, `MaterialData::specular`; `pbr.wgsl` | automatic, `FORJA_FX=-porous`, `-micro` |
| 3D LUT grade, grain, vignette, CA | `look.rs` (`Look`, `Grade`, `CameraFx`), `shaders/cinematic.wgsl` | `FrameDesc::look` (default `Look::cinematic()`), `FORJA_FX=-look` |
| Thin-lens DOF | `look.rs` `DepthOfField`, `shaders/dof.wgsl` | `Look::photo(focus, f_stop)`, `FORJA_DOF=<m>,<f>`, `FORJA_FX=-dof` |
