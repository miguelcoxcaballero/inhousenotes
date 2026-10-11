# Contrato 1

`updates/A.json` y `updates/B.json` contienen `schemaVersion: 1`, `channel`, `latest` (release o null), y `releases` (historial descendente). Cada release contiene `version`, `versionCode`, `apkUrl`, `apkSha256`, `apkSizeBytes`, `changelog` (1–120 caracteres) y `releasedAt`. La versión semántica no incluye la letra; el popup la presenta al lado y Android compara versionCode globalmente.

Los archivos de canal se consultan con caché desactivada; la recuperación usa la API de contenidos de GitHub del mismo canal, nunca `/releases/latest`. B vacío se muestra explícitamente. Al cambiar de A a una B anterior se conserva la aplicación instalada hasta que B la alcance; no hay reinstalación ni pérdida de datos.

La política se persiste por aplicación. El recordatorio es un plazo de 14 días, no una alarma en segundo plano: se vuelve a ofrecer al abrir/enfocar la app o durante su comprobación periódica después del plazo. Una comprobación manual ignora el aplazamiento. Una versión nueva del mismo canal tampoco interrumpe los 14 días.

Las publicaciones anteriores a este protocolo pueden necesitar el manifiesto legado `android-update.json` para descubrir el primer APK compatible. Ese manifiesto mantiene `required:true` únicamente porque los comprobadores antiguos interpretan false como ocultar la actualización; el nuevo actualizador nunca lo usa para bloquear la app. La primera migración es A. No se crean versiones B sin instrucción expresa.

Bridge Android: `getAppVersion`, `getAppVersionCode`, `getUpdateState`, `saveUpdateState`, `installAppUpdate(url, sha256)`, `closeUpdater`. Las apps web usan localStorage si el shell antiguo no expone estado nativo. El host permite puente de instalación solo en sus recursos locales de confianza; Instagram/YouTube no reciben ese puente.
