# Estado de la reconstrucción — actualizado 2026-06-12

**IMPORTANTE — qué es cada cosa ahora:**
- `dist/index.html` = **la app ORIGINAL completa** (misma UI y funciones, byte
  a byte) **+ 5 bugfixes/optimizaciones quirúrgicos**: (1) `pointerleave` ya
  no corta trazos; (2) el historial del timeline se acota por tamaño
  comprimido al embeberlo en Keywords (antes crecía a MB y se perdía en
  silencio al fallar el decode); (3) undo ampliado 10→30 pasos (acotado por
  memoria como antes); (4) puntos predichos del stylus en la cola de
  previsualización (menor latencia percibida; el trazo confirmado no cambia);
  (5) `DRIVE_SYNC_MAX_DELAY` 3 s→12 s — el build forzado del PDF ya no
  arranca en pleno trazo (causa principal de tirones; el guardado local no
  se ve afectado). Verificado: arranca con la welcome original, DOM completo
  (welcome/toolbar/drive-home) y sin errores de consola; sintaxis de los
  bloques script idéntica en validez a la original. Es el candidato a
  sustituir `../index.html` y el HTML del APK cuando el usuario lo pruebe.
- `src/` = la reimplementación desde cero (proyecto a largo plazo). Motor
  sólido y testeado, pero **NO tiene paridad de UI/funciones** con la
  original; no presentarla como sustituta. Para regenerar su build:
  `npm run build` (nota: sobrescribe `dist/`; volver a aplicar este parche o
  guardar el build en otra ruta).



**Dónde está todo:** esta carpeta (`rebuild-v2/`) contiene el proyecto completo
de la reconstrucción (fuente TS + build). La app original SIGUE siendo
`../index.html` (intacta) y es la que empaqueta `../android app/` — la
reconstrucción aún **no tiene paridad de UI** con la original.

## Verificado en navegador (esta sesión)

- Crear documento y abrir por `?docId=`
- Dibujar con pluma (capas bg/ink/live correctas)
- Persistencia en IndexedDB + recarga sin pérdida
- Undo/redo exactos (píxeles idénticos al estado previo)
- Borrador por trazo
- Plantilla original (marco + cuadrícula de puntos) renderizada
- 50/50 tests, typecheck y build single-file en verde

## Bugs encontrados y corregidos en esta sesión

Introducidos por otras herramientas al implementar las fases 3–6:

1. `Editor.buildToolbar()` llamaba a `setTool()` antes de asignar
   `this.toolbar` → TypeError → **el editor entero no se montaba** (sin
   toolbar, sin dibujo). Corregido en `src/editor.ts`.
2. `pointerPipeline` usaba `e.screenX/screenY` (coordenadas de pantalla
   física, incluyen marco de ventana) para un hit-test que espera coordenadas
   relativas al viewport → reescrito con `clientX - rect`, pointer capture, y
   sin finalizar trazos en `pointerleave` (bug C4 del plan).
3. `gestures` hacía pan con CUALQUIER puntero — incluido el lápiz: el lienzo
   se desplazaba al dibujar → reescrito: gestos solo táctiles; 1 dedo pan,
   2 dedos pinch, rueda scroll / Ctrl+rueda zoom; rechazo de palma vía el
   nuevo `src/input/router.ts`.
4. Faltaba TODO el CSS estructural del renderer: las 3 capas de canvas se
   apilaban **verticalmente** y el contenedor escalaba desde el centro →
   inyectado `RENDERER_CSS` en `src/render/docRenderer.ts`.
5. Límites de zoom incoherentes (0.1–10 en gestos vs 0.5–3 en cámara) →
   `setCamera` acota siempre con `clampZoom`.
6. Overscroll-para-crear-página con la condición invertida (se disparaba al
   alejarse del final, no al sobrepasarlo) → corregido.
7. `../index.html` original tenía un byte espurio (`c`) antepuesto →
   restaurado desde la copia congelada.
8. `../android app/Inhouse Notes.html` había sido sustituido por el build sin
   paridad → restaurado a la app original.

## Añadido después (misma sesión): shell visual legacy

`src/ui/legacyTheme.ts` porta los tokens de diseño, fuentes, welcome (logo +
Sign in), home estilo drive-home (topbar, tarjetas, búsqueda) y la toolbar
flotante vertical original (activo en amarillo). `main.ts` reescrito con ese
shell. OAuth client ID del original embebido por defecto en `driveClient.ts`
(ya no se pide por input). Verificado en navegador: welcome/home/editor con
el aspecto original y dibujo funcionando. Las sobrescrituras `!important`
sobre los estilos inline del editor son deuda técnica marcada.

**Continuación:** `ui/modal.ts` retematizado con los tokens legacy → todos los
paneles (timeline, pages, share, calendar) heredan el diseño original.
Timeline verificado end-to-end en navegador: agrupación por mes, miniaturas,
restore correcto (vuelve exactamente a la versión elegida) y persistencia del
restore tras recargar.

**Developer mode (añadido a `dist/index.html`):** 10 toques en el logo de la
welcome → popup PROPIO "developer mode entered" (no alert nativo). Con él
activo, "Sign in" ofrece "Normal sign in" / "Dummy data". Dummy: sesión falsa
"developer <developer@dummy.local>" (perfil completo en el menú: Night mode,
Diary, Agenda, Sign out…), home sin documentos, y "New PDF" crea documentos
de prueba (default/agenda/diary) que viven SOLO en memoria — verificado 0
registros en IDB tras crear: se stubean scheduleSave/saveToStorage/
savePageToIndexedDb/scheduleLocalStorageBackup y ensureDriveToken (este
último falla al instante; sin el stub el overlay "Creating PDF..." se colgaba
esperando a Google). Estado solo en memoria: recargar la página SALE del
developer mode (verificado: tras reload, Sign in va directo al flujo normal).
Todo el script vive en un único bloque `<script>` al final del archivo.

## Pendiente (en orden)

1. **Fase 6 — paridad de UI** (lo grande): portar el shell visual legacy
   (welcome con sign-in, Drive home con grid/carpetas/papelera/búsqueda,
   chrome del editor con la toolbar original arrastrable, modales, timeline
   con el diseño original) sobre este motor. Referencia: `../index.html`,
   secciones §2 (CSS, ~6.200 líneas) y §3 (HTML del body) de su índice.
2. Verificar con credenciales reales: OAuth de Drive (el client ID debe ir
   embebido como en la original, no pedirse en un input), autosave del
   sidecar, colaboración, calendario de Google.
3. QA de migración con documentos reales del usuario (abrir PDFs viejos con
   `STROKES_Z`) y build del APK.

## Para retomar

```
cd rebuild-v2
npm install
npm run dev        # abre http://localhost:5173/app.html
npm test           # 50 tests
npm run build      # genera dist/index.html single-file
```
