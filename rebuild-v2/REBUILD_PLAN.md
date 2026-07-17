# Plan de reconstrucción — Inhouse Notes v2

> Documento de trabajo para la(s) próxima(s) sesión(es). Contiene el diagnóstico del
> código actual ([index.html](index.html), 28.368 líneas, ~800 funciones, un solo archivo),
> la arquitectura nueva propuesta y un plan de ejecución por fases.
> Objetivo: **misma app, mismas features, cero bugs estructurales, código profesional.**

---

## 1. Resumen ejecutivo

La app funciona, pero los tres problemas reportados (escritura/movimiento poco fluidos,
bugs de guardado, timeline roto) **no son bugs aislados: son consecuencias directas de
tres decisiones de arquitectura**:

1. **El PDF es el formato de persistencia en Drive.** Los trazos van comprimidos en
   base64 dentro del campo *Keywords* de los metadatos del PDF (`STROKES_Z`), el
   historial de versiones también va ahí, y el PDF original limpio viaja como adjunto
   embebido (`IH_CLEAN_ORIGINAL.pdf`). Cada autosave reconstruye el PDF completo
   (renderizar overlays PNG por página + manipulación pdf-lib + compresión) y resube
   el archivo entero. Eso convierte cada guardado en una operación de cientos de ms a
   segundos **en el hilo principal**, y de ahí nace toda la maquinaria de
   "Save paused...", watchdogs, deferred saves, retries y abort controllers
   (≈1.500 líneas solo en §4.17) que aun así no logra que escribir sea fluido.

2. **Todo está indexado por posición (`pageIndex`), no por identidad.** IndexedDB
   guarda páginas con el índice como clave, la cola de stroke-ops usa `pageIndex`,
   el undo usa índices de array. Cualquier reorden/inserción/borrado de páginas (o un
   merge colaborativo) obliga a re-clavear todo, y las carreras se tapan con tokens de
   sesión y "structure versions" comprobados a mano en docenas de sitios. Cuando una
   comprobación falta → bug de guardado.

3. **Estado global disperso.** Además del objeto `state`, hay **465 variables
   `let/const` a nivel de módulo** que forman máquinas de estados implícitas
   (driveDirty, deferredDriveSave, driveSaveQueued, driveSaveQueuedForce,
   strokeOpsFlushScheduled, collabPullInProgress, …). Es imposible razonar sobre
   ellas o testearlas.

**La solución no es parchear: es reconstruir sobre un modelo de datos correcto**,
manteniendo las (muchas) ideas buenas que ya existen: coalesced events, finalización
incremental de trazos, anclaje por `pageId` durante el dibujo, tombstones para el
merge colaborativo, fases de presencia.

---

## 2. Inventario funcional (paridad — todo esto debe seguir existiendo)

- **Editor de tinta**: pluma con presión, subrayador (alpha 0.35, min-dist), borrador
  por trazo y por área, lasso (seleccionar/mover/escalar trazos e imágenes), imágenes
  pegadas/redimensionables, plantillas de página (default / agenda / diary), portada.
- **Páginas**: scroll infinito vertical, crear página con overscroll inferior,
  panel "Manage pages" (reordenar/añadir/borrar), lazy-load/unload con previews,
  páginas de tamaño heterogéneo (PDF).
- **PDF**: importar (archivo/URL), anotar encima conservando texto/vectores
  (overlay pdf-lib), exportar (estándar y alta calidad), revertir al original limpio.
- **Persistencia local**: IndexedDB + backup localStorage, recuperación tras crash
  (cola de stroke-ops), funciona sin sesión de Drive.
- **Drive**: sign-in Google (PKCE + refresh), home con grid de archivos, carpetas,
  papelera, búsqueda, quick-access, compartir (permisos, link), colaboración
  (polling 5s, presencia, merge a 3 bandas con tombstones), autosave.
- **Timeline**: historial de versiones con autor/avatar, agrupado por mes/cluster,
  preview con zoom, restaurar versión, entrada "Original".
- **Calendario**: integración Google Calendar, modal mensual, eventos CRUD,
  "Add calendar to pages", side-panels por página (agenda/diario).
- **Plataforma**: modo app Android (WebView wrapper, `android app/` empaqueta el HTML
  con `html_to_apk_builder.py`), temas, toolbar arrastrable con snap, responsive.
- Fuera de alcance: `scanner/` es una app aparte; no se toca.

---

## 3. Diagnóstico detallado

### 3.1 Canvas / fluidez de escritura y movimiento

Lo que ya está bien (conservar el diseño):
- `getCoalescedEvents()` en pointermove ([index.html:10623](index.html:10623)).
- Finalización incremental del trazo: segmentos viejos se pintan en el canvas de página
  y solo la "cola" (6 puntos) se repinta en un overlay (`LIVE_PREVIEW_POINTS`/
  `FINALIZE_STEP_POINTS`, [index.html:10956](index.html:10956)).
- Anclaje del trazo a `pageId` capturado en pointerdown para sobrevivir reorders
  remotos ([index.html:10739](index.html:10739)).

Causas concretas de jank:

| # | Causa | Evidencia |
|---|-------|-----------|
| C1 | El build del PDF de autosave corre en el hilo principal, con un sistema de "yield + pausa si hay interacción" que compite con el pointermove | `buildPdfBlobWithPdfLib` y su `yieldToUI` ([index.html:29933](index.html:29933)) |
| C2 | El "baking" de trazos antiguos está **desactivado por pérdida de datos** → páginas con cientos de trazos se repintan completas (todas las curvas) en cada `redrawPage`, p. ej. en cada pasada del borrador | `shouldBakeStrokes` devuelve `false` ([index.html:8283](index.html:8283)) |
| C3 | Borrador: hit-test lineal contra todos los puntos de todos los trazos por cada posición del puntero + redraw completo de página | `eraseStrokeAtPoint`/`eraseAreaAtPoint` por posición ([index.html:10490](index.html:10490)) |
| C4 | `pointerleave` finaliza el trazo → trazos cortados al rozar el borde del canvas o al perder hover el stylus | [index.html:10804](index.html:10804) |
| C5 | Geometría del canvas cacheada en pointerdown y no invalidada si hay zoom/pan a mitad de trazo | `drawGeometry` ([index.html:10440](index.html:10440)) |
| C6 | Re-rasterización de canvases al cambiar zoom (`ensurePageCanvasResolution` + `scheduleCanvasResolutionSync` en cada `updateTransform`) en mitad del gesto | [index.html:20148](index.html:20148) |
| C7 | Unload de páginas genera previews con `toDataURL` síncrono en el hilo principal mientras se hace scroll | `unloadPageData` → `renderPagePreview` ([index.html:24326](index.html:24326)) |
| C8 | `MAX_ACTIVE_PAGES = 3` + IDB load/unload continuo al hacer scroll rápido = stutter y carreras (toda la maquinaria de `sessionToken`/`structureVersion`) | [index.html:7976](index.html:7976), `ensurePageDataLoaded` ([index.html:24181](index.html:24181)) |
| C9 | Sin `getPredictedEvents()` ni `desynchronized: true` en los contexts → latencia visible del trazo respecto al stylus | — |

### 3.2 Guardado

Hay **tres capas con tres formatos** y coordinación manual:

1. IndexedDB: registro por página (clave = índice), cola de stroke-ops, meta-payload.
2. localStorage: backup del meta + formatos legacy (múltiples claves antiguas).
3. Drive: PDF reconstruido con strokes/versiones/original embebidos en Keywords.

Bugs estructurales:

- **Claves por índice**: reordenar páginas (Manage Pages), restaurar una versión o
  recibir un merge remoto invalida las claves de IDB y de la cola de ops. El código
  lo mitiga limpiando el page-store completo (`clearPageStore` en restore,
  [index.html:16826](index.html:16826)) y marcando todo dirty — costoso y frágil.
- **Doble pista de "dirty"**: `pageDirty`/`pageDirtyMode` (local) vs `driveDirty`/
  `deferredDriveSave`/`driveSaveQueued`/`driveContentVersion` vs
  `driveUploadedContentVersion` (Drive). Estados acoplados por timers
  (`scheduleDeferredDriveSaveRetry` con delays mágicos 80/180/280/420/900/1500 ms,
  [index.html:25241](index.html:25241)) y un "watchdog de save pausado". Cuando un
  timer no dispara en el orden esperado → "no se guardó" o "se guardó dos veces".
- **Subida = archivo completo**: no hay guardado incremental en Drive; un documento
  con un PDF de 5 MB se resube entero tras cada ráfaga de trazos. En conexiones
  móviles esto se aborta/encola → más estados intermedios.
- **El merge colaborativo y el autosave comparten el mismo recurso** (el archivo PDF
  entero), de ahí `waitForRemoteSaveQuietBeforeUpload`, `mergeLatestRemoteBeforeDriveUpload`,
  `deferredRemotePullPending`… cada combinación de carrera necesita su parche.

### 3.3 Timeline

- Cada snapshot guarda **el documento completo** (`pages: pagesJsonArray`,
  [index.html:29711](index.html:29711)), hasta `VERSION_HISTORY_MAX = 50` entradas,
  todo comprimido **dentro del campo Keywords del PDF**. Con documentos reales el
  string crece a megabytes: la compresión/decompresión se hace en cada save/open,
  los límites prácticos de metadatos se superan y el decode falla → `catch` →
  `null` → **historial perdido en silencio** ([index.html:29745](index.html:29745)).
- El historial solo se captura tras un upload a Drive exitoso → documentos locales
  no tienen timeline, y los fallos de upload agujerean la línea temporal.
- Restaurar una versión: reconstruye páginas, limpia todo el page-store de IDB,
  re-marca todo dirty, fuerza upload, añade tombstones de todo lo no restaurado
  ([index.html:16868](index.html:16868)) — cinco subsistemas tocándose a la vez.

---

## 4. Nueva arquitectura

### 4.1 Principios

1. **Identidad sobre posición**: `docId`, `pageId`, `strokeId` (ULIDs). Los índices
   solo existen en la capa de presentación.
2. **Un solo documento de verdad**: el modelo en memoria. Persistencia y render son
   *proyecciones* que se derivan de él mediante eventos, nunca al revés.
3. **El PDF es un artefacto de exportación, no la base de datos.** El documento de
   verdad en Drive pasa a ser un **sidecar JSON** (pequeño, rápido de subir); el PDF
   se regenera en segundo plano y con menos frecuencia.
4. **El hilo principal solo dibuja y atiende input.** Serialización, compresión,
   build de PDF y previews van a un **Web Worker** (OffscreenCanvas).
5. **Máquinas de estado explícitas** para guardado y sync (estados nombrados,
   transiciones puras, testeables) en lugar de 20 booleanos + timers.
6. **Código en módulos TypeScript** con build a un único `index.html` (requisito del
   wrapper Android), tests unitarios del core y E2E de los flujos críticos.

### 4.2 Stack

- **TypeScript + Vite** + `vite-plugin-singlefile` → `dist/index.html` único
  (mismo artefacto que hoy consume `android app/html_to_apk_builder.py`).
- **Sin framework UI**: la app es 90 % canvas + paneles propios; se mantiene DOM
  vanilla con un helper mínimo de componentes/templating. (Migrar a un framework
  añadiría riesgo sin resolver ninguno de los tres problemas.)
- **Vitest** para el core (modelo, oplog, merge, máquinas de estado) y
  **Playwright** para flujos E2E (dibujar→recargar→persistencia, restore, share).
- Librerías que se conservan: pdf.js, pdf-lib, jsPDF (solo export), Google Identity.

### 4.3 Estructura de módulos

```
src/
├─ main.ts                    # boot, registro de vistas
├─ core/                      # SIN dependencias de DOM — 100% testeable
│  ├─ model.ts                # Doc, Page, Stroke, Img, tipos + invariantes
│  ├─ ids.ts                  # ULIDs (docId/pageId/strokeId)
│  ├─ ops.ts                  # Op = addStroke|eraseStrokes|transform|addPage|movePage|…
│  ├─ store.ts                # DocStore: apply(op) → evento; suscripciones
│  ├─ history.ts              # undo/redo por ops inversas (IDs, no índices)
│  ├─ merge.ts                # merge 3-bandas por strokeId + tombstones (port del actual)
│  └─ versions.ts             # checkpoints + deltas del timeline
├─ render/
│  ├─ pageRenderer.ts         # 3 capas por página: bg / tinta estática / tinta viva
│  ├─ inkStatic.ts            # raster incremental: trazo terminado se pinta 1 vez
│  ├─ inkLive.ts              # cola del trazo en curso (port del diseño actual)
│  ├─ strokesIndex.ts         # índice espacial (grid) para borrador/lasso O(1)
│  ├─ templates.ts            # plantillas default/agenda/diary (port)
│  └─ previews.worker.ts      # previews y rasters en OffscreenCanvas
├─ input/
│  ├─ pointerPipeline.ts      # captura, coalesced+predicted, palm rejection
│  ├─ gestures.ts             # pan/pinch/inercia/overscroll-crear-página
│  └─ tools/                  # pen, highlighter, eraserStroke, eraserArea, lasso
├─ viewport/
│  ├─ camera.ts               # zoom/pan puros (matemática sin DOM)
│  ├─ transformApplier.ts     # CSS transform + re-raster SOLO al asentarse el zoom
│  └─ pageWindow.ts           # qué páginas están montadas/cargadas
├─ persist/
│  ├─ idb.ts                  # esquema v2 (ver 4.5)
│  ├─ oplog.ts                # append-only de ops, flush por lotes
│  ├─ snapshots.ts            # compactación oplog→snapshot (en worker)
│  └─ migrate.ts              # lectura de TODOS los formatos viejos (una vez)
├─ sync/
│  ├─ driveClient.ts          # fetch + auth PKCE + refresh (port de driveFetch)
│  ├─ syncMachine.ts          # máquina de estados: idle→dirty→uploading→…
│  ├─ sidecar.ts              # formato JSON del documento en Drive
│  ├─ pdfArtifact.worker.ts   # build del PDF (pdf-lib) fuera del hilo principal
│  ├─ collab.ts               # polling, presencia, pull/merge (port simplificado)
│  └─ shares.ts               # permisos, links (port)
├─ ui/
│  ├─ editor/                 # toolbar, menús de pluma/borrador, status bar
│  ├─ driveHome/              # grid de archivos, carpetas, papelera, búsqueda
│  ├─ timeline/               # panel de versiones (mismo diseño visual)
│  ├─ managePages/
│  ├─ calendar/               # modal + side-panels + add-to-pages
│  └─ modals/
└─ styles/                    # CSS dividido por área (hoy ~6.200 líneas en uno)
```

Regla de dependencias: `ui → (render|input|viewport|sync) → core`; `core` no importa
de nadie. `persist` y `sync` solo hablan con `core` a través de ops/eventos.

### 4.4 Modelo de datos

```ts
type Stroke = { id: StrokeId; tool: Tool; color: string; width: number;
                points: Float32Array /* x,y,p empaquetados */ ; bbox: Box };
type Page   = { id: PageId; size: {w:number;h:number};
                background: {kind:'template'|'pdf'|'custom'; ...};
                strokes: Map<StrokeId, Stroke>; images: Map<ImgId, Img>;
                tombstones: Set<StrokeId> };
type Doc    = { id: DocId; rev: number; pageOrder: PageId[];
                pages: Map<PageId, Page>; meta: {...} };
```

- `points` como `Float32Array` empaquetado: −60 % memoria y serialización mucho
  más rápida que arrays de objetos `{x,y,p}`.
- `bbox` precalculada por trazo + índice espacial por celda → borrador y lasso dejan
  de ser O(total de puntos).
- Toda mutación pasa por `store.apply(op)`. La op se anota en el oplog, dispara el
  evento de render y alimenta undo/redo y sync. **Un solo camino de escritura.**

### 4.5 Persistencia local (IndexedDB v2)

```
db: inhouse-notes-v2
├─ docs        { docId → meta, pageOrder, rev, savedAt }
├─ pages       { [docId+pageId] → página serializada }     ← clave por IDENTIDAD
├─ oplog       { [docId+seq] → op }                        ← recovery tras crash
└─ versions    { [docId+versionId] → checkpoint/delta }
```

- Escritura: cada op se apunta al oplog en ≤1 microtask (igual que la cola actual,
  que funciona bien); un compactador en worker consolida oplog→`pages` cuando hay
  ≥N ops o al perder foco. **Reordenar páginas ya no reescribe nada** (solo
  `pageOrder` en `docs`).
- localStorage queda solo para sesión/tokens/preferencias — no para documento
  (elimina la tercera copia y sus carreras).
- Al abrir: `pages` + replay del oplog pendiente. Mismo nivel de crash-safety que
  hoy, sin los parches de `sessionToken`.

### 4.6 Sincronización con Drive (el cambio más importante)

**Formato nuevo en Drive — por documento:**

| Archivo | Contenido | Frecuencia de subida |
|---|---|---|
| `<nombre>.pdf` | Artefacto visible/compartible (igual que hoy: overlay pdf-lib + texto original intacto) | Lazy: idle ≥15 s, al cerrar el doc, o antes de compartir/exportar |
| `<nombre>.ihn.json` (oculto vía `appProperties` o carpeta `.inhouse`) | Documento de verdad: ops/snapshot comprimidos + historial de versiones + puntero al PDF | Cada ráfaga de edición (debounce ~2 s) — son KB, no MB |

- **Autosave pasa de "reconstruir y subir un PDF" a "subir unos KB de JSON"** →
  desaparece el 90 % de la maquinaria de pausas/watchdogs y el jank de C1.
- El build del PDF se hace **en worker** y no bloquea ni input ni saves.
- Colaboración: el polling y el merge operan sobre el sidecar (cheap ETag/revision
  check). La presencia se mantiene igual. El merge a 3 bandas actual por strokeId +
  tombstones se porta casi tal cual a `core/merge.ts` (es de lo mejor del código).
- **Compatibilidad**: al abrir un PDF viejo sin sidecar, `persist/migrate.ts` extrae
  `STROKES_Z` + historial de Keywords + adjunto original (las funciones de parseo
  actuales se portan) y crea el sidecar. Los PDFs siguen siendo legibles por
  cualquier visor, como hoy.
- Máquina de estados única de sync con estados nombrados:
  `idle → localDirty → debouncing → uploadingSidecar → idle` y carril paralelo
  `pdfStale → buildingPdf(worker) → uploadingPdf`, con `conflict → pulling → merging`
  como interrupción. Todos los booleanos sueltos actuales desaparecen.

### 4.7 Timeline v2

- Versiones = **checkpoints + deltas** en `versions` (IDB) y espejadas en el sidecar:
  un checkpoint completo cada K entradas, el resto solo ops entre versiones.
  Cap por **tamaño total** (p. ej. 2 MB), no por nº de entradas.
- Se capturan al consolidar el oplog (también offline) — ya no dependen de que un
  upload tenga éxito.
- Restaurar = `store.apply(restoreOp(checkpoint))`: una op más, que genera sus
  propios tombstones, viaja por sync normal y es undoable. Sin `clearPageStore`,
  sin marcar todo dirty, sin forzar uploads especiales.
- UI: se conserva el diseño actual (agrupación mes/cluster, avatares, preview con
  zoom, entrada "Original" derivada del PDF limpio adjunto).

### 4.8 Render de tinta (fluidez)

- **3 capas por página**: (1) fondo (template/PDF) raster cacheado; (2) tinta
  estática — los trazos terminados se pintan una sola vez, nunca se repintan en
  bloque; (3) tinta viva — overlay con la cola del trazo en curso (se porta el
  esquema actual de finalización incremental, que es correcto).
- Contexts con `{ desynchronized: true }` donde el dispositivo lo soporte +
  `getPredictedEvents()` con corrección al confirmar puntos → latencia de stylus
  visiblemente menor (C9).
- **Borrado sin redraw completo**: el índice espacial da los trazos candidatos; se
  repinta solo el dirty-rect afectado de la capa estática (C2/C3). Con `bbox` por
  trazo, repintar un rect toca una fracción de los trazos.
- **Zoom**: durante el gesto solo CSS transform (como hoy); la re-rasterización a la
  nueva resolución ocurre una vez asentado el zoom (debounce en rAF) y en worker
  para las páginas no activas (C6).
- Previews de unload en `previews.worker.ts` con OffscreenCanvas (C7).
- pointerup/pointercancel finalizan el trazo; `pointerleave` NO (C4). La geometría
  del trazo se invalida si cambia el transform a mitad (C5).
- El "baking" deja de ser necesario: la capa de tinta estática ya es el baked raster,
  pero el vector sigue siendo la verdad (no hay riesgo de pérdida como el que obligó
  a desactivar `bakeOldStrokes`).

### 4.9 Undo/redo v2

- Cada op define su inversa (`addStroke ↔ removeStroke(id)`, transform guarda
  matrices antes/después por id). Nada de `page.strokes.pop()` ni índices que un
  merge remoto puede desplazar.
- Cap por memoria estimada (se conserva la idea de `MAX_HISTORY_MEMORY`).

### 4.10 Migración de datos del usuario

`persist/migrate.ts`, se ejecuta una vez por origen de datos:
1. localStorage legacy (todas las `LEGACY_DATA_KEYS`) → modelo v2.
2. IDB v1 (page records por índice + stroke-ops + meta) → modelo v2.
3. PDFs de Drive sin sidecar → extraer Keywords/adjunto al abrir (permanente,
   porque siempre habrá PDFs viejos por ahí).
Tests de migración con fixtures reales exportadas de la app actual **antes** de
empezar (ver Fase 0).

---

## 5. Plan de ejecución por fases

Cada fase termina con la app usable y con sus tests en verde. Orden pensado para
des-riesgar primero el núcleo (modelo+persistencia) y dejar la UI portada al final.

| Fase | Contenido | Criterio de aceptación |
|---|---|---|
| **0. Salvaguardas** | Congelar `index.html` actual como `legacy/index.html`. Exportar 3–4 documentos reales (con PDF, con agenda, con colaboración) como fixtures. Scaffold Vite+TS+singlefile con CI de build. | `dist/index.html` único se genera y abre en blanco; fixtures guardadas en `test/fixtures/`. |
| **1. Core** | `core/` completo: modelo, ops, store, history, merge (port), versions. | Vitest: 100 % de ops con inversa correcta; merge reproduce los casos del código actual (tombstones, reorder). |
| **2. Persistencia** | IDB v2 + oplog + compactador worker + `migrate.ts` (localStorage/IDB v1). | E2E: dibujar → kill tab → reabrir → nada perdido. Fixtures v1 migran sin pérdida. |
| **3. Lienzo** | Render 3 capas, input pipeline, herramientas (pen/highlighter/erasers/lasso), viewport (pan/zoom/inercia/overscroll-crear-página), lifecycle de páginas. | Escritura fluida con 500+ trazos/página (perf trace sin frames >16 ms por redraw); borrador sin redraws completos; trazos no se cortan en bordes. |
| **4. Sync Drive** | driveClient (port PKCE), syncMachine, sidecar, pdfArtifact en worker, migración de PDFs viejos al abrir. | Editar en dos dispositivos sin pisarse; autosave de ráfaga <200 ms percibidos; PDF en Drive se refresca en idle. |
| **5. Timeline** | versions v2 + panel UI (mismo diseño). | Restaurar/revert al original funcionan offline y online; historial sobrevive a 200 saves. |
| **6. UI restante** | Drive home, share/colaboración+presencia, calendario+side-panels, manage pages, welcome, temas, toolbar, modales, modo Android. | Checklist de paridad (§6) completa. |
| **7. Corte** | QA con los documentos reales, build APK con el nuevo HTML, retirar legacy. | Una semana de uso propio sin regresiones; `android app/` apunta al nuevo dist. |

Estimación relativa de esfuerzo: F1–F3 ≈ 50 %, F4 ≈ 20 %, F5 ≈ 10 %, F6 ≈ 20 %.
F1–F5 no requieren decisiones de producto; F6 es port mecánico de UI.

---

## 6. Checklist de paridad funcional (verificar en Fase 7)

- [ ] Pluma con presión / subrayador / borrador-trazo / borrador-área / lasso
- [ ] Mover/escalar selección (trazos + imágenes), borrar selección
- [ ] Crear página por overscroll; indicador de "añadir página"
- [ ] Manage Pages: reorden drag&drop, borrar, añadir; reset de documento
- [ ] Plantillas default/agenda/diary + portada + imagen de fondo custom
- [ ] Importar PDF (archivo y URL), anotar, exportar estándar/HQ, revertir a original
- [ ] Persistencia local sin cuenta; recuperación tras cierre forzado
- [ ] Sign-in Google, Drive home (grid, carpetas, papelera, búsqueda, quick access)
- [ ] Autosave Drive + indicador de estado de guardado
- [ ] Compartir: invitaciones, roles, link general; abrir link compartido (resourceKey)
- [ ] Colaboración: presencia (avatares), merge sin pérdida, edición simultánea
- [ ] Timeline: lista agrupada, preview con zoom, restaurar, "Original", milestones
- [ ] Calendario Google: modal mes, eventos CRUD, add-to-pages, side-panels día/semana,
      "ir a hoy" (`scrollToTodayCalendarPage`)
- [ ] Documentos viejos (PDF con STROKES_Z) abren y migran sin pérdida
- [ ] Modo Android app (flag/UA), build APK funciona con el nuevo HTML
- [ ] Temas, toolbar arrastrable con snap/escala, atajos, responsive móvil

---

## 7. Riesgos y decisiones

| Riesgo / decisión | Postura recomendada |
|---|---|
| Sidecar JSON vs todo-en-PDF | Sidecar (§4.6). Único cambio visible: el PDF de Drive puede ir unos segundos por detrás del estado real. Si se quiere evitar el segundo archivo, alternativa: mantener strokes en Keywords pero subir **solo metadatos** vía `files.update` sin media — peor opción, sigue atando el historial al PDF. |
| Compatibilidad con docs antiguos | El lector de formato viejo es permanente (no solo migración one-shot) porque hay PDFs viejos compartidos. |
| Reescribir merge colaborativo | NO reescribir la lógica: portar `§4.21c` casi literal a `core/merge.ts` con tests; es la parte más probada del código actual. |
| Single-file para Android | Resuelto con `vite-plugin-singlefile`; verificar en Fase 0 que el APK builder acepta el output (inline de workers como blobs). |
| Workers dentro de un single-file | Los workers se inlinean como Blob URLs (Vite lo soporta con `?worker&inline`). Verificar en WebView Android en Fase 0. |
| Alcance de F6 (mucha UI) | Es port mecánico: el HTML/CSS actual se puede trocear y reutilizar casi tal cual. |

---

## Cómo empezar la próxima sesión

1. Leer este documento.
2. Ejecutar Fase 0 completa (salvaguardas + fixtures + scaffold).
3. Empezar Fase 1 (`core/model.ts`, `core/ops.ts`, `core/store.ts`) con Vitest.

Referencias rápidas al código actual (para portar):
- Dibujo en vivo: [index.html:10418](index.html:10418)–11040 · Render: 16396–17600
- Viewport/gestos: 19228–20180 · Lifecycle páginas: 9485–10417 y 24181–24370
- IDB/oplog actual: 23901–25230 · Drive save: 25230–25990 · Merge colab: 26920–27885
- Timeline: 17598–18290 y 29677–29980 · Undo: 27885–28520 · Constantes: 7699–8542
