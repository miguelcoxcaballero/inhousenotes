# Procedencia

El flujo de comprobación, popup y progreso parte de Inhouse Read y de su integración en YouTube/Instagram. Se centraliza y amplía aquí para canales A/B, aplazamiento opcional y cambios breves. Ya no se exige identidad de CSS con la versión antigua: el usuario pidió pulir y unificar la interfaz.

`android/PhotosUpdateInstaller.kt` conserva el instalador Kotlin de Inhouse Photos usado en YouTube 0.1.5: descargas parciales/reanudación, redirecciones, SHA-256, comprobación de paquete, versión, firma y FileProvider. Se adapta la conexión al host; no se crea un segundo descargador. La fuente histórica está documentada en `ig-calma/vendor/UPSTREAM.md` y `youtube-calma/vendor/`.

Apps consumidoras: miguelcoxcaballero/youtube-calma, ig-calma, inhousenotes, Inhouse-Photos e inhouse-read.
