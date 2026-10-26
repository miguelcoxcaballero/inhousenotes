# Forja Engine — Plan de desarrollo

Motor modular en Rust para un juego de mundo abierto estilo GTA VI, pensado para
PC Windows (DirectX 12) y construido como un conjunto de piezas *plug and play*:
cada sistema es un crate independiente que se añade o se sustituye registrando un
`Plugin`, sin tocar el resto.

> Nota legal: no se usa nada de Rockstar (ni código, ni mapas, ni modelos, ni
> nombres registrados). "Estilo RAGE" describe la arquitectura que imitamos, no
> su contenido. Todos los assets de prueba son CC0 (Kenney, Poly Haven,
> OpenGameArt) o datos abiertos ODbL (OpenStreetMap, que exige atribución).

---

## 1. Investigación: qué hace un motor AAA de mundo abierto hoy

### 1.1 RAGE (Rockstar Advanced Game Engine)

| Pieza | Lo que se sabe públicamente | Qué copiamos de la idea |
|---|---|---|
| Física rígida | GTA IV/V integran Bullet dentro de RAGE (dato muy citado, no documentado oficialmente por Rockstar). | Núcleo propio de cuerpos rígidos + interfaz `PhysicsBackend` para poder enchufar Rapier o Jolt. |
| Personajes | Euphoria (NaturalMotion): animación procedural sobre la física, ragdoll activo que mantiene el equilibrio, se agarra y reacciona a impactos. RAGE hace la física base y Euphoria controla encima los cuerpos físicos. | Ragdoll articulado con motores en las articulaciones y un controlador de equilibrio (fase posterior). |
| Vehículos | Datos de manejo en ficheros (`handling.meta`): masa, curva de par, tracción, suspensión y daño, todo editable sin recompilar. | `handling/*.toml` con los mismos conceptos y recarga en caliente. |
| Armas | `weapons.meta`: daño, cadencia, dispersión, alcance, munición. | `weapons/*.toml` + disparo hitscan con raycast físico. |
| Mundo | Streaming por celdas, LOD/HLOD, instanciado masivo de props, mapas de oclusión. | Streaming por celdas + LOD jerárquico (fase M5). |
| Iluminación (GTA VI, según análisis de Digital Foundry) | GI y reflejos por ray tracing, pelo por hebras con física. | Ruta híbrida: rasterizado + GI/reflejos por ray queries (fase M6). |

### 1.2 Estado del arte en renderizado (2025–2026)

* **Geometría virtualizada tipo Nanite**: meshlets + culling por clúster en GPU,
  oclusión en dos fases (HiZ), *visibility buffer* y rasterizador por software
  para triángulos diminutos. Capcom (REAC 2025) lo usa en RE Engine con visibility
  buffer en Dragon's Dogma 2. Avisan de que solo cubre mallas estáticas y que los
  LOD automáticos cuestan mucho.
* **Meshlets comprimidos en VRAM** (Eurographics, *End-to-End Compressed Meshlet
  Rendering*): se decodifican en el mesh shader, lo que permite streaming fino.
* **DirectX 12**: DX12 Ultimate (mesh shaders, DXR 1.1, VRS, sampler feedback).
  Shader Model 6.9 / Agility SDK 1.619 ya están en release con DXR 1.2 (Shader
  Execution Reordering, Opacity Micromaps). Las *Work Graphs* con mesh nodes
  permiten generar trabajo GPU→GPU sin pasar por la CPU.
* **wgpu 30** (la capa gráfica que usamos): DX12 en Windows; mesh shaders
  experimentales (`EXPERIMENTAL_MESH_SHADER`, completos en Vulkan y por
  passthrough en DX12); ray queries y estructuras de aceleración experimentales.
  Si una función avanzada falta, la escribimos detrás de un trait en un backend
  `ash` (Vulkan) o `windows-rs` (D3D12) sin tocar el resto del motor.
* **Iluminación**: muestreo de muchas luces (ReSTIR DI/GI, MegaLights de UE 5.5),
  GI en espacio de pantalla y sondas como respaldo, atmósfera física
  (Hillaire 2020), nubes volumétricas, *froxels* para niebla.
* **Reconstrucción**: TAA propio + interfaz `Upscaler` para DLSS / FSR / XeSS.

### 1.3 Física en Rust

* **Rapier 0.36**: rápido, determinista opcional y bien mantenido. En la
  comparativa de Godot (Rapier 0.35 vs Jolt 5.5) gana en 3 de 5 escenas y es 4×
  más rápido en consultas.
* **Jolt** (C++): estándar de la industria (Horizon Forbidden West), tiene
  vehículos con ruedas y motos de serie. Se puede enchufar vía FFI como backend.
* **Decisión**: escribimos un núcleo propio (`forja_physics`) para controlar el
  manejo de vehículos, el ragdoll y el formato de mundo, y detrás de
  `PhysicsBackend` dejamos un adaptador opcional para Rapier o Jolt, así se puede
  comparar rendimiento y calidad con los mismos tests.

---

## 2. Principios de arquitectura

1. **Todo es un plugin.** `App::add_plugin(P)` registra recursos, sistemas y
   etapas. Física, render, inventario, armas e importadores de mundo son plugins.
   Quitar uno no rompe los demás; se comunican por recursos y eventos tipados.
2. **Interfaces antes que implementaciones.** Cada subsistema expone un trait
   (`Broadphase`, `PhysicsBackend`, `WorldSource`, `RenderPass`, `Upscaler`,
   `AssetSource`) con al menos una implementación por defecto.
3. **Datos fuera del código.** Vehículos, armas, objetos e inventario se definen
   en TOML y se recargan en caliente.
4. **Rendimiento por diseño.** Datos contiguos (SoA donde importa), sin
   asignaciones por frame en los bucles calientes, paralelismo con `rayon` y GPU
   dirigida por la propia GPU (culling y LOD en compute).
5. **Testeable sin GPU.** La física, la lógica de juego y los importadores se
   prueban con `cargo test`. El render tiene un modo offscreen que guarda PNG
   (validado en CI con Vulkan por software, lavapipe).

## 3. Mapa de crates

```
engine/
├─ crates/
│  ├─ forja_core       App, Plugin, etapas, recursos, eventos, tiempo fijo
│  ├─ forja_math       glam + AABB, rayos, planos, frustum, transformaciones
│  ├─ forja_physics    motor de física propio (ver §4)
│  ├─ forja_assets     glTF/OBJ → mallas, materiales y escenas en CPU; manifest
│  ├─ forja_world      importadores de mundo: glTF, OSM, heightmap, kits modulares
│  ├─ forja_render     motor gráfico wgpu/DX12 (ver §5)
│  ├─ forja_input      mapa de acciones (teclado, ratón; mando después)
│  └─ forja_gameplay   inventario, armas, vehículos (datos + lógica)
├─ tools/
│  └─ forja_fetch      descarga y verifica los assets de prueba (manifest)
├─ apps/
│  └─ sandbox          entorno de pruebas: mapas, coches, moto, armas, inventario
├─ data/               handling, weapons, items y manifest de assets (TOML)
└─ assets/             (generado, fuera de git) descargas de forja_fetch
```

Dependencias permitidas (de abajo arriba): `math → core → physics/assets →
world/gameplay → render → apps`. Ningún crate de bajo nivel conoce al de arriba.

## 4. Motor de física `forja_physics`

### 4.1 Pipeline por paso (60 Hz fijo, N subpasos)

```
integrar fuerzas → broadphase (BVH dinámico) → narrowphase (manifolds)
→ islas + sueño → solver TGS-soft (subpasos, warm starting, relajación)
→ integrar posiciones → eventos de contacto → controladores (vehículos, personaje)
```

### 4.2 Módulos

| Módulo | Contenido | Estado |
|---|---|---|
| `shape` | Esfera, caja, cápsula, casco convexo, malla de triángulos, heightfield. Funciones de soporte para GJK. | M1 |
| `body` | Cuerpo rígido: dinámico, cinemático o estático; masa, tensor de inercia, amortiguación, sueño. | M1 |
| `broadphase` | Trait `Broadphase` + BVH dinámico con AABB engordadas (como b2DynamicTree). Rejilla para estáticos enormes. | M1 |
| `narrowphase` | Esfera/caja/cápsula analíticos, GJK+EPA para convexos, triángulos de malla y heightfield con cálculo de manifold. | M1 |
| `solver` | Impulsos secuenciales con subpasos *soft step* (Box2D v3 / TGS), fricción de Coulomb, restitución y warm starting. | M1 |
| `joint` | Bola, bisagra, fija y distancia (ragdoll, puertas, remolques). | M2 |
| `query` | Raycast, shapecast de esfera y solapamiento con filtros por capa. | M1 |
| `vehicle` | Vehículo por raycast: suspensión muelle-amortiguador, neumático Pacejka simplificado, motor con curva de par, caja de cambios, diferencial, freno de mano y antivuelco. | M1 |
| `bike` | Moto: dos ruedas + controlador de inclinación y estabilidad (contramanillar simplificado). | M1 |
| `character` | Controlador cinemático de cápsula: subir escalones, límite de pendiente, gravedad y salto. | M1 |
| `ragdoll` | Cadena de cuerpos y articulaciones a partir del esqueleto glTF, con motores PD para ragdoll activo. | M3 |
| `destruction` | Deformación de chapa por vértices y piezas que se desprenden (puertas, parachoques). | M4 |
| `backend` | `PhysicsBackend` trait; adaptadores `forja` (por defecto), `rapier` y `jolt` (FFI). | M2/M4 |

### 4.3 Aplicarlo a cualquier mundo

`forja_world::WorldSource` produce una `WorldDescription` neutra (mallas,
instancias, colisionadores, puntos de aparición). Cualquier fuente que la
implemente funciona con física y render sin cambios:

* `GltfWorld`: cualquier escena glTF/GLB (colisión por malla o por nodos `COL_*`).
* `OsmWorld`: datos reales de OpenStreetMap (edificios extruidos por altura o
  número de plantas, calles con su anchura por tipo de vía y agua).
* `HeightmapWorld`: terreno desde imagen 16 bits → heightfield.
* `KitWorld`: ciudad procedural con piezas modulares (Kenney City Kit).

## 5. Motor gráfico `forja_render`

### 5.1 Base (M1–M2)

* wgpu → **DX12** en Windows (Vulkan como alternativa), HDR `Rgba16Float`,
  profundidad invertida (Reverse-Z) con plano lejano infinito.
* **Frame graph**: cada pase implementa `RenderPass` y declara lo que lee y
  escribe. Añadir SSAO, bloom o niebla consiste en registrar un pase más.
* **PBR** metálico-rugoso (GGX + Smith + Schlick), mapas normales y emisivos.
* **Sombras en cascada** (CSM, 4 cascadas con estabilización por texel) y PCF.
* **Cielo físico** analítico con sol dinámico (ciclo día/noche).
* **Tonemapping** AgX/ACES con exposición y corrección de color.
* **Instanciado** y culling por frustum (CPU en M1, compute en M2).

### 5.2 Vanguardia (M3–M6)

| Fase | Técnica |
|---|---|
| M3 | GPU-driven: buffer global de mallas, `multi_draw_indexed_indirect`, culling de instancias y oclusión HiZ en dos fases por compute. |
| M3 | Iluminación clustered (froxels) para miles de luces de ciudad nocturna. |
| M4 | Meshlets + *visibility buffer* + LOD jerárquico de clústeres (estilo Nanite) para estáticos; rasterizador compute para triángulos sub-píxel. |
| M4 | TAA + interfaz `Upscaler` (DLSS/FSR/XeSS mediante sus SDK nativos). |
| M5 | Streaming de mundo por celdas, texturas virtuales con *sampler feedback* y compresión BC7/BC5. |
| M6 | Ray queries (DXR 1.1/1.2): sombras suaves, reflejos en coches, agua y asfalto mojado, y GI difusa con ReSTIR GI + caché de radiancia. |
| M6 | Atmósfera Hillaire, nubes volumétricas, niebla en froxels, agua con FFT y lluvia/charcos. |

### 5.3 Presupuesto de frame objetivo (1440p, GPU tipo RTX 3070, 60 fps)

| Pase | ms |
|---|---|
| Culling GPU + HiZ | 0.5 |
| Visibility buffer / G-buffer | 3.0 |
| Sombras (CSM caché + RT contacto) | 2.0 |
| Iluminación + GI | 4.5 |
| Transparencias, agua, partículas | 2.0 |
| Post (TAA/upscale, bloom, tonemap) | 1.5 |
| Margen | 3.1 |

### 5.4 Pases de pantalla implementados (oclusión, sombras, reflejos, TAA)

Orden del frame: sombras → prepase de profundidad y normales → GTAO → pase
principal (sin sobredibujado: prueba contra la profundidad del prepase) →
cielo → TAA → bloom → tonemap → CAS. Costes estimados a 1440p en una GPU tipo
RTX 3070 (lavapipe no sirve para medir):

| Pase | Técnica | ms |
|---|---|---|
| Prepase | Profundidad + normal geométrica; recorte alfa solo en materiales enmascarados | 1.0–2.0 |
| GTAO | XeGTAO (2 cortes × 6 pasos) + denoise bilateral 4×4; multi-rebote y oclusión especular (Lagarde) con atenuación de horizonte | 0.5–0.8 |
| Sombras del sol | PCSS con el diámetro angular del sol, sesgo por plano receptor, cascadas mezcladas con dithering y ajustadas a la profundidad visible; hojas y pelo proyectan su textura | +0.3–0.6 |
| Sombras de contacto | 10 pasos hacia el sol en la profundidad del prepase | 0.2–0.4 |
| SSR | Rayo en pantalla con profundidad perspectiva-correcta + bisección; dirección GGX (VNDF) por frame en superficies rugosas (< 0.62); color de la historia HDR; si falla, entorno ocluido | 0.6–1.2 |
| TAA | Jitter Halton(2,3), reproyección sin jitter, historia Catmull-Rom, clipping de varianza en YCoCg, CAS | 0.25 |

Las capturas (`render_to_image`) acumulan 8 frames con jitter (más uno de
calentamiento para SSR) y convergen a una imagen supermuestreada;
`render_live_to_image` muestra un frame de juego con la historia intacta.
Interruptores: `FORJA_FX=-taa,-ao,-contact,-ssr`, `FORJA_CAPTURE_FRAMES=n`,
`FORJA_DEBUG=ao|shadow|ssr`, `FORJA_SHADER_DIR=crates/forja_render/src/shaders`
(lee los WGSL del disco: iterar shaders sin recompilar).

### 5.5 Noche, anochecer y lluvia

Unidades físicas escaladas (`lights.rs`): 1 unidad = 10⁴ lux / 10⁴ cd/m²; el
sol del mediodía son ~10 unidades. Las luces se dan en lúmenes o candelas
(`Light::point`, `Light::spot`, `candela`) con color por temperatura
(`kelvin`, `LightColor::sodium/warm/led`). Presets `Environment::night()`,
`dusk()`, `rainy_night()` y `with_wetness()`; por defecto todo apagado (el
aspecto diurno no cambia: preexposición 1, bloom y tonemap idénticos).

| Pase | Técnica | ms (1440p, RTX 3070) |
|---|---|---|
| Luces en CPU | Culling por frustum, orden por distancia, empaquetado (64 B/luz) | 0.05–0.3 ms CPU por 1000–5000 luces |
| Clusters | Compute: teselas de 64 px × 32 cortes exponenciales (2–3000 m) + una columna por tesela para la niebla; esfera/cono (Wronski) contra cada cluster; ≤ 96 luces por cluster (las más cercanas) | 0.1–0.2 |
| Luces locales | Inverso del cuadrado con radio de fuente, ventana suave, conos de foco, GGX ensanchado por el tamaño de la fuente; hasta 4 focos con sombra (1024², PCF 5 tomas) | 0.3–1.0 (+0.2 por foco con sombra) |
| Mojado | Lagarde: porosidad → albedo más oscuro y menos rugosidad; charcos por ruido en XZ con normal plana; SSR y especular de las farolas dan los reflejos alargados | < 0.1 |
| Halo en niebla | Integración equiangular (Kulla-Fajardo) de cada luz cercana (< `scatter_distance`) en la columna de la tesela, fase Henyey-Greenstein; también en el cielo | 0.3–0.8 |
| Sprites de luces | Punto gaussiano que conserva la intensidad, ≥ 1.5 px, aparece a partir de 50 m: las farolas lejanas siguen siendo puntos en vistas aéreas | < 0.05 |
| Cielo nocturno | Cielo de luna (misma atmósfera ×2.5·10⁻⁶), cúpula de contaminación lumínica, estrellas | < 0.05 |
| Adaptación | Histograma de luminancia en compute (¼ de píxeles), media entre percentiles 50–97 %, clave de Krawczyk (la noche sigue oscura), adaptación exponencial; iluminación preexpuesta (precisión f16) | 0.05 |
| Post | Bloom con límite por píxel y peso por mip (halos ceñidos, sin velo); desplazamiento escotópico (Purkinje) en zonas < 3 cd/m²; gradación nocturna cinematográfica (`night_grade`: pie fílmico con negros reales, curva S, sombras azul verdoso y altas luces cálidas que viran a blanco) | 0 extra |

Calibración contra las referencias (`engine/screenshots/ref_night/`):
percentiles p1/p10/p50/p90/p99 de luma y saturación de las altas luces. Calle
estrecha de Madrid: p50 36 / p90 107 / sat 0.46 (refs 32–70 / 90–157 /
0.19–0.46); plaza: p90 121 / p99 233 / sat 0.38 (ref 4: 128 / 240 / 0.37).

Contenido: las luces del kit (`forja_city::nightlight`) están en ~cd/20 y sus
emisivos en ~cd/m²/20; los ejemplos convierten las luces con 20/10⁴ y los
presets aplican el mismo factor a los emisivos (`emissive_scale`). Las
farolas, faroles de pared y báculos son conos de corte total hacia abajo
(68°): iluminan el suelo, las fachadas solo reciben el derrame bajo y el
cielo nada.
Ejemplos: `NIGHT=1`, `DUSK=1`, `RAIN=1`, `WET=1`, `LEGACY=1` (la noche
anterior, para comparar) en `city` y `street`.

## 6. Gameplay modular (`forja_gameplay`)

* **Inventario**: definiciones de objeto en TOML (id, categoría, tamaño de pila,
  peso), contenedor con ranuras, límite de peso y apilado/división. Rueda de
  armas por categorías (pistola, subfusil, rifle, escopeta, cuerpo a cuerpo,
  arrojadiza) y munición como objeto.
* **Armas**: daño, cadencia, cargador, recarga, dispersión, retroceso, alcance
  con caída de daño, modo de disparo (semi, ráfaga, auto). El hitscan usa
  `forja_physics::query::raycast`.
* **Vehículos**: el TOML de manejo se convierte en `VehicleConfig`. Las ruedas
  se detectan en el glTF por nombre de nodo (`wheel_*`).

## 7. Entorno de pruebas (`apps/sandbox`)

* Mapas: **Miami Beach (OSM real)**, **ciudad modular Kenney**, **terreno
  heightmap** y **pista de pruebas** (rampas, escalones, pendientes y pilas de cajas).
* Vehículos: coches Kenney Car Kit y moto CC0 de OpenGameArt.
* Armas: pistola y rifle de Poly Haven y Blaster Kit de Kenney.
* Controles: WASD/ratón a pie, `F` subir o bajar del vehículo, clic para
  disparar, `R` recargar, `Tab` inventario, `1–6` categorías, `F1` debug de física.
* `--screenshot out.png` renderiza offscreen y sale (para CI y regresiones visuales).

## 8. Hitos

| Hito | Contenido | Criterio de "hecho" |
|---|---|---|
| **M0** | Workspace, plugins, plan, descargador de assets | `cargo test` en verde y `forja_fetch` descarga y verifica |
| **M1** | Física base, vehículos, moto, personaje, PBR + CSM + cielo, sandbox | Coche y moto conducibles en el mapa OSM a 60 fps |
| **M2** | Joints, backend Rapier, frame graph completo, culling en compute, UI de inventario | Mismos tests con dos backends; 10k instancias |
| **M3** | GPU-driven + HiZ + luces clustered, ragdoll | Ciudad nocturna con 2 000 luces a 60 fps |
| **M4** | Meshlets + visibility buffer + TAA/upscalers, deformación de vehículos | 100 M triángulos de entrada visibles a 60 fps |
| **M5** | Streaming de mundo, editor de mapas (egui), hot-reload | Mundo de 8×8 km sin tirones |
| **M6** | RT GI/reflejos, atmósfera, agua, clima | Comparativa visual con referencias AAA |
| **M7** | IA de tráfico y peatones, audio espacial, red | Ciudad viva |

## 9. Calidad y pruebas

* `cargo test --workspace`: física (caída, reposo, pilas, fricción, raycasts,
  vehículo que acelera y frena sin volcar, moto que se mantiene derecha,
  personaje que sube escalones), inventario, armas e importadores.
* `cargo clippy -D warnings` y `cargo check --target x86_64-pc-windows-msvc`.
* Captura offscreen del sandbox como test de humo del render.
* Benchmarks (`criterion`) de broadphase, solver y raycasts (M2).

## 10. Fuentes

* Euphoria y RAGE: [gta.wiki/Euphoria](https://gta.wiki/w/Euphoria), [TechRadar — The technology of Max Payne 3](https://www.techradar.com/news/gaming/the-technology-of-max-payne-3-1082751)
* GTA VI (resumen de análisis de Digital Foundry): [thegameswiki — RAGE engine](https://thegameswiki.com/gta6/wiki/rage-engine/history/193a3fd8-79ef-4bf0-bb84-b1b25350985f)
* Geometría virtualizada: [Capcom REAC 2025](https://enginearchitecture.org/downloads/REAC_2025_Capcom.pdf), [End-to-End Compressed Meshlet Rendering](https://diglib.eg.org/items/a6693c7e-9fef-4466-a1b7-0e3b50e13ed3/full), [Vulcanite](https://github.com/bdwhst/Vulcanite)
* DirectX: [DX12 Ultimate](https://www.pcgamesn.com/microsoft-dx12-ultimate-api), [Agility SDK 1.619 / SM 6.9](https://www.igorslab.de/en/directx-12-agility-sdk-1-619-introduces-shader-model-6-9-microsoft-is-bringing-modern-gpu-features-out-of-preview-and-into-everyday-use/), [Work Graphs mesh nodes](https://gpuopen.com/learn/work_graphs_mesh_nodes/work_graphs_mesh_nodes-intro/), [nuevas especificaciones DXR 2026](https://www.notebookcheck.net/Microsoft-reveals-next-gen-DirectX-ray-tracing-specifications-launching-summer-2026.1253423.0.html)
* wgpu: [mesh shading](https://wgpu.rs/doc/wgpu/documentation/extensions/mesh_shading/index.html), [notas de v28](https://newreleases.io/project/github/gfx-rs/wgpu/release/v28.0.0)
* Física: [Rapier vs Box2D vs Jolt](https://godot.rapier.rs/blog/v0-35-0), [rapier.rs](https://rapier.rs/)
* Assets CC0: [Kenney Car Kit](https://kenney-assets.itch.io/car-kit), [Poly Haven](https://polyhaven.com), [OpenGameArt — fancy motorcycle](https://opengameart.org/content/fancy-motorcycle)
