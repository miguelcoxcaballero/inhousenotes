# Estado de la reconstruccion - 2026-07-17

## Artefactos

- `dist/index.html` es el build standalone actual del motor nuevo. Funciona sin
  conexion, no presupone una cuenta iniciada y contiene todo el JavaScript y CSS
  de la aplicacion en un unico archivo.
- `../index.html` y `../android app/Inhouse Notes.html` siguen siendo la version
  legacy. No se han sustituido durante este trabajo para no publicar en el APK
  un candidato sin probar primero con documentos y credenciales reales.
- `src/` es la fuente de verdad del motor nuevo.

## Implementado

- Persistencia incremental en IndexedDB: cada guardado reescribe solo las
  paginas modificadas y conserva los assets binarios por separado.
- Op-log append-only para recuperacion tras cierre o bloqueo, compactacion con
  Web Locks y vaciado al ocultar/cerrar la pagina.
- IDs estables para documentos, paginas, trazos y operaciones; reordenar paginas
  no cambia su identidad.
- Historial desacoplado del autosave: se captura por inactividad y por limite de
  tiempo, no en cada trazo.
- Sincronizacion Drive por sidecar schema 3, merge por IDs, tombstones y linaje
  de escrituras. Antes de escribir se relee el remoto para reducir
  sobreescrituras entre dispositivos.
- PDFs fuente guardados una sola vez como assets. En Drive se suben como archivos
  binarios separados y el sidecar solo conserva referencias.
- Upload reanudable para archivos grandes y reintentos de red.
- Importacion de anotaciones PDF estandar `/Ink` como trazos editables.
- Exportacion de los trazos de la app como `/Ink`, de modo que siguen siendo
  anotaciones editables al reimportarlos en Inhouse Notes y en lectores PDF que
  soporten ese estandar.
- Pantallas welcome, home y editor con el tema visual legacy; modo offline
  explicito, botones New notebook/New PDF operativos y modales propios.
- Tipografias Comfortaa y DM Sans embebidas en el standalone: el aspecto no
  depende de Google Fonts ni cambia al perder la conexion.
- Barra de estado local/Drive, timeline, paginas, compartir y calendario.

## Verificado

- 62 tests automatizados: core, merge, historial, versiones, migracion,
  persistencia incremental, recuperacion del op-log, sidecar, concurrencia Drive,
  assets PDF y roundtrip de `/Ink`.
- TypeScript sin errores y build single-file correcto.
- Auditoria de dependencias de produccion: 0 vulnerabilidades conocidas.
- Navegador desktop y movil: welcome, entrada offline, home, creacion de nota,
  dibujo, autosave local, recarga, barra de acciones y botones principales.
- PDF exportado renderizado externamente: la anotacion Ink aparece correctamente.

## Compatibilidad Samsung Notes

Los detalles, fuentes oficiales y limites estan en
`SAMSUNG_NOTES_COMPATIBILITY.md`.

## Pendiente antes del corte al APK

1. Probar varios PDFs y `.sdocx` reales del usuario. Un PDF aplanado no conserva
   la identidad de sus trazos y ninguna aplicacion puede reconstruirla de forma
   fiable; las anotaciones `/Ink` si son editables.
2. QA real de OAuth/Drive con dos cuentas y dos dispositivos, incluyendo
   conflictos y permisos de archivos PDF fuente.
3. Probar Calendar con credenciales reales.
4. Ejecutar el builder del APK, instalarlo en un Samsung y validar S Pen,
   suspension/reanudacion, importacion desde Samsung Notes y archivos grandes.
5. Solo despues de estas pruebas, sustituir los dos HTML legacy por
   `dist/index.html` y publicar la version.

## Comandos

```powershell
npm install
npm run dev
npm test
npm run typecheck
npm run build
```
