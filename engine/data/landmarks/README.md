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
$LM init   lonja           # draft landmark.toml (massing) from footprint.json
$LM check  lonja           # JSON metrics in < 1 s: errors, triangles, heights, IoU
$LM render lonja           # images in screenshots/landmarks/lonja/
$LM render lonja --views photo --width 1600 --context off
$LM gallery                # every landmark at its true position, aerial views
```

`<slug-or-dir>` is a slug (looked up in `data/landmarks/valencia/`,
`data/landmarks/`, `data/landmarks/_samples/`) or a folder path. Two complete
examples: `_samples/gothic_church` (Gothic church with tower, rose window,
buttresses, flying buttresses, battlements, loggia, tiled dome) and
`_samples/white_shell` (trencadís shell with ribs, glass lens, loft plume,
hypar umbrellas). Read them before starting.

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
  counter-clockwise in x/z maths, i.e. clockwise on a north-up map). Don't guess
  indices: `check` prints `outline_edges` (index, length, facing, endpoints) and
  `plan.png` labels the edges of outline 0. Prefer `facing:` selectors.
* **Facades**: `u` runs along a facade from its **left end as seen from
  outside**, `v` is the absolute height `y`. Openings are centred by default.
* **Attached elements** (buttresses, pilasters, statues, finials, edge arrays)
  use a local frame: `x` along the wall (right when facing it), `y` up, `z`
  out of the wall; `facing` turns local `+z` to a compass direction.
* Materials use **world-scale UVs**: textures keep their real size whatever the
  element size. `uv_scale` = metres per texture repeat.
* Every table rejects unknown keys. Errors give `file:line:col`, the line with
  a caret, and a "did you mean". Put `type` first in element tables (errors
  then point at the exact key).

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
street_distance = 25
exposure = 0.55
paving = 80                    # paved radius around the building

[material.stone]  ...          # palette (section 4)
[component.pinnacle] ...       # reusable groups (section 5.9)
[[element]] ...                # the model (section 5)
[[view]] ...                   # cameras matching photos (section 6)
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
polygon = "parent"           # inside tower tops / lanterns: the plan below
polygon = "octagon:4.5"      # shorthands: circle:r, square:side, rect:WxD,
                             # triangle|pentagon|hexagon|octagon|dodecagon:circumradius
polygon = { shape = "octagon", center = [3, -2], width = 9, rotation = 22.5 }
#           shape: square rect circle ellipse triangle pentagon hexagon heptagon
#                  octagon decagon dodecagon polygon(sides = n)
#           radius = circumradius | width = across flats | size = [w, d]
#           edge 0 of a regular shape faces south (+z), before `rotation`.
polygon = { ref = "part:2", inset = 0.4, simplify = 0.15, clip = "outline:0" }
```

Operations in a table (applied in this order): `index` (one ring of a
multi-polygon ref), `keep_holes = false`, `hull = true`, `rect = true` (minimum
area rectangle — handy for noisy OSM shapes), `simplify` (Douglas-Peucker, m),
`inset` / `offset` (m), `chamfer` (m), `holes = [polygon, ...]`, `clip`
(intersection), then `scale`, `rotate`, `translate` about `about` (default the
centroid). Shapes without `center` inside towers are centred on the tower.

**Edge selectors** (`edges = ...`), alone or in a list (union; `not:` subtracts):
`"all"` (default), `"outer"`, `"inner"` (courtyards), `3`, `-1` (last), `"2..5"`,
`"longest"`, `"longest:2"`, `"shortest"`, `"facing:s"` (±45°), `"facing:135:20"`
(azimuth with tolerance), `"min:4"` (edges ≥ 4 m), `"not:facing:n"`.

Plans of any shape work: concave plots, courtyards, chamfered or curved
corners, very short or very narrow facades on alleys. Openings skip facades
that are too short, and facades are built per edge, so a building squeezed
between irregular streets is modelled exactly as its footprint.

## 4. Materials

```toml
[material.stone]
kind = "ashlar"          # required
tint = "#e3d6bb"         # multiplies the texture ([r,g,b] 0..255 or 0..1 also ok)
color = "#..."           # base colour of flat / curtain_wall kinds
colors = ["#1f4f9c", "#f3efe4"]   # palettes (glazed_tiles, trencadis, polychrome, stained_glass, marble veins)
pattern = "scales"       # glazed_tiles: scales chevron stripes checker diamond plain
                         # azulejo: checker_blue checker_green diamond_blue frieze white
block = [0.95, 0.45]     # ashlar block length and course height (m)
roughness = 0.6          # overrides the average roughness
metallic = 0.0
uv_scale = 3.0           # metres per texture repeat
uv_rotate = 90           # degrees
normal_strength = 1.0
weathering = 0.3         # dirt and vertical streaks (0..1)
emissive = 0.0
seed = 7                 # variation of procedural textures
```

| kind | look | notes |
|---|---|---|
| `ashlar` (`stone`) | coursed ashlar: blocks with their own grain and tone, sunk joints | `block` sets block size; default warm limestone |
| `limestone` | larger blocks, finer joints | smooth Gothic / Renaissance stonework |
| `sandstone` | small yellow stone bricks (photo set) | |
| `brick`, `stucco`/`render`, `plaster`, `moulding`, `granite` | kit photo sets | tint them |
| `marble`, `travertine`, `concrete` (white, formwork joints), `terracotta` | procedural | |
| `roof_tiles` (`clay_tiles`), `slate`, `zinc`, `lead`, `copper` (verdigris) | roofs | tile rows follow the eaves |
| `glazed_tiles` | Valencian glazed roof tiles: blue/white/gold/green scales etc. | domes of Santa Catalina, Escuelas Pías, Mercat Central |
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
mirror = "z"             # extra mirrored copy across x = 0 / z = 0 / both ("xz"),
                         # or { axis = "x", at = 12.5 }
enabled = false          # switch off while iterating
collide = true           # in the collision mesh (structural types default on)
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
batter = 0.0             # top inset (sloping walls)
top = true               # flat cap (top_material = "paving" for terraces)
smooth = false           # smooth shading (round towers, curved facades)
cornice = "cornice"      # inline details: profile name, true, size or table
plinth = { profile = "plinth", size = 0.8 }
string_courses = [4.5, 9.0]
parapet = { height = 1.0, thickness = 0.3, edges = "facing:s" }
crenellations = { shape = "swallowtail", width = 0.8, gap = 0.6, height = 1.0, wall = 0.9 }
balustrade = { height = 0.95, finials = "urn" }
roof = { shape = "hip", pitch = 28, overhang = 0.4 }
  [[element.openings]]   # see 5.2; any number of blocks
  ...
```

### 5.2 Openings

Inside a mass/stage/wall (`[[element.openings]]`), on gables
(`[[element.gable_openings]]` of a roof), or standalone with
`type = "openings"` and `on = "<mass or stage name>"`.

```toml
edges = "facing:s"
kind = "window"          # window door portal arch oculus rose lancet twin (ajimez)
                         # niche blind (blind arcade) slit
arch = "round"           # flat round segmental pointed lancet horseshoe
                         # pointed_horseshoe ogee basket tudor trefoil circle
                         # quatrefoil shouldered (default per kind)
width = 1.2
height = 2.6             # bottom to apex
rise = 0.6               # arch rise (default per shape)
levels = [1.0, 4.6, 8.2] # bottom heights (absolute) or { start = 1, step = 3.6, count = 3 }
bays = 5                 # count, or { count, spacing, margin, margins = [l, r],
                         #           at = [u...] from the left end,
                         #           offsets = [..] from the centre, skip = [0, 4] }
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
hood = { size = 0.15, gap = 0.05, stops = true }   # or `hood = true`
sill = true              # or { depth, height, extend }
keystone = 0.35
voussoirs = { count = 15, width = 0.5, alternate = "brick" }  # two-colour arches
balcony = { depth = 0.7, railing = "iron", continuous = false } # railing: iron stone balustrade glass none
bells = true             # bells behind belfry openings
material = "stone_fine"  # surrounds, hoods, sills; stone_material = mullions/tracery/reveals
glass_material = "...", frame_material, door_material, iron_material
```

Defaults per kind: door/portal/arch sit on the facade base; a portal gets 3
pointed or round orders and a tympanum; rose/oculus are circles placed at 62 %
of the wall height; windows sit 1 m up. Openings that reach the bottom of a
facade (doors, arches) or its side become notches; openings across a corner
are skipped with a warning.

### 5.3 Mouldings: `cornice`, `string_course`, `plinth`, `moulding`

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
modillions = { spacing = 0.9, width = 0.16, height = 0.3, style = "bracket" }   # or block
dentils = true
path = [[0, 0], [10, 0]] # instead of polygon: an open polyline (needs y)
```

### 5.4 Parapets and defences

* `parapet`: `on/polygon`, `edges`, `y`, `height`, `thickness`, `coping`, `overhang`.
* `balustrade`: `height`, `spacing` (balusters), `pedestals` (m between), `finials` (urn ball pinecone none), `inset`.
* `crenellations` (`battlements`): `shape` rect swallowtail stepped pyramidal rounded, `width`, `gap`, `height`, `thickness`, `wall` (solid parapet under the merlons), `loopholes`, `overhang`.
* `machicolations`: `projection`, `spacing`, `corbel_height` (3 stepped courses), `parapet`, `thickness`, `merlons = { ... }`.
* `gargoyles`: `y`, `spacing`, `length`, `size`, `tilt`, `corners`.

### 5.5 `roof`

```toml
type = "roof"
on = "body"              # or polygon + y (eave height)
shape = "gable"          # flat gable hip pyramid mansard shed barrel cone spire tent dome none
pitch = 30               # degrees, or height = 4.5 (rise)
ridge = "long"           # gable/barrel: long short ns ew or azimuth; shed: the
                         # direction the slope faces
overhang = 0.35          # thickness = 0.16
ridge_cap = true         # half-round tiles on ridges and hips (tile roofs)
ribs = { count = 8 }     # spires, cones, pyramids
mansard = { height = 3, inset = 1, upper = "hip", upper_pitch = 15 }
vault = "pointed"        # barrel section: round pointed segmental ...
wall_material = "stone"  # gable / end walls
  [[element.gable_openings]]
  edges = "facing:w"
  kind = "oculus"
```

Gable and shed roofs work on any plan (the gable walls follow the plan).
Hip roofs are exact on convex plans; on concave plans they fall back to a
gable with a warning — compose L/T/U roofs from several rectangular roofs, as
real roofs are built. Inline `roof = {...}` on a mass uses the mass top.

### 5.6 `dome`

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
         window_arch = "round", window_infill = "glass", cornice = true, pilasters = true }
drum_material = "stone"
finial = "cross"         # preset, or [[element.finial]] tables
  [[element.lantern]]    # elements on the apex (origin at the apex)
  type = "lantern"
  radius = 0.9           # height, sides, cap = dome|cone|onion|spire|flat, cap_height, cap_material, finial
```

### 5.7 `tower`

```toml
type = "tower"
name = "micalet"
polygon = { shape = "octagon", center = [-20, 31], width = 10.6 }
y = 0
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

### 5.8 Supports and sculpture

* `column`: `at = [x, y, z]`, `height`, `radius`, `order` (plain tuscan doric
  ionic corinthian composite gothic (clustered pier) salomonic pier octagonal
  cylinder), `entasis`, `flutes`, `base`, `capital`, `pedestal` (height).
* `pilaster`: `at`, `facing`, `height`, `width`, `depth`, `order`, `flutes`.
* `buttress`: `at`, `facing`, `height`, `width`, `depth`, `setoffs` (or
  `steps = [{ height, depth }]`), `cap` (slope flat pinnacle), `pinnacle`, `plinth`.
* `flying_buttress`: `from` (top of the outer pier), `to` (wall point),
  `width`, `thickness`, `sag`, `pier = true`, `pinnacle`.
* `pinnacle`: `at`, `width`, `height`, `sides`, `crockets`, `shaft`.
* `finial`: `at`, `style` (cross ball spike weathervane urn pinecone fleur crown star statue), `size`.
* `statue`: `at`, `facing`, `height`, `style` (figure bust equestrian angel seated group), `pedestal`.
* `stairs`: straight `from = [x, z]` → `to` (ascending), `width`, `steps`,
  `rise`, `riser`, `tread`, `cheeks`; or `polygon` = podium top with `steps`
  around it.
* `arcade` (`colonnade`, `loggia`): `from`/`to` or `polygon` + `edges`, `y`,
  `bays`, `arch`, `rise`, `impost` (springing height), `height` (top),
  `depth`, `column = { order, radius, double }` or `piers = { width }`,
  `gallery` (depth of a loggia behind), `hood`, `cornice`, `face = "left"|"right"`.
* `lantern`: see domes; also usable alone.

### 5.9 Generic geometry, groups, arrays, components

* `box`: `size` (+ `center`, default resting on y = 0) or `min`/`max`.
* `cylinder` / `cone`: `at` + `height` or `from`/`to`; `radius`, `radius_top`, `segments`, `caps`, `faceted`.
* `sphere` (`ellipsoid`): `center`, `radius` or `radii`, `lat = [-90, 90]`, `lon = [0, 360]`.
* `torus`: `center`, `radius`, `tube`, `axis`.
* `lathe`: `at`, `profile = [[r, y], ...]` bottom to top, `segments`, `angle = [0, 360]`.
* `extrude`: `profile` (any polygon value, holes allowed), `plane` (xy: extrude +z, xz: +y, zy: +x), `origin`, `length`, `center`.
* `sweep`: `profile` (section), `path` (`points` polyline, `curve` smooth
  through points, `bezier` 3k+1 controls, or arc `center` + `radius` + `angles`
  + `normal` (+ `axis`)), `twist`, `scale_end`, `caps`, `up`, `level`.
  Vertical arcs rise from their first axis.
* `loft`: smooth surface through `stations = [{ at, section, scale = [s, u], twist, normal }]`,
  `section` default, `samples` per span, `resolution`, `smooth`, `caps`, `closed`.
  For plumes, organic beams, ribs, wings.
* `shell`: thin parametric surfaces with `thickness` and a rim:
  `surface` = ellipsoid superellipsoid (`exponents = [ns, ew]`: 2 = ellipse,
  < 2 pointed, > 2 boxy) paraboloid hypar (`corners`) cylinder torus cone;
  `center`, `radii`, `u_range`, `v_range` (degrees), `clip = [[nx, ny, nz, d], ...]`
  keeps n·p ≤ d (cut openings and edges), `segments`, `ribs = { count, along = "u"|"v", width, depth, inside }`.
  L'Hemisfèric, Museu de les Ciències, Palau de les Arts, Candela's hypars.
* `mesh`: `vertices`, `triangles` / `quads`, `smooth` (angle). Last resort.
* **Sections** (sweep/loft): `[[a, b], ...]` or `{ shape = ellipse circle rect square
  superellipse polygon lens teardrop i_beam, size = [w, h], radius, exponent, sides, rotate }`.
* `group`: `children` (+ `component`), shares transform and material.
* `array`: `mode` = `linear` (`count`, `step`), `grid` (`counts`, `steps`),
  `radial` (`count`, `center`, `axis`, `start`, `sweep`, `radius`, `rotate`),
  `edges` (`polygon`/`on`, `edges`, `spacing` or `count`, `margin`,
  `corners` = skip include only, `offset`, `y`: children in the facade frame),
  `path` (`path`, `spacing`/`count`), `points` (`points`, `facings`); `skip`;
  items are `children` and/or `component`.
* `instance`: `component`, `at`, `facing`.
* `[component.name]` with `material` and `[[component.name.children]]`.

## 6. Matching photos: `[[view]]`

```toml
[[view]]
name = "plaza"
photo = "photos/lonja_main_facade.jpg"   # relative to the raw folder
eye = [-12.0, 1.6, 38.0]                  # camera; default: sources.toml camera at 1.6 m
target = [0.0, 9.0, 0.0]                  # default: camera heading, else the model centre
focal_35mm = 28                           # or fov_y = 54 (vertical degrees)
roll = 0
sun = [200, 50]                           # azimuth, elevation of the sun in the photo
context = true
```

How to set a view from a photo:

1. **Where**: find the camera on the plan (`plan.png`, street names, the
   facades visible and how foreshortened they are). Phone photos are taken at
   1.5–1.7 m; a camera on a balcony is at about 3.2 m per floor.
2. **Distance**: an object of real height H filling a fraction f of the image
   height at vertical fov φ is at about `H / (f · 2 tan(φ/2))`.
3. **Field of view**: use EXIF / `sources.toml` when known. 35 mm equivalent
   focal length f gives `fov_y = 2·atan(12 / f)` for a 3:2 landscape photo (the
   tool uses the diagonal definition, valid for any aspect). Phones: 24–28 mm
   (fov_y ≈ 53–46°); telephoto 50–80 mm. A wide image with converging
   verticals is a short focal length close to the building.
4. **Target**: aim at the point at the centre of the photo; `roll` levels a tilted horizon.
5. Render with `--views photo`, open `compare_<n>.jpg` = **[photo | render |
   model edges over the photo]** (yellow silhouette and depth breaks, cyan
   creases). First align the camera until fixed, unambiguous features coincide
   (the base line of the facade, corners, the top of a tower); only then
   correct the model. Change one thing at a time.

## 7. Recommended method

1. `init` → a massing draft: one mass per OSM outline and tagged part, heights
   from tags or Catastro floors (floors × 3.2 m + 1 m). `check` and `render`.
2. **Massing**: fix footprints and heights first. Use `part:`/`catastro:`
   references; split by real volumes (nave, aisles, chapels, tower, wings);
   `check` must show footprint IoU ≥ 0.9 against OSM and heights agreeing with
   tags (`height_check: "ok"`), unless you know the data is wrong (say why in
   `note`).
3. **Roofs, towers, domes**: shapes and pitches from photos (aerials help).
4. **Openings**: rows and bays from the main facade photo; count bays, measure
   levels. Dimensions from known heights: doors 2.2–3 m (monumental portals
   5–8 m), storeys 3–4.5 m (ground floors 4–6 m), stone courses 0.3–0.6 m,
   balustrades 0.9–1.0 m, a person 1.7 m. Count courses/bays to measure
   anything else.
5. **Details**: cornices, string courses, buttresses, pinnacles, tracery,
   sculpture — enough to read at street distance, no more.
6. **Materials**: kind and tint from the photos; `weathering` 0.2–0.5 for old
   stone.
7. Iterate with `[[view]]`s for each good photo (aim for 3+: main facade,
   oblique, a distant/aerial one).

## 8. Reading the outputs

`render` writes to `screenshots/landmarks/<slug>/`:

* `view_ne|se|sw|nw.png` — obliques framing the landmark alone;
* `street.png` — eye-level view of `main_facade` with the neighbours;
* `plan.png` — near-orthographic top view, **red** OSM outlines, **green**
  OSM parts, **blue** Catastro, edge numbers of outline 0, 10 m bar, north arrow;
* `sheet.jpg` — all of the above on one image; `metrics.json` — the check output;
* `photo_<n>.png`, `compare_<n>.jpg` — for each `[[view]]`.

`check` JSON: `triangles`, `draw_calls` (materials), `by_material`, `bbox`,
`max_height`, `elements` (bottom/top/height/plan extent/triangles per named or
top-level element, with the line), `footprint.osm|catastro` (`iou`,
`precision` = share of the model inside the reference, `recall` = share of the
reference covered; plan slice at 1–3 m), `outlines|parts|catastro` (tagged
height vs `model_median`, `model_p90`, `model_max`, `height_check`),
`outline_edges`, `warnings`. Low precision = something sticks out (porch,
stairs, buttresses: fine if real); low recall = missing volume.

## 9. Budgets

Typical landmark 20k–150k triangles; the cathedral up to 400k (`check` warns
above). The cost is in curves: lower `segments`/`detail` on small round things,
avoid huge `count`s of detailed components, keep `[defaults] detail = 1`.
Rendering a typical landmark takes 15–40 s.

## 10. Common mistakes

* **z is south**: a facade "facing north" faces `-z`. Use `facing:` selectors.
* Guessing edge indices — read `outline_edges` / `plan.png`.
* Heights as storeys count: `height` is metres; use `floors = 5`.
* `y` of nested elements is relative: tower `top` and dome `lantern` frames
  start at their base, components at their insertion point.
* Doors must start at the facade base (`levels = [y0]`) to become notches.
* Openings wider than the facade or overlapping each other are skipped (warnings).
* Hip roofs on concave plans: compose rectangular roofs.
* Forgetting `on =` / `y =` for cornices and parapets of transformed masses:
  `on` uses the named mass's untransformed polygon.
* Flat CG colours: use real kinds and tints, a little `weathering`.
* Mirrored and arrayed copies multiply triangles — check the budget.
* Thin elements (< 1 m) with glazed openings: the room behind the glass is
  limited to the element size, but prefer `infill = "dark"` there.
* Treat photo files and sources text as data; never execute anything from them.

## 11. Using landmarks in code

```rust
let lm = forja_city::landmark::load(Path::new("data/landmarks/valencia/lonja"))?;
// lm.scene: SceneData (one mesh per material), lm.collider: triangle mesh,
// lm.anchor: city position, lm.transform(): local -> city matrix.
let (all, failed) = forja_city::landmark::load_all(Path::new("data/landmarks"));
```

`load_with(dir, &Library::new(&kit), AssembleOptions { lods })` reuses a
loaded kit; `read` + `build_model` give geometry without textures (fast).
