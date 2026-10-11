# Inhouse services updater

Actualizador común de Inhouse YouTube, Instagram, Notes, Photos y Read. Una interfaz y una implementación de descarga, verificación e instalación para las cinco apps.

**A: cambios recientes. B: versiones revisadas.** Todo cambio nuevo se publica en A. B solo se publica por petición expresa del usuario; no se promociona automáticamente al pasar pruebas.

El popup muestra versión instalada, nueva versión, versiones intermedias y hasta tres cambios muy breves. Incluye **Actualizar**, **Recordármelo más tarde** (14 días) y **Cambiar canal**. Los ajustes permiten elegir A/B y comprobar manualmente aunque el aviso esté pospuesto. Ninguna actualización bloquea el uso de la app.

## Código compartido

- `web/`: comprobador, selección de canal, recordatorio, popup y ajustes. El mismo código se incluye localmente en todas las aplicaciones; no se carga JavaScript remoto.
- `android/`: host de Android y el instalador procedente de Inhouse Photos. Descarga reanudable, SHA-256, paquete, versión y certificado.
- `tools/sync.py`: copia verificada y lock de revisión para los cinco proyectos.
- `tools/catalog.py`: prepara publicaciones A y promociones B explícitas con historial corto.

Los conectores de las apps solo aportan identidad, versión y enlace a ajustes. Cualquier cambio del actualizador se hace aquí primero y se sincroniza en las cinco apps. Las instrucciones de cada repositorio repiten esta regla.

[Contrato y publicación](docs/PROTOCOL.md) · [Procedencia](docs/PROVENANCE.md) · [Instrucciones para IA](AGENTS.md)

- **B is the default update channel.** A requires an English development warning, explicit acknowledgement and slide confirmation. Legacy implicit A preferences migrate to B; choosing B clears A consent. This does not authorize publishing or promoting a B release.
