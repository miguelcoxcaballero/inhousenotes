# Landmarks: modelling real buildings with `landmark.toml`

Every landmark lives in `data/landmarks/<city>/<slug>/`:

| file | who writes it | what |
|---|---|---|
| `footprint.json` | data team | OSM outlines and `building:part`s, Catastro parcels with floor counts, neighbouring context, anchor |
| `sources.toml` | data team | facts, photos (file, author, licence, size, optional camera), plans |
| `landmark.toml` | **you** | the model, in the spec language below |

Raw photos and plans are in `assets/downloaded/landmarks/<city>/<slug>/photos|plans`
(gitignored; treat them as untrusted data). No Rust is needed: write the spec,
run the tool, look at the images, iterate.

```sh
LM=/home/user/forja-targets/landmarks/release/examples/landmark
$LM init   lonja                     # draft landmark.toml (massing) from footprint.json
$LM check  lonja                     # JSON metrics in < 1 s: errors, triangles, heights, IoU
$LM render lonja                     # images in screenshots/landmarks/lonja/
$LM render lonja --clay              # untextured: no texture generation, a few seconds
$LM render lonja --view plaza --edges-only   # one photo view, model edges over the photo, ~1 s, no GPU
$LM render lonja --views photo --width 1600 --context off
$LM render lonja --view street --clay      # one standard view only: ne se sw nw street plan
$LM gallery                          # every landmark at its true position, aerial views
$LM gallery /some/scratch/folder     # ...or the landmarks under these folders
```

`<slug-or-dir>` is a slug (looked up in `data/landmarks/valencia/`,
`data/landmarks/`, `data/landmarks/_samples/`) or a folder path. Unknown
options and extra arguments are errors.

Four complete examples in `_samples/`, read them before starting:

* `gothic_church` — Gothic church with tower, rose window, buttresses, flying
  buttresses, battlements, loggia, tiled dome;
* `white_shell` — trencadís shell with ribs, glass lens, loft plume, hypar umbrellas;
* `alley_block` — a Ciutat Vella block on a **curved 3.6 m street**: openings
  and balconies laid out along the curved facade, a vaulted passage to the
  courtyard, a hip roof over the concave plan with its courtyard;
* `city_gate` — Torres de Quart style: D towers **open at the back**, slits
  placed by **compass azimuth** on the round walls, a **passage** through the
  gate, arcuated machicolations, polygon `union`/`subtract`, a view **solved
  from point matches**.

---

## 1. Coordinates and conventions

* **Landmark-local metres**: `x` = east, `y` = up, `z` = **south**. The origin is
  the landmark's `anchor` (city frame = UTM 30N minus the Plaça de l'Ajuntament
  origin). Footprint rings are already in this frame, so `[[x, z], ...]` from
  `footprint.json` can be pasted into polygons directly.
* Ground is `y = 0`. Valencia is flat; model the street level at 0.
* **Plan points are `[x, z]`**, 3D points `[x, y, z]`.
* **Compass**: north = `-z`, east = `+x`. Azimuths are degrees clockwise from
  north (`facing = "s"` = 180°). `rotate_y` / `rotation` are degrees
  **counter-clockwise seen from above** (rotate_y = 90 turns east into north).
* **Polygon edges** are numbered after normalising the ring (outer ring
  counter-clockwise in x/z maths, i.e. clockwise on a north-up map; duplicate
  points, collinear points and zero-width spikes removed). Don't guess indices:
  `check` prints `outline_edges` (index, length, facing, endpoints) and
  `plan.png` labels the edges of outline 0. Prefer `facing:` selectors.
* **Facades follow the plan as runs.** Consecutive edges that turn by less
  than 25° (`run_angle`) form one *facade run*: a curved street front that OSM
  digitised as many short facets, a round tower, a drum. A run is laid out as
  one developed (unrolled) wall: `u` is the **arc length** along the run from
  its **left end as seen from outside**, `v` is the absolute height `y`.
  Openings, balconies and bays are placed along the whole run and bent onto
  the facets, so they never depend on where the OSM nodes fall. Sharper
  corners start a new facade.
* **Attached elements** (buttresses, pilasters, statues, finials, edge arrays)
  use a local frame: `x` along the wall (right when facing it), `y` up, `z`
  out of the wall; `facing` turns local `+z` to a compass direction.
* **Transforms** apply **scale, then rotation (`rotate`, then `rotate_y`),
  then translation**, all about `pivot` (default the element's origin):
  `p' = translate + pivot + R·S·(p − pivot)`. Children, steps and array
  layouts are expressed in the element's own (already rotated) frame. Example:
  a row of struts along local `x` with `transform = { translate = [-102, 0, 0],
  rotate_y = -20 }` is first turned 20° clockwise about the origin and then
  moved 102 m west — put `pivot` at the row's start to turn it in place.
* Materials use **world-scale UVs**: textures keep their real size whatever the
  element size. `uv_scale` = metres per texture repeat.
* Every table rejects unknown keys and every fixed vocabulary (kinds, shapes,
  pediments, railings, finials, caps, ...) rejects typos. Errors give
  `file:line:col`, the line with a caret, and a "did you mean". `nan`/`inf`
  are rejected with their position. Put `type` first in element tables
  (errors then point at the exact key).

## 2. File layout

```toml
[landmark]
name = "Llotja de la Seda"
slug = "lonja"                 # default: folder name
main_facade = "facing:s"       # edge selector on outline 0 (street view)
footprint = "footprint.json"   # default
notes = "free text"
[landmark.info]                # free-form metadata
architect = "Pere Compte"

[defaults]
material = "stone"             # element default (built-in or [material.x])
roof_material = "roof"         # also glass_material, frame_material,
                               # door_material, iron_material
floor_height = 3.2             # for `floors = n`
detail = 1.0                   # curve resolution multiplier (0.25..4)

[render]
sun = [215, 45]                # [azimuth, elevation] for standard views
sky = "clear"                  # clear (sky-only HDRI, default) | street (street HDRI)
                               # | atmosphere (analytic sky following the sun)
time = "day"                   # day | morning | evening | dusk | night
street_distance = 25           # force the street camera distance
street_eye = [3, 1.6, 9]       # or set the street camera explicitly
street_target = [0, 8, 0]
street_fov = 70
exposure = 0.55
paving = 80                    # paved radius around the building
hide_context = [3, "context:12"]   # drop context parcels (index or context:<i|id|refcat>)
                               # from renders and the street camera's obstacles

[reference_heights]            # measured heights for `check` (section 8)
"outline:0" = 32.5
"part:2" = { height = 21.0, tolerance = 1.0, source = "elevation, plan 1882" }
micalet = 51                   # an element name: its top

[ground]
cut = ["part:4"]               # holes in the street level (see `sunken`, 5.10)

[material.stone]  ...          # palette (section 4)
[component.pinnacle] ...       # reusable groups (section 5.10)
[[element]] ...                # the model (section 5)
[[view]] ...                   # cameras matching photos (section 6)
[[light]] ...                  # point lights for dusk / night (section 6)
```

## 3. Polygons and edge selectors

A **polygon** value is one of:

```toml
polygon = [[0, 0], [12, 0], [12, 8], [0, 8]]   # literal ring (any winding, not closed)
polygon = "outline:0"        # footprint OSM outline 0 (inner rings = courtyards)
polygon = "outline:all"      # every outline (one element per polygon)
polygon = "part:3"           # OSM building:part 3        ("part:all")
polygon = "osm:way/123456"   # outline or part by OSM id
polygon = "catastro:2"       # Catastro parcel by index, id or refcat ("catastro:all")
polygon = "context:1"        # a neighbour
polygon = "parent"           # inside tower tops / lanterns / masses: the plan below
polygon = "octagon:4.5"      # shorthands: circle:r, square:side, rect:WxD,
                             # triangle|pentagon|hexagon|octagon|dodecagon:circumradius
polygon = { shape = "octagon", center = [3, -2], width = 9, rotation = 22.5 }
#           shape: square rect circle ellipse triangle pentagon hexagon heptagon
#                  octagon decagon dodecagon polygon(sides = n)
#           radius = circumradius | width = across flats | size = [w, d]
#           edge 0 of a regular shape faces south (+z), before `rotation`.
polygon = { ref = "part:2", inset = 0.4, simplify = 0.15, clip = "outline:0" }
polygon = { points = [[0, 0], [9, 0], [9, 6], [0, 6]], holes = [[[3, 2], [5, 2], [5, 4], [3, 4]]] }
polygon = { ref = "part:0", union = ["part:1", "part:2"], subtract = ["circle:2"], offset = 0.4 }
```

A table takes exactly one source: `ref` (a reference string), `points` (a
literal ring — use the table form when you also need `holes` or operations)
or `shape`. Operations, in this order: `index` (one ring of a multi-polygon
ref), `keep_holes = false`, `hull = true`, `rect = true` (minimum-area
rectangle — handy for noisy OSM shapes), `simplify` (Douglas-Peucker, m),
`inset` / `offset` (m), `chamfer` (m), `holes = [polygon, ...]`,
**`union = [polygon, ...]`**, **`subtract = [polygon, ...]`** (exact
booleans; the result may be several pieces, each built), `clip`
(intersection), then `scale`, `rotate`, `translate` about `about` (default the
centroid). Shapes without `center` inside towers are centred on the tower.
A hollow, open-backed tower: `{ ref = "part:0", subtract = [{ ref = "part:0",
inset = 2.4 }, [[5, -20], [9, -20], [9, 0], [5, 0]]] }`, or simply a mass with
`open_edges` (5.1).

**Edge selectors** (`edges = ...`), alone or in a list (union; `not:` subtracts):
`"all"` (default), `"outer"`, `"inner"` (courtyards), `3`, `-1` (last), `"2..5"`,
`"longest"`, `"longest:2"`, `"shortest"`, `"facing:s"` (the 90° sector around
south, half-open: every edge belongs to exactly one of `n`/`e`/`s`/`w`, even on
plots rotated 45°), `"facing:135:20"` (azimuth with an inclusive tolerance),
`"min:4"` (edges ≥ 4 m), `"not:facing:n"`.

**Organic plans.** Concave plots, courtyards, chamfered or curved corners and
streets of any width are modelled from the footprint as they are:

* curved or faceted facades are runs (section 1): openings follow the arc
  length, cross facet nodes with folded reveals and are never dropped for
  landing on a node;
* consecutive selected edges of a run form one *interval*; `bays` are laid out
  along each interval (`per_edge = true` on an openings block restores
  per-edge layout);
* hip roofs follow any plan, courtyards included (straight skeleton, 5.5);
* cornices, battlements, balustrades, machicolations and gargoyles are spaced
  by arc length along each run, with corners only at real corners;
* the street view finds the free space in front of the facade and switches
  to a wide oblique view in narrow streets (section 8).

## 4. Materials

```toml
[material.stone]
kind = "ashlar"          # required
tint = "#e3d6bb"         # multiplies the texture ([r,g,b] 0..255 or 0..1 also ok)
color = "#..."           # base colour of flat / curtain_wall kinds
colors = ["#1f4f9c", "#2a5fb4", "#f3efe4"]   # palettes (glazed_tiles, trencadis, polychrome, stained_glass, marble veins)
weights = [0.5, 0.4, 0.1]  # glazed_tiles: share of each colour, tiles picked at random
pattern = "scales"       # glazed_tiles: scales chevron zigzag stripes (vertical) stripes_h
                         # (horizontal bands, alias hstripes / bands: no uv_rotate needed) checker diamond plain
                         # azulejo: checker_blue checker_green diamond_blue frieze white
block = [0.95, 0.45]     # ashlar block length and course height (m)
roughness = 0.6          # overrides the average roughness
metallic = 0.0
clearcoat = 0.8          # glossy coat (default 0.8 on glazed_tiles, trencadis, polychrome, azulejo)
uv_scale = 3.0           # metres per texture repeat
uv_rotate = 90           # degrees
normal_strength = 1.0
weathering = 0.3         # dirt and vertical streaks (0..1), tileable
emissive = 0.0           # self-lit (lit windows at dusk/night)
seed = 7                 # variation of procedural textures
```

| kind | look | notes |
|---|---|---|
| `ashlar` (`stone`) | coursed ashlar: blocks of varied length, course heights that differ a little, each block with its own grain and tone, a few replaced or stained blocks, sunk joints | `block` sets block size; default warm limestone |
| `limestone` | larger blocks, finer joints | smooth Gothic / Renaissance stonework |
| `sandstone` | small yellow stone bricks (photo set) | |
| `brick`, `stucco`/`render`, `plaster`, `moulding`, `granite` | kit photo sets | tint them |
| `marble`, `travertine`, `concrete` (white, formwork joints), `terracotta` | procedural | |
| `roof_tiles` (`clay_tiles`), `slate`, `zinc`, `lead`, `copper` (verdigris) | roofs | tile rows follow the eaves |
| `glazed_tiles` | Valencian glazed roof tiles: imbricated blue/white/gold/green scales etc., glossy | domes of Santa Catalina, Escuelas Pías, Mercat Central; give `colors` + `weights` |
| `azulejo`, `polychrome`, `trencadis` | wall tiles, colourful ceramic, white broken-tile mosaic | trencadís shards each glint differently |
| `bronze`, `gold` (leaf), `iron`, `white_steel` (`steel`) | metals | |
| `glass` | window glass (reflective, shows the room behind) | |
| `curtain_wall` | reflective glazing in a mullion grid (2×2 panels per repeat) | `color` tints the glass |
| `stained_glass` | dark jewel tones and lead cames as seen from outside | |
| `wood`, `painted_wood` | joinery | |
| `water`, `paving`, `cobble`, `asphalt`, `grass`, `interior`, `dark`, `flat` | misc | `flat` = plain `color` with fine noise |

**Built-in names** usable without a definition: `stone ashlar limestone
sandstone brick plaster stucco render moulding granite marble travertine
concrete white_concrete terracotta roof roof_tiles tiles slate zinc lead copper
bronze gold iron steel white_steel glass curtain_wall stained_glass wood door
frame azulejo glazed_tiles blue_tiles trencadis polychrome water interior dark
paving cobble asphalt grass context`. Defining `[material.stone]` overrides the
built-in. Match colours to the photos with `tint` (sample a sunlit and a shaded
patch and take the average; photos are usually warmer than the render).

## 5. Elements

Every element is a `[[element]]` table with `type` plus these common keys:

```toml
name = "nave"            # used by `on = "nave"`, metrics, error messages
material = "stone"
transform = { translate = [x, y, z], rotate_y = 0, rotate = [rx, ry, rz], scale = 1.0, pivot = [x, y, z] }
                         # order: scale, rotate, rotate_y, translate (about pivot), section 1
mirror = "z"             # extra mirrored copy across x = 0 / z = 0 / both ("xz"),
                         # or { axis = "x", at = 12.5 }
enabled = false          # switch off while iterating
collide = true           # in the collision mesh (structural types default on)
solid = false            # an open / thin surface (canopy, loft sheet, open shell): still
                         # collides but stays out of check's plan slices (footprint IoU)
note = "source: plan 1882, sheet 3"
```

Nested lists use TOML arrays of tables: `[[element.openings]]`,
`[[element.stages]]`, `[[element.stages.openings]]`, `[[element.top]]`,
`[[element.lantern]]`, `[[element.children]]`.

### 5.1 `mass` (alias `prism`) — the workhorse

```toml
[[element]]
type = "mass"
name = "body"
polygon = "outline:0"
y0 = 0
height = 18              # or y1 = 18, or floors = 5 (x floor_height)
batter = 0.0             # top inset (sloping walls; no openings then)
top = true               # flat cap; top_material = "paving" for terraces
bottom = false           # underside (default: on when y0 > 0)
smooth = false           # smooth shading around the walls (round towers);
                         # vertices of a run turning < 20° are smooth anyway
run_angle = 25           # edges turning less than this form one facade run;
                         # 0 = every edge its own facade
open_edges = "facing:e"  # no wall on these edges: the mass becomes a shell
wall_thickness = 2.4     #   this thick, showing its inside (open-backed towers)
cornice = "cornice"      # inline details: profile name, true, size or table
plinth = { profile = "plinth", size = 0.8 }
string_courses = [4.5, 9.0]
parapet = { height = 1.0, thickness = 0.3, edges = "facing:s" }
crenellations = { shape = "swallowtail", width = 0.8, gap = 0.6, height = 1.0, wall = 0.9 }
balustrade = { height = 0.95, finials = "urn" }
roof = { shape = "hip", pitch = 28, overhang = 0.4 }
  [[element.openings]]   # see 5.2; any number of blocks
  ...
edge_materials = [{ edges = "facing:n", material = "brick" }]   # wall material per edge
```

`edge_materials`: walls of the selected edges use another material (later
entries win); their reveals and trim follow it. A material change splits the
facade run there, so bays are laid out per stretch. Not on battered walls.

Windows show a room behind the glass; its depth is measured from the real
plan (a ray from the window into the solid), so thin wings, L plans and
windows near corners never poke out; under 0.15 m there is a dark plane
instead.

### 5.2 `slab` and `wall`

```toml
[[element]]
type = "slab"            # horizontal plate: floors, terraces, platforms, paving
polygon = { ref = "part:0", inset = 2.4 }
y = 9.5                  # top surface
thickness = 0.3

[[element]]
type = "wall"            # free-standing wall with through openings
path = [[0, 0], [12, 0], [18, 4]]   # or polygon = ... + edges = ... (chains of selected edges)
closed = false           # path loops back (orientation does not matter)
y0 = 0
height = 4               # or y1
thickness = 0.6
align = "center"         # center | left | right of the path (seen along it)
coping = true
  [[element.openings]]   # edges = path segment indices; the front is the
  kind = "arch"          # right side of the path, the back gets the same holes
```

Wall paths are runs too: a curved garden wall gets its openings by arc length.

**Stepped and raked tops** (`wall` only): `top_profile = [[s, y], ...]` gives
the top height `y` at distance `s` along the path from its first point,
linear in between; two points with the same `s` make a step. `y1`/`height`
are then optional. Openings, top faces, step risers and the ends follow the
profile; `coping` is not built on profiled walls.

```toml
type = "wall"
path = [[0, 0], [30, 0]]
top_profile = [[0, 7], [8, 7], [8, 5], [14, 5], [30, 2]]   # step down at 8 m, then a rake
```

### 5.3 Openings

Inside a mass/stage/wall (`[[element.openings]]`), on gables
(`[[element.gable_openings]]` of a roof), or standalone with
`type = "openings"` and `on = "<mass or stage name>"`.

```toml
edges = "facing:s"
per_edge = false         # true: lay bays out per plan edge instead of per run interval
kind = "window"          # window door portal arch oculus rose lancet twin (ajimez)
                         # niche blind (blind arcade) slit passage (gate, tunnel)
arch = "round"           # flat round segmental pointed lancet horseshoe
                         # pointed_horseshoe ogee basket tudor trefoil circle
                         # quatrefoil shouldered (default per kind)
width = 1.2
height = 2.6             # bottom to apex
rise = 0.6               # arch rise (default per shape)
levels = [1.0, 4.6, 8.2] # bottom heights (absolute) or { start = 1, step = 3.6, count = 3 }
bays = 5                 # count, or a table:
#   count = 5            evenly between the margins
#   spacing = 3.4        as many as fit, centred (with count: a centred group)
#   margin = 1.0         clear wall at both ends; margins = [left, right]
#   at = [2.0, 6.5]      centres in metres from the interval's left end (seen from outside)
#   offsets = [-3, 3]    centres from the interval's centre (symmetric facades)
#   azimuth = [270]      one opening where the facade faces each compass azimuth
#                        (round towers, curved walls); azimuth_tolerance = 30
#   skip = [0, 4]        bay indices left solid
skip = [[0, 2]]          # [level, bay] left solid
depth = 0.35             # reveal depth; splay = 0.2 widens it outward
infill = "glass"         # glass stained door shutters louvres grille void dark
                         # blind tracery curtain_wall
frame = 0.07             # joinery width; mullions = 1; transoms = 1; panes = [2, 3]
tracery = "circle"       # y circle trefoil quatrefoil geometric mullions
spokes = 12              # rose windows; rings = 1
orders = 4               # recessed archivolts; order_step = 0.3; colonnettes = true
tympanum = true          # stone tympanum over door leaves (portals default)
surround = { width = 0.2, projection = 0.06, ears = true, pediment = "triangle" }
                         # pediment: triangle segmental broken broken_segmental
hood = { size = 0.15, gap = 0.05, stops = true }   # or `hood = true`
sill = true              # or { depth, height, extend }
keystone = 0.35
voussoirs = { count = 15, width = 0.5, alternate = "brick" }  # two-colour arches
balcony = { depth = 0.7, railing = "iron", continuous = false }
                         # railing: iron stone balustrade glass none
bells = true             # bells behind belfry openings
material = "stone_fine"  # surrounds, hoods, sills; stone_material = mullions/tracery/reveals
glass_material = "...", frame_material, door_material, iron_material
```

**Where openings go.** The selected edges of each facade run form intervals
(consecutive selected edges merge, laid out by arc length; a whole round
tower is one closed interval with bays evenly around it). Bays are centred
in each interval. The default `margin` is 0.8 × the width (at least 0.6 m)
but never more than 15 % of the interval, and 0 for `bays = 1` (centred);
when the margins leave no room but the opening still fits with 15 cm of wall
each side, one is centred. An opening may cross facet nodes of its run, never
a real corner: one running past the end of its facade is skipped **with a
warning naming the element, its line, the facade length and the width**, and
a block that places nothing says why (lengths of the selected facades and
the width each one needs).

Defaults per kind: door/portal/arch/passage sit on the facade base; a portal
gets 3 orders and a tympanum; rose/oculus are circles placed at 62 % of the
wall height; windows sit 1 m up. Openings that reach the bottom of a facade
(doors, arches) or its side become notches.

**Passages** (`kind = "passage"`): a gate or vaulted passage through the whole
mass. The arch outline is cut on the facade, carried straight through the
solid along the facade's inward normal to the opposite facade (or a
courtyard), cut there too, and lined with a vault and jambs; the collider
follows, so the passage is walkable and the plan metrics see it. If the far
side is not a single facade (a corner, or outside the plan), it becomes a
deep recess with a warning. Surrounds, voussoirs and hoods apply on the entry
side.

### 5.4 Mouldings: `cornice`, `string_course`, `plinth`, `moulding`

```toml
type = "cornice"
on = "body"              # polygon + heights of a named mass (or polygon = ...)
edges = ["facing:s", "facing:e"]
y = 17.2                 # base height (cornice default: top - size; plinth: base)
profile = "classical"    # cornice classical gothic cyma ovolo cavetto torus fillet band
                         # chamfer plinth base string drip eave stepped art_deco modern
                         # slab corbel_table, or [[out, up], ...] in metres
size = 0.8               # height; projection = 0.6 sets the depth separately
offset = 0.0             # outward shift
modillions = { spacing = 0.9, width = 0.16, height = 0.3, style = "bracket" }   # bracket block corbel console
dentils = true
path = [[0, 0], [10, 0]] # instead of polygon: an open polyline (needs y)
```

Mouldings sweep continuously along chains of selected edges (curved runs
stay smooth); modillions and dentils are spaced by arc length.

### 5.5 Parapets and defences

* `parapet`: `on/polygon`, `edges`, `y`, `height`, `thickness`, `coping`, `overhang`.
* `balustrade`: `height`, `spacing` (balusters, by arc length), `pedestals`
  (m between; also at every real corner), `finials` (`urn ball pinecone none`), `inset`.
* `crenellations` (`battlements`): `shape` rect swallowtail stepped pyramidal
  rounded, `width`, `gap`, `height`, `thickness`, `wall` (solid parapet under
  the merlons), `loopholes`, `overhang`. Merlons are spaced along each run (a
  merlon at both ends of a straight wall, evenly round a tower).
* `machicolations` — defaults match the Valencian gates (Quart, Serranos):

  ```toml
  type = "machicolations"
  polygon = "part:0"       # or on = "tower"
  edges = "not:facing:e"
  y = 26.5                 # bottom of the corbels (default: top of `on` - corbel_height)
  projection = 0.9         # overhang of the parapet's face beyond the wall
  spacing = 1.6            # corbel centres, by arc length
  corbel_height = 1.5      # total corbel height
  corbel_courses = 4       # stepped courses, deeper towards the top
  corbel_width = 0.45
  arches = true            # arches spanning between the corbel heads (false = plain brackets)
  arch = "pointed"         # round pointed segmental ...
  parapet = 1.1            # projecting parapet height on top; thickness = 0.5
  merlons = { shape = "rect", width = 1.1, gap = 0.7, height = 1.3 }
  ```
* `gargoyles`: `y`, `spacing`, `length`, `size`, `tilt`, `corners` (real corners only).

### 5.6 `roof`

```toml
type = "roof"
on = "body"              # or polygon + y (eave height)
shape = "gable"          # flat gable hip pyramid mansard shed barrel cone spire tent dome none
pitch = 30               # degrees, or height = 4.5 (rise at the ridge)
ridge = "long"           # gable/barrel: long short ns ew or azimuth; shed: the
                         # direction the slope faces
overhang = 0.35          # thickness = 0.16
ridge_cap = true         # half-round tiles on ridges and hips (tile roofs)
ribs = { count = 8 }     # spires, cones, pyramids
mansard = { height = 3, inset = 1, upper = "hip", upper_pitch = 15 }   # upper: hip flat pyramid
vault = "pointed"        # barrel section: round pointed segmental ...
wall_material = "stone"  # gable / end walls
  [[element.gable_openings]]
  edges = "facing:w"
  kind = "oculus"
```

**Hip roofs are exact on any plan**: the faces are those of the plan's
straight skeleton (every eave rises at the same pitch), so concave Ciutat
Vella blocks, L/T/U plans, plots with small OSM dents and **courtyards** get
a correct hip roof with hips, valleys and ridges, plus ridge caps. Gable and
shed roofs work on any plan (the gable walls follow the plan) but cover
courtyards (a warning says so). Flat roofs keep courtyards open. Inline
`roof = {...}` on a mass uses the mass top.

### 5.7 `dome`

```toml
type = "dome"
center = [7, 0]          # plan centre
y = 18                   # base: bottom of the drum (or springing without drum)
radius = 4.5             # or rx / rz (elliptical domes)
height = 5.2             # rise of the shell
profile = "hemisphere"   # hemisphere pointed onion ellipsoid segmental bell parabolic ribbed
gores = 16               # melon segments; gore_depth = 0.06
ribs = { count = 8, width = 0.3, depth = 0.15, material = "stone" }
oculus = 0.8             # open eye with a ring
base_ring = true
rotation = 22.5
drum = { height = 4.5, sides = 8, windows = 8, window_width = 1, window_height = 2.2,
         window_arch = "round", window_infill = "glass", cornice = true, pilasters = true,
         openings = [{ kind = "oculus", width = 0.8, levels = [21], bays = { count = 8 } }] }
                         # drum `openings`: any openings blocks, as on a mass
drum_material = "stone"
finial = "cross"         # preset, or [[element.finial]] tables
  [[element.lantern]]    # elements on the apex (origin at the apex)
  type = "lantern"
  radius = 0.9           # height, sides, cap = dome|cone|onion|spire|flat, cap_height, cap_material, finial
```

### 5.8 `tower`

```toml
type = "tower"
name = "micalet"
polygon = { shape = "octagon", center = [-20, 31], width = 10.6 }
y = 0
smooth = false           # smooth shading on every stage (round towers)
run_angle = 25           # as on masses
  [[element.stages]]     # bottom to top
  name = "shaft"         # usable as on = "shaft"
  height = 30
  inset = 0.0            # from the previous stage; or polygon = { shape = ..., width = ... }
  batter = 0.0
  material = "stone"
  smooth = false
  cornice = "gothic"     # plinth, string_courses, parapet, crenellations, balustrade
  belfry = true          # arched dark openings on every face, with bells
  pinnacles = { width = 0.6, height = 3 }   # at the stage's top corners
    [[element.stages.openings]]
    kind = "lancet"
  [[element.top]]        # origin at the top centre; polygon "parent" = last stage
  type = "roof"
  shape = "spire"
  polygon = { ref = "parent", inset = 0.6 }
```

An error in a stage's polygon names the stage; an inset that collapses a
stage keeps the previous plan with a warning.

### 5.9 Supports and sculpture

* `column`: `at = [x, y, z]`, `height`, `radius`, `order` (plain tuscan doric
  ionic corinthian composite gothic (clustered pier) salomonic pier octagonal
  cylinder), `entasis`, `flutes`, `base`, `capital`, `pedestal` (height).
* `pilaster`: `at`, `facing`, `height`, `width`, `depth`, `order`, `flutes`.
* `buttress`: `at`, `facing`, `height`, `width`, `depth`, `setoffs` (or
  `steps = [{ height, depth }]`), `cap` (`slope gable flat pinnacle`), `pinnacle`, `plinth`.
* `flying_buttress`: `from` (top of the outer pier), `to` (wall point),
  `width`, `thickness`, `sag`, `pier = true`, `pinnacle`.
* `pinnacle`: `at`, `width`, `height`, `sides`, `crockets`, `shaft`.
* `finial`: `at`, `style` (cross ball spike weathervane urn pinecone fleur crown star statue), `size`.
* `statue`: `at`, `facing`, `height`, `style` (figure bust equestrian angel seated group), `pedestal`.
* `relief` (`plaque`): a panel on a wall in the wall frame of `at` (bottom
  centre) / `facing`: `size = [w, h]`, `shape` (rect arched circle oval shield
  cartouche), `depth` (projection, > 0), `frame` (width, 0 = none) and
  `frame_depth`, `relief` (none boss panel figure: a raised field or motif),
  `field_material` (inscription stone, dark marble; frame = `material`).
* `stairs`: straight `from = [x, z]` → `to` (ascending), `width`, `steps`,
  `rise`, `riser`, `tread`, `cheeks`; or `polygon` = podium top with `steps`
  around it.
* `arcade` (`colonnade`, `loggia`): `from`/`to` or `polygon` + `edges`, `y`,
  `bays`, `arch`, `rise`, `impost` (springing height), `height` (top),
  `depth`, `column = { order, radius, double }` or `piers = { width }`,
  `gallery` (depth of a loggia behind), `hood`, `cornice`, `face = "left"|"right"`.
* `lantern`: see domes; also usable alone.

### 5.10 Generic geometry, groups, arrays, components

* `box`: `size` (+ `center`, default resting on y = 0) or `min`/`max`.
* `cylinder` / `cone`: `at` + `height` or `from`/`to`; `radius`, `radius_top`,
  `segments` (3..512), `caps`, `faceted`. A vertical cylinder takes
  `[[element.openings]]` like a mass (laid out round it by arc length;
  `bays = { azimuth = [...] }` works); not cones or tilted cylinders.
* `sphere` (`ellipsoid`): `center`, `radius` or `radii`, `lat = [-90, 90]`,
  `lon = [0, 360]`, `segments` (4..512).
* `torus`: `center`, `radius`, `tube`, `axis`, `segments`.
* `lathe`: `at`, `profile = [[r, y], ...]` bottom to top, `segments` (3..512), `angle = [0, 360]`.
* `extrude`: `profile` (any polygon value, holes allowed), `plane` (`xy`:
  extrude +z, `xz`: +y, `zy`/`yz`: +x), `origin`, `length`, `center`,
  `smooth = true` (or a crease angle) for curved profiles such as roof arcs.
* `sweep`: `profile` (section), `path` (`points` polyline, `curve` smooth
  through points, `bezier` 3k+1 controls, or arc `center` + `radius` + `angles`
  + `normal` (+ `axis`)), `twist`, `scale_end`, `caps`, `up`, `level`.
  Vertical arcs rise from their first axis.
* `loft`: smooth surface through `stations = [{ at, section, scale = [s, u], twist, normal }]`,
  `section` default, `samples` per span (≤ 256), `resolution`, `smooth`, `caps`, `closed`,
  `double_sided = true` (open surfaces visible from both sides), `flip = true` (inside out).
  For plumes, organic beams, ribs, wings.
* `shell`: thin parametric surfaces with `thickness` and a rim; `center`,
  `clip = [[nx, ny, nz, d], ...]` keeps n·p ≤ d (cut openings and edges),
  `segments = [nu, nv]` (each 2..1024, nu·nv ≤ 250 000),
  `ribs = { count, along = "u"|"v", width, depth, inside }`.

  | surface | parameters (u, v in degrees) |
  |---|---|
  | `ellipsoid` (`sphere`) | `radii = [rx, ry, rz]` (or `radius`); u = longitude from +x towards +z (`u_range`, default 0..360), v = latitude (`v_range`, default 0..90 = upper half) |
  | `superellipsoid` | as ellipsoid plus `exponents = [ns, ew]` (2 = ellipse, < 2 pointed, > 2 boxy) |
  | `paraboloid` | `radii` (rx, ry, rz), `height` (default ry): apex at `center + height`, rim at `center`; `v_range` = radial fraction 0..1 |
  | `cylinder` | a **horizontal** barrel along local **x**: length `height` (default 2·rx), section in the y–z plane, u from +z (0°) over the top (+y, 90°) to −z (180°; `u_range` default 0..180), `radii[1]` = vertical radius, `radii[2]` = horizontal; `v_range` has no effect; turn it with `transform.rotate_y` |
  | `torus` | `radii[0]` = ring radius, `radii[1]` = tube radius; u around the vertical axis, v around the tube |
  | `cone` | `radii` (rx, rz at the base), `height`; apex up; u around the axis |
  | `hypar` | `corners` = 4 points [x, y, z] in order around the patch (radii unused) |

  L'Hemisfèric, Museu de les Ciències, Palau de les Arts, Candela's hypars.
* `mesh`: `vertices`, `triangles` / `quads`, `smooth` (angle). Last resort.
* `sunken` (`moat`, `pit`): an area below the street (moats, sunken plazas,
  light wells): `polygon`, `depth` (m below 0, default 3), `floor = true`,
  `floor_material` (default paving), `walls = true` (retaining walls facing
  in, `material`), `kerb` (rim height above the street, 0 = none). Its plan
  becomes a hole in the ground: the render ground and the world terrain stay
  open there (`Landmark::ground_cuts`), and `check` does not warn about
  geometry below the ground inside it. For custom pits (stairs down, ramps),
  model them with any elements and list their plan in `[ground] cut`.
* **Sections** (sweep/loft): `[[a, b], ...]` or `{ shape = ellipse circle rect square
  superellipse polygon lens teardrop i_beam, size = [w, h], radius, exponent, sides, rotate }`.
* `group`: `children` (+ `component`), shares transform and material.
* `array`: items are `children` and/or `component`, built once and copied.
  * `mode = "linear"`: `count`, `step = [dx, dy, dz]`;
  * `"grid"`: `counts = [nx, nz]` or `[nx, ny, nz]`, matching `steps`;
  * `"radial"`: `count`, `center`, `axis`, `start`, `sweep` (degrees), `radius`
    (items at local +z, then turned); `rotate = false` keeps every copy's
    orientation and only moves it (its centre, or its origin with `radius`);
  * `"edges"`: `polygon`/`on`, `edges`, `spacing` or `count`, `margin`,
    `corners` = skip include only, `offset`, `y` (children in the facade frame);
  * `"path"`: `path`, `spacing`/`count`; `"points"`: `points`, `facings`;
  * `skip = [i, ...]` drops instances;
  * **ramps** for modules that change along the run (Calatrava ribs, arches
    of the Oceanogràfic): `scale_end = 0.6` or `[sx, sy, sz]` (the last copy's
    scale, linear from 1), `rotate_step = 4` (degrees about each copy's own
    vertical axis, per copy), `offset_step = [0, 0.2, 0]` (per copy, in its frame).
    Each copy's own transform is `offset · rotate · scale` about its origin.
* `instance`: `component`, `at`, `facing`.
* `[component.name]` with `material` and `[[component.name.children]]`.

## 6. Matching photos: `[[view]]`

```toml
[[view]]
name = "plaza"
photo = "photos/lonja_main_facade.jpg"   # relative to the raw folder
eye = [-12.0, 1.6, 38.0]                  # camera; default: sources.toml camera at 1.6 m
target = [0.0, 9.0, 0.0]                  # default: camera heading, else the model centre
focal_35mm = 28                           # or fov_y = 54 (vertical degrees), or "exif"
roll = 0
fix = ["fov"]                             # match keeps these as written: fov eye height roll pitch
level = true                              # match keeps roll = 0
upright = false                           # perspective-corrected photo (see below)
shift = [0, 0.12]                         # lens shift (fractions of the image), or "solve"
eye_height = 1.6                          # soft prior on the solved eye height (eye_height_sigma = 0.3)
sun = [200, 50]                           # azimuth, elevation of the sun in the photo
time = "dusk"                             # day morning evening dusk night (sky and light)
context = true
match = [                                 # optional: solve the camera from the photo
  { px = [412, 980], world = [-10.0, 0.0, 7.0] },     # pixel from the photo's top-left
  { px = [1388, 1002], world = [10.0, 0.0, 7.0] },    # and the landmark-local point it shows
  { px = [1401, 233], world = [10.0, 25.0, 7.0] },
  { px = [398, 240], world = [-10.0, 25.0, 7.0] },
  { px = [905, 140], world = [0.0, 32.0, 0.0] },
]

[[light]]                                 # point lights (dusk / night)
at = [0, 4.2, 5.6]
color = "#ffc27a"
intensity = 1.0                           # 1 = a street lamp
radius = 12
```

How to set a view from a photo:

1. **Fastest: point matches.** Pick 5–8 points you can locate both in the
   photo (pixel coordinates from its top-left corner, as any image viewer
   shows them) and in the model: footprint corners at `y = 0`, cornice
   corners at their heights, tower tops, the apex of a gable. Spread them
   over the image and in depth. `check` (and `render`) solve the camera
   (eye, target, fov, roll) by least squares and print it with the RMS error
   in pixels: < 5 px is good. Paste the printed eye/target/fov_y/roll into the
   view (or keep `match` there; it overrides them). Four points is the minimum.
   **Constraints** make `match` well posed with few or coplanar points
   (one facade only): `focal_35mm = "exif"` reads the 35 mm focal length
   from the photo and holds the field of view; `fix = ["fov"]` holds a
   written `fov_y`/`focal_35mm`; `eye_height = 1.6` pulls the eye to street
   level; `fix = ["height"]` holds it exactly; `fix = ["eye"]` solves only
   the direction, roll and fov (2 points suffice); `level = true` holds roll
   at 0. **Perspective-corrected photos** (verticals parallel, typical of
   architecture photos and Wikimedia "corrected" images): `upright = true`
   holds pitch and roll at 0 and solves the vertical lens shift; the render
   is shifted the same way (`shift` is printed; `shift = [x, y]` sets it by
   hand, `"solve"` solves both). `check` lists per view `points` (each
   match's `error_px` and `reprojected` pixel), `planarity`, `fixed`, and
   warnings: nearly coplanar points without constraints, a camera solved
   underground, a match far off (likely a wrong pixel or world point), too
   few points; and for every view (and the street view) a camera standing
   inside a context building (hide it with `[render] hide_context`).
2. **By hand**: find the camera on the plan (`plan.png`, street names, the
   facades visible and how foreshortened they are). Phone photos are taken at
   1.5–1.7 m; a camera on a balcony is at about 3.2 m per floor. An object of
   real height H filling a fraction f of the image height at vertical fov φ
   is at about `H / (f · 2 tan(φ/2))`. 35 mm equivalent focal length f gives
   `fov_y = 2·atan(12 / f)` for a 3:2 landscape photo (the tool uses the
   diagonal definition, valid for any aspect). Phones: 24–28 mm (fov_y ≈
   53–46°); telephoto 50–80 mm. A wide image with converging verticals is a
   short focal length close to the building. Aim `target` at the point at
   the centre of the photo; `roll` levels a tilted horizon.
3. Iterate with `render --view <name> --edges-only` (about a second, no GPU):
   `compare_<n>.jpg` = **[photo | model edges | model edges over the photo]**
   (yellow silhouette and depth breaks, cyan creases). Align the camera until
   fixed, unambiguous features coincide (the base line of the facade, corners,
   the top of a tower); only then correct the model, then do a full render.
   Change one thing at a time.

Dusk and night photos: set `time` (or `sun` with a low elevation — below
12° the sky switches to the analytic atmosphere, which follows the sun),
add `[[light]]`s and `emissive` materials for lit windows.

## 7. Recommended method

1. `init` → a massing draft: one mass per OSM `building:part` (parts replace
   the outline they cover, as in OSM Simple 3D Buildings; the outline mass is
   written with `enabled = false`), outlines without parts, heights from tags
   (`height`, else `building:levels × 3.2 + roof:height`; levels count from the
   ground) or Catastro floors (floors × 3.2 m + 1 m), and `main_facade` from the
   photo cameras (else the longest edge facing a street, not a neighbour).
   `check` and `render`.
2. **Massing**: fix footprints and heights first. Use `part:`/`catastro:`
   references, `union`/`subtract` to combine them; split by real volumes
   (nave, aisles, chapels, tower, wings); `check` must show footprint IoU ≥ 0.9
   against OSM and heights agreeing with tags (`height_check: "ok"`), unless
   you know the data is wrong (say why in `note`).
3. **Roofs, towers, domes**: shapes and pitches from photos (aerials help).
   Irregular blocks: one hip roof on the whole plan.
4. **Openings**: rows and bays from the main facade photo; count bays, measure
   levels. Dimensions from known heights: doors 2.2–3 m (monumental portals
   5–8 m), storeys 3–4.5 m (ground floors 4–6 m), stone courses 0.3–0.6 m,
   balustrades 0.9–1.0 m, a person 1.7 m. Count courses/bays to measure
   anything else. On curved fronts, measure bay offsets along the street.
5. **Details**: cornices, string courses, buttresses, pinnacles, tracery,
   sculpture — enough to read at street distance, no more.
6. **Materials**: kind and tint from the photos; `weathering` 0.2–0.5 for old
   stone.
7. Iterate with `[[view]]`s for each good photo (aim for 3+: main facade,
   oblique, a distant/aerial one), aligned with `match` points.

## 8. Reading the outputs

`render` writes to `screenshots/landmarks/<slug>/` (or `--out`):

* `view_ne|se|sw|nw.png` — obliques framing the landmark alone;
* `street.png` — eye-level view of `main_facade` with the neighbours. The
  camera backs off until the facade fits (from its width and the model
  height), but stops 0.5 m before the first building across the street
  (context or the landmark itself). In a narrow street it stands near the far
  side, a little along the street, and looks back obliquely with a 96° lens.
  The log says which (`street: ...`); override with `street_eye`,
  `street_target`, `street_fov`, `street_distance`;
* `plan.png` — near-orthographic top view, **red** OSM outlines, **green**
  OSM parts, **blue** Catastro, edge numbers of outline 0, 10 m bar, north arrow;
* `sheet.jpg` — all of the above on one image; `metrics.json` — the check output;
* `photo_<n>.png`, `compare_<n>.jpg` — for each `[[view]]`.

The sky is a sky-only HDRI (`[render] sky = "clear"`) so nothing in the
backdrop looks like context; `sky = "street"` uses the street HDRI and
`sky = "atmosphere"` the analytic sky.

`check` JSON: `triangles`, `draw_calls` (materials), `by_material`, `bbox`,
`max_height`, `elements` (bottom/top/height/plan extent/triangles per named or
top-level element, with the line), `footprint.osm|catastro` (`iou`,
`precision` = share of the model inside the reference, `recall` = share of the
reference covered; plan slice at 1–3 m; through passages are holes in it),
`outlines|parts|catastro` (tagged height vs `model_median`, `model_p90`,
`model_max`, `height_check`), `outline_edges`, `solved_views` (cameras from
`match`, with per-point errors), `ground_cuts`, `reference_checks`,
`warnings`. **Measured heights**: `[reference_heights]` keys
`outline:<i>`, `part:<i>`, `catastro:<i>` (or a refcat), an OSM id
(`way/123`) replace that polygon's tagged / level-based height: the row gets
`reference_height`, `height_source = "reference"` and a `height_check`
("model too high" when even its 90th percentile is above, "too low" when its
highest point is below, beyond `tolerance`, default 1 m), and the OSM
warning is replaced. Any other key is an element name, compared with the
element's top (`reference_checks`, default tolerance 0.5 m). Low precision = something sticks out (porch, stairs,
buttresses: fine if real); low recall = missing volume.

## 9. Budgets and limits

Typical landmark 20k–150k triangles; the cathedral up to 400k (`check` warns
above). The cost is in curves: lower `segments`/`detail` on small round things,
avoid huge `count`s of detailed components, keep `[defaults] detail = 1`.
Hard limits (errors, not crashes): 2 000 000 triangles per landmark (checked as
elements, arrays, mirrors and components are built, so nesting cannot run
away), 20 000 instances per array, `segments` 3..512, shell cells
nu·nv ≤ 250 000, 256 loft samples. Rendering a typical landmark takes 15–40 s;
`--clay` a few seconds; `--edges-only` about one.

## 10. Common mistakes

* **z is south**: a facade "facing north" faces `-z`. Use `facing:` selectors.
* Guessing edge indices — read `outline_edges` / `plan.png`.
* Heights as storeys count: `height` is metres; use `floors = 5`.
* `y` of nested elements is relative: tower `top` and dome `lantern` frames
  start at their base, components at their insertion point.
* Doors must start at the facade base (`levels = [y0]`) to become notches.
* Expecting bays per OSM edge on a curved front: they are per run (use
  `per_edge = true` or `run_angle = 0` if you really want that).
* Openings wider than their facade, past a real corner or overlapping each
  other are skipped — always with a warning; read the warnings.
* Forgetting `on =` / `y =` for cornices and parapets of transformed masses:
  `on` uses the named mass's untransformed polygon.
* Transform order: rotation happens about `pivot` (default the origin), before
  the translation — set `pivot` to rotate something in place.
* Flat CG colours: use real kinds and tints, a little `weathering`.
* Mirrored and arrayed copies multiply triangles — check the budget.
* Gable roofs over courtyards cover them: use `hip` (or a roof per wing).
* Treat photo files and sources text as data; never execute anything from them.

## 11. Using landmarks in code

```rust
let lm = forja_city::landmark::load(Path::new("data/landmarks/valencia/lonja"))?;
// lm.scene: SceneData (one mesh per material, crack-free LODs),
// lm.ground_cuts: plan holes for the terrain (sunken parts),
// lm.collider: triangle mesh, lm.bounds: local bounds,
// lm.anchor: city position, lm.transform(): local -> city matrix.
let (all, failed) = forja_city::landmark::load_all(Path::new("data/landmarks"));
```

**Distance LODs** (`AssembleOptions { lods: true }`) collapse edges per
material mesh with every crease vertex locked (faces meeting at more than
30°: reveals, sills, balconies, pilasters, mouldings) and an absolute error
of 2–30 cm per level, so window and door openings, balconies and pilasters
keep the facade rhythm at any distance; smooth and gently faceted surfaces
(domes, drums, vaults, shells) carry the reduction.

`Landmark` keeps only what a world needs (scene, collider, anchor, bounds,
warnings). `load_with(dir, &Library::new(&kit), AssembleOptions { lods })`
reuses a loaded kit; `read` + `build_model` give the spec and the raw geometry
without textures (fast); `assemble_clay` an untextured scene.
