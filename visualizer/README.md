# OpenMirai Studio — visualizador local

Visualizador local de agentes OpenMirai. Abre cualquier config `.yaml` y ve el **paso a paso**: los nodos, su configuración, cómo conectan los edges, y un `Run` que recorre el grafo. 100% offline, sin build ni servidor — se abre con doble-click.

## Uso

```bash
# tras clonar el repo (o al crear/editar agentes), regenera el catálogo embebido:
node visualizer/build-catalog.js

# luego abre el visualizador:
open visualizer/mirai-app.html      # macOS  (o doble-click en el Finder)
```

## Cómo funciona

- **Catálogo por defecto:** el build embebe el YAML crudo de `agents/` (tus agentes) y `examples/` (los oficiales del engine) dentro de `mirai-app.html`, junto con el parser [`js-yaml`](https://github.com/nodeca/js-yaml). Por eso funciona offline: no lee el disco en tiempo real.
- **Abrir desde cualquier ruta:** el botón **“Abrir agente…”** deja cargar un `.yaml` de cualquier carpeta de tu computador; se parsea en el navegador y se asocia a la sesión del visualizador (por seguridad el navegador no expone la ruta absoluta de archivos abiertos así — solo el nombre).
- **Abrir ubicación:** en el detalle de cada agente del catálogo por defecto hay botones **Copiar ruta / Abrir carpeta / Abrir .yaml** (usan `file://`).

## Vistas

- **Inicio** — agentes abiertos recientemente (por fecha de archivo) y últimas ejecuciones.
- **Agentes** — la biblioteca completa, filtrable.
- **Detalle** — flujo del agente, su ruta en disco y sus ejecuciones.
- **Canvas** — el grafo (auto-layout por etapas) + inspector de cada nodo + toggle **YAML** (muestra el archivo real).

## Notas

- El grafo, las etapas (columnas) y el orden del `Run` se **derivan** del YAML: la categoría de cada nodo sale del `tool_type` (`ai/*`, `data/*`, `trigger/*`, `logic/*`, `output/*`), los puertos salen del `data_map` de los edges, y las columnas por el camino más largo desde el trigger.
- `mirai-app.html` es un solo archivo autocontenido (el `build-catalog.js` inyecta el bloque de datos entre el marcador `@BUILD_INJECT@` y el script principal). No editar ese bloque a mano.
- `js-yaml.min.js` está vendorizado en esta carpeta para que el build sea portable (no depende de un `node_modules` externo).
