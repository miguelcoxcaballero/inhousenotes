# Inhouse services updater

Actualizador común de Inhouse YouTube, Instagram, Notes, Photos y Read. Una interfaz y una implementación de descarga, verificación e instalación para las cinco apps.

**A recibe todas las versiones A y B. B recibe solo las versiones B.** En A se ofrece siempre la versión más nueva por `versionCode`: una B nueva sustituye a una A antigua, y una A posterior vuelve a ser la actualización disponible. Publicar una B anterior no desplaza una A más nueva.

Los cambios nuevos y las publicaciones automáticas van a A. B contiene versiones revisadas y solo se publica por petición expresa del usuario; puede ser una promoción de A o una publicación directa de un APK verificado. Pasar las pruebas no autoriza una publicación B.

El popup muestra versión instalada, nueva versión, versiones intermedias y hasta tres cambios muy breves. Incluye **Actualizar**, **Recordármelo más tarde** (14 días) y **Cambiar canal**. Los ajustes permiten elegir A/B y comprobar manualmente aunque el aviso esté pospuesto. Ninguna actualización bloquea el uso de la app.

## Código compartido

Los hosts web pueden abrir los mismos ajustes dentro de su menú de cuenta con
`updater.settings('', { container: elemento })`. La región integrada no usa
overlay, bloqueo de foco ni fondo oscurecido; conserva canales, consentimiento,
comprobación e instalación. Al cerrar su menú, el host debe llamar `updater.close()`
para invalidar comprobaciones pendientes y desmontar la región. Sin `container`,
los hosts existentes conservan el diálogo habitual.

- `web/`: comprobador, selección de canal, recordatorio, popup y ajustes. El mismo código se incluye localmente en todas las aplicaciones; no se carga JavaScript remoto.
- `android/`: host de Android y el instalador procedente de Inhouse Photos. Descarga reanudable, SHA-256, paquete, versión y certificado.
- `tools/sync.py`: copia verificada y lock de revisión para los cinco proyectos.
- `tools/catalog.py`: prepara publicaciones A y publicaciones/promociones B explícitas con historial corto; incluye automáticamente cada B en A conservando la versión más nueva.

Los conectores de las apps solo aportan identidad, versión y enlace a ajustes. Cualquier cambio del actualizador se hace aquí primero y se sincroniza en las cinco apps. Las instrucciones de cada repositorio repiten esta regla.

La inclusión de B en A se hace al publicar los catálogos, por lo que también funciona con los clientes ya instalados. Cada catálogo apunta a APK públicos verificados e inmutables; el canal B nunca recurre a A ni a la última release genérica de GitHub.

[Contrato y publicación](docs/PROTOCOL.md) · [Procedencia](docs/PROVENANCE.md) · [Instrucciones para IA](AGENTS.md)

- **B is the default update channel.** A requires an English development warning, explicit acknowledgement and slide confirmation. Legacy implicit A preferences migrate to B; choosing B clears A consent. This does not authorize publishing or promoting a B release.
