# inhousenotes

[Android 1.0.16 — A y B](https://github.com/miguelcoxcaballero/inhousenotes/releases/tag/android-v1.0.16) · Web 6.2.4. El enlace de recuperación abre la página de la versión en GitHub para elegir la APK en Assets. Disponible en los dos canales para instalaciones antiguas.

## Inhouse Notes 6

La web 6.0.0 es la reescritura de 5.18.0 en Rust/WebAssembly, con el mismo aspecto y comportamiento ([cambios previstos](docs/v6/BEHAVIOUR-CHANGES.md)). La APK Android y su instalador congelado no cambian: la app carga la web publicada.

| Ruta | Contenido |
| --- | --- |
| `index.html`, `offline-v5.css`, `boot.js` | Marcado, estilos y arranque de 5.18.0 (tema, esqueleto, comprobación de versiones) |
| `host/*.js` | La capa JavaScript fina: cargador wasm, DOM, canvas, IndexedDB, red, pdf.js, WebRTC, plataforma |
| `service-worker.js` | Caché del shell `inhouse-shell-6.2.4` (misma URL que en 5.x) |
| `rust/` | Workspace cargo: `ihn-core`, `ihn-sync`, `ihn-app`, `ihn-wasm` |
| `wasm/ihn.wasm`, `offline-assets.js`, `native-assets.json` | Generados por `npm run build` y confirmados en `main` con la versión que publican |
| `tests/v6/` | e2e (Playwright), paridad de píxeles contra 5.18.0, mock de Google, rendimiento |

`npm ci`, después `npm run build` (presupuestos de tamaño), `npm run test:cargo`, `npm run test:shell`, `npm run test:e2e` y `npm run test:parity -- --quick`. Diseño: [docs/v6/ARCHITECTURE.md](docs/v6/ARCHITECTURE.md); pruebas: [docs/v6/TESTING.md](docs/v6/TESTING.md); política de versión: [AGENTS.md](AGENTS.md).

## Inhouse services updater

Android usa el [actualizador común](https://github.com/miguelcoxcaballero/inhouse-services-updater). En los ajustes de actualizaciones puedes elegir **A: cambios recientes; B: versiones revisadas** y comprobar manualmente. El aviso es opcional y «Recordármelo más tarde» lo aplaza 14 días. B solo se publica por petición expresa. A incluye todas las versiones A y B y ofrece la de mayor `versionCode`; B recibe solo versiones B. [Reglas de publicación y sincronización](UPDATER.md).
