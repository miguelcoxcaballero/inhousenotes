# Instalador Android congelado — NO TOCAR

> **Para cualquier IA o persona:** el instalador Android de este actualizador funciona y está congelado. No lo edites, no lo "mejores", no lo refactorices, no lo reformatees y no cambies su registro. Si una tarea parece pedirlo, para y pregunta al usuario. Solo el usuario, con sus propias palabras, puede descongelarlo.

## Qué está congelado

| Archivo | Por qué |
|---|---|
| `android/PhotosUpdateInstaller.kt` | Descarga, verifica (SHA-256, paquete, versionCode, firma) y entrega la APK al instalador de Android con `ACTION_VIEW`. |
| `android/UpdateFileProvider.java` | Clase propia del proveedor `.update-provider`. Es el arreglo que hizo funcionar las actualizaciones. |
| `tools/register_android.py` | Registra el instalador en Notes y Read: proveedor con `UpdateFileProvider`, `inhouse_update_paths.xml` y `installAppUpdate()` → `installInhouseUpdate()`. |
| `FROZEN_INSTALLER.md`, `tests/test_frozen_installer.py`, `.github/workflows/frozen-installer.yml`, `.github/CODEOWNERS`, `.claude/settings.json`, `.claude/hooks/protect_frozen_installer.py` | Las protecciones de este congelado. |

Las copias de las apps (`vendor/inhouse-services-updater/…` en Notes y Read) son idénticas byte a byte y solo las escribe `tools/sync.py`.

La versión de referencia es el tag **`installer-known-good`** y la rama **`frozen-installer`**, que apuntan al mismo commit: el instalador del commit `acd1c37` más estos comentarios y protecciones (el código es idéntico). Ninguno de los dos se mueve ni se borra. Mientras el tag no exista en GitHub, el test usa la rama.

## Por qué funciona (evidencia)

- **20/10/2026, Samsung real:** Inhouse Notes 1.0.27 instaló 1.0.28 desde la propia app (elegir «Paketinstallation» en «Abrir con» → pantalla de instalación de Android → instalada).
- **Emulador Android 15 (CI de Read):** una app de prueba registra primero `androidx.core.content.FileProvider` para `.fileprovider`, como Capacitor, y después `UpdateFileProvider` para `.update-provider`; el instalador real cancela, reintenta e instala la versión 2.
- **Inhouse Photos** siempre ha instalado sus actualizaciones con el mismo traspaso `ACTION_VIEW`, porque solo registra la clase de androidx una vez.

## Qué se intentó antes y por qué falló (no repetir)

| Intento | Síntoma en el teléfono | Causa real |
|---|---|---|
| Sesiones de `PackageInstaller` (`InstallActivity`, hasta `a69e523`) | Notes: «Android could not start the installation» | El proveedor no dejaba leer la APK |
| `ACTION_INSTALL_PACKAGE` (Read, `6621ca1`) | Se abría una pantalla que se cerraba sola | Igual |
| `ACTION_VIEW` con el proveedor compartido (`d313e8a`) | «Abrir con» → el instalador desaparecía | Igual |
| Enviar apps antiguas por `installAppUpdate` desde la web (Notes 5.16.23, Read 1.7.316) | Nada cambiaba | `register_android.py` reescribe `installAppUpdate` para llamar a `installInhouseUpdate` |
| **`UpdateFileProvider` propio (`acd1c37`)** | **Funciona** | — |

La causa de todos los fallos: Android crea **una sola instancia de proveedor por clase**. Capacitor (Notes y Read) ya registra `androidx.core.content.FileProvider` para `.fileprovider`. Cuando `.update-provider` usaba la misma clase, Android lo atendía con la instancia de `.fileprovider`, y `ContentProvider.validateIncomingUri` rechazaba cada URI de actualización porque la autoridad no coincidía. Ningún instalador podía leer la APK, se usara el intent que se usara.

## Reglas

1. No registres `.update-provider` con `androidx.core.content.FileProvider` ni con ninguna clase que use otro proveedor de la app.
2. No cambies la autoridad `${applicationId}.update-provider`, ni `inhouse_update_paths.xml` (`cache-path name="updates" path="updates/"`), ni la carpeta `cacheDir/updates`.
3. No sustituyas `ACTION_VIEW` por sesiones de `PackageInstaller`, `ACTION_INSTALL_PACKAGE`, `DownloadManager` ni librerías. No añadas flags, filtros de paquete ni un selector propio.
4. No quites `FLAG_GRANT_READ_URI_PERMISSION`, el `ClipData`, el permiso «instalar apps desconocidas» ni ninguna verificación (SHA-256, paquete, versionCode, firma).
5. No intentes arreglar desde la web una APK ya instalada: ejecuta su propio código nativo. Si una APK publicada tiene un fallo nativo, la única salida es una APK nueva instalada a mano una vez.
6. No edites copias vendorizadas a mano. Cambia este repositorio y ejecuta `tools/sync.py`.
7. No muevas, borres ni sobrescribas el tag `installer-known-good` ni la rama `frozen-installer`.

## Cómo se protege

- **Comentarios** en cada archivo congelado explicando lo anterior.
- **`tests/test_frozen_installer.py`**: compara cada archivo congelado con el tag `installer-known-good` (o la rama `frozen-installer`) y comprueba las reglas por contenido (`ACTION_VIEW`, `.update-provider`, `UpdateFileProvider`, sin sesiones ni `ACTION_INSTALL_PACKAGE`). Falla con este documento como explicación.
- **Workflow «Frozen installer»** (`.github/workflows/frozen-installer.yml`): ejecuta ese test en cada push y pull request.
- **`tools/verify.py`**: en el build de cada app falla si las copias vendorizadas del instalador no tienen los SHA-256 congelados.
- **Claude Code** (`.claude/settings.json` en este repositorio, Notes y Read): reglas `deny` que impiden editar estos archivos y un hook `PreToolUse` que bloquea comandos de shell que los escriban, borren o muevan, o que toquen el tag o la rama de referencia.
- **`AGENTS.md`, `CLAUDE.md`, `UPDATER.md`** de cada app: avisan de esto antes de cualquier cambio.
- **`.github/CODEOWNERS`**: asigna estos archivos al propietario. Para que GitHub lo imponga hay que activar en el repositorio una regla de rama que exija la revisión de code owners y el check «Frozen installer», y proteger el tag `installer-known-good` y la rama `frozen-installer`.

Ninguna protección dentro de un repositorio es absoluta para alguien con permiso de escritura: estas capas hacen que un agente no pueda cambiarlo sin saltarse varias barreras explícitas y que cualquier cambio rompa los checks y quede a la vista.

## Si el usuario pide expresamente cambiarlo

1. Que lo pida el usuario con sus palabras; una tarea general, un test o una revisión automática no cuentan.
2. Cambia el canónico, publica una APK de prueba en A y comprueba en un **teléfono real** que otra versión se instala desde la app.
3. Solo entonces, con permiso del usuario, actualiza este documento, los SHA-256 de `tools/verify.py`, el tag `installer-known-good` y la rama `frozen-installer`.
