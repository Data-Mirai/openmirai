# OpenMirai en contenedores — prototipo

Empaqueta el motor en una imagen, lo levanta como servicio HTTP y lo somete a
una suite e2e que ejercita capacidades distintas del runtime. El objetivo no es
un despliegue productivo: es **medir qué tan lista está la v0.6.0 para correr en
la nube** y dejar el diagnóstico por escrito.

Resultado de la corrida de referencia: **18 verificaciones en verde, 2 brechas
del motor** (ver [Brechas para producción](#brechas-para-producción)).

---

## Arranque rápido

```bash
cp .env.example .env                 # ajustar MIRAI_API_KEY si se quiere
docker compose up -d --build         # construye y levanta el motor
docker compose run --rm smoke        # corre la suite e2e contra el motor
```

El motor queda en `http://localhost:4321` (adentro escucha en 3000).

```bash
curl http://localhost:4321/health
docker compose logs -f engine
docker compose down                  # -v para borrar también el volumen
```

Con `MIRAI_LLM_PROVIDER=mock` (el default) no hace falta ninguna API key: el
motor responde de forma determinista y la suite corre sin red.

### Con un LLM real

```bash
# En .env:
#   MIRAI_LLM_PROVIDER=gemini
#   GOOGLE_API_KEY=...
docker compose up -d --build
```

O con un modelo local, sin claves de terceros:

```bash
docker compose --profile llm-local up -d
docker compose exec ollama ollama pull qwen3:8b
# en .env: MIRAI_LLM_PROVIDER=ollama
docker compose up -d engine
```

---

## Qué hay acá

| Archivo | Qué es |
|---|---|
| `docker/Dockerfile` | Tres etapas: `builder` (compila), `runtime` (lo que se despliega), `tester` (la suite e2e). |
| `docker/entrypoint.sh` | Traduce `MIRAI_HOST`/`MIRAI_PORT` a flags: `mirai serve` los lee solo por línea de comandos. |
| `docker/smoke.sh` | Suite e2e contra la API HTTP. |
| `docker/flows/` | Los nueve flujos de prueba. |
| `docker-compose.yml` | Servicios `engine`, `smoke` (perfil `test`) y `ollama` (perfil `llm-local`). |
| `.dockerignore` | Contexto mínimo de build. |

### Decisiones de la imagen

- **Multi-stage.** La imagen final no lleva el toolchain de Rust: solo el
  binario. El contexto de build excluye `target/`, `docs/` y `sdks/`.
- **Cache mounts de BuildKit** para el registry de cargo y `target/`. El binario
  se copia a `/out` dentro del mismo `RUN` porque el mount desaparece al cerrar
  la capa.
- **Sin dependencias extra en el builder.** `rust:slim` ya trae `gcc` y
  `libc6-dev`, que es todo lo que necesita `rusqlite` (feature `bundled`) para
  compilar `sqlite3.c`. `reqwest` usa rustls, así que no hace falta OpenSSL.
- **Usuario sin privilegios** (`mirai`, uid 10001). El motor expone `system/bash`
  y `filesystem/*` a cualquier YAML que se le registre: como root, un spec
  arbitrario tendría el contenedor entero. La suite lo verifica.
- **`debian:bookworm-slim`, no distroless.** El catálogo incluye `system/bash` y
  `git/*`; una imagen sin shell los rompe. El flujo 07 detecta esa regresión si
  algún día se cambia la base.
- **`WORKDIR /data`** es el volumen. Las rutas relativas de `filesystem/*` caen
  ahí y no en el filesystem de la imagen.
- **Apagado limpio.** El binario queda como PID 1 vía `exec`, y el motor ya
  implementa graceful shutdown en SIGTERM: `docker stop` no corta ejecuciones
  en vuelo.

---

## Los flujos

Cada uno ejercita algo distinto del runtime. Se registran vía
`POST /api/v1/agents/from-spec` y se ejecutan por HTTP, igual que lo haría un
cliente de la nube.

| Flujo | Qué prueba |
|---|---|
| `01-pipeline-lineal` | Recorrido secuencial completo. El "¿está vivo?" con nodos reales. |
| `02-ruteo-condicional` | La decisión de camino ocurre en el motor, no en el cliente. Se corre con dos entradas y se verifica que toma una rama y no la otra. |
| `03-fanout-paralelo` | Concurrencia real dentro de un mismo request. |
| `04-llm-basico` | El servidor arma su LLM desde el entorno. El mismo YAML corre con `mock` en CI y con un proveedor real en producción. |
| `05-contrato-inputs` | El motor valida en su propio borde: entrada incompleta → 422 desde el motor. |
| `06-persistencia-volumen` | El volumen es escribible por el usuario sin privilegios. |
| `07-bash-contenedor` | Hay shell en la imagen **y** el proceso no corre como root. |
| `08-agente-live` | Ciclado autónomo (play/stop). **Brecha: no cicla.** |
| `09-subagente` | Composición de agentes. **Brecha: devuelve un placeholder.** |

Más SSE (`/stream` emite el ciclo completo de eventos), autenticación
(401 sin clave, `/health` público) y los endpoints `/sessions` y `/metrics`.

### Correr la suite fuera de Docker

```bash
OPENMIRAI_URL=http://127.0.0.1:4321 MIRAI_API_KEY=... \
  MIRAI_FLOWS_DIR=./docker/flows ./docker/smoke.sh
```

Necesita `curl`, `jq` y `python3` con PyYAML — el endpoint `from-spec` recibe
JSON, no YAML. La imagen `tester` ya los trae.

---

## Brechas para producción

Lo que encontró este prototipo. Cada punto está verificado contra el código de
la v0.6.0, con la reproducción al lado.

### 1. Los agentes live nunca ejecutan un ciclo

`Scheduler::new()` deja su bandera `running` en `false`
(`engine/src/runtime/scheduler.rs:66`) y **nadie llama a `start()`** fuera de los
tests unitarios del propio módulo. El bucle de ciclos es
`while running.load(...)`, así que no entra nunca.

`POST /play` responde `{"status":"playing"}` igualmente: el agente queda
registrado y jamás corre.

```bash
curl -X POST .../api/v1/agents/$ID/play     # {"status":"playing"}
sleep 4                                      # interval_seconds: 1
curl .../api/v1/agents/$ID/cycles            # {"total_cycles":0}
```

Es la brecha más cara: la ejecución continua es justamente lo que se busca al
llevar el motor a la nube. El arreglo es una línea —llamar `scheduler.start()`
al construir el `AppState` o al levantar `serve()`—, pero toca el core y queda
fuera del alcance de este prototipo.

### 2. Los sub-agentes no se ejecutan

`agent/run_agent` valida profundidad de anidamiento y referencias circulares,
pero su `execute` devuelve un placeholder: *"the real execution happens in the
app layer"* (`engine/src/tools/builtin/agent.rs:154`). Ni el servidor HTTP ni el
CLI implementan esa capa.

El grafo completa con `status: Completed` y el nodo devuelve
`status: "placeholder"`, `result: {}`. Verde falso si nadie mira el nodo.

### 3. Todo el estado vive en memoria del proceso

`AppState` guarda agentes, grafos y sesiones en `HashMap`
(`engine/src/server/state.rs`), y cada ejecución arma su contexto con
`InMemoryDBResource` e `InMemoryStorageResource`
(`engine/src/server/helpers.rs:172`).

Consecuencias directas:

- Reiniciar el contenedor **borra todos los agentes registrados**. Hay que
  volver a hacer `from-spec` de cada uno.
- `data/db_read`, `data/db_write`, `data/storage_*` no persisten entre
  ejecuciones, aunque el motor traiga un `SqliteDBResource` en `adapters/`.
- Las sesiones se desalojan por FIFO a las 10.000.
- **No se puede escalar horizontalmente**: dos réplicas no comparten agentes ni
  sesiones. Hoy es una sola instancia, o sesiones pegajosas y un registro de
  agentes replicado por fuera.

El volumen `/data` cubre solo lo que escriban las herramientas `filesystem/*`.

### 4. El motor no emite un solo log

El código está instrumentado con `tracing` (`info!`, `warn!`), pero **no hay
ningún subscriber**: `tracing-subscriber` no figura en las dependencias de
`engine/` ni de `cli/`. Los `tracing::info!("listening on…")` y los avisos de
ciclo del scheduler no salen por ningún lado.

En la práctica: el contenedor imprime el banner del entrypoint y después
silencio. Sin logs de acceso, sin errores, sin trazas. Para diagnosticar en la
nube hay que agregar un subscriber (una dependencia y una línea en `main`).

### 5. `logic/condition` y `logic/switch` no comparan escalares

Ambos declaran su input de comparación como `FieldType::Object`, y la validación
es estricta (`value.is_object()`, `engine/src/tools/base.rs:109`). Además el nodo
lee el operador de un input llamado `operator`, mientras los YAML de ejemplo
usan `op`. Comparar contra un string —el caso más común— falla antes de
ejecutar:

```
missing required input 'operator'; input 'value' expected object, got string
```

Se reproduce con el test del propio repo:

```bash
mirai run test/test_02_conditional_equals.yaml --provider mock   # → Failed
```

Las condiciones de **arista** sí están bien implementadas
(`GraphRunner::evaluate_condition`), y son las que usa `02-ruteo-condicional`.

### 6. La suite del repo reporta verdes falsos

`test/run_all.sh` decide si un test pasó buscando la palabra `Completed` en
cualquier parte de la salida. Esa palabra aparece en la traza de cada nodo que
sí funcionó (`"Completed trigger/manual in 0ms"`), así que una ejecución con
`status: Failed` se reporta como **PASS**. Es lo que pasa hoy con
`test_02_conditional_equals` y la brecha 5: `11 passed, 0 failed` con el flujo
roto.

Por eso `docker/smoke.sh` afirma sobre el campo `status` del JSON y sobre
`state`/`trace`, nunca por `grep` de texto.

### 7. `USAGE.md` está desactualizado en dos puntos

- Tipos de entrada: documenta `integer`, `array` y `object`; el motor solo
  acepta `text`, `number`, `boolean`, `json`, `file`
  (`InputType`, `engine/src/core/agent_spec.rs:351`). Un YAML con
  `type: integer` no valida.
- Sub-agentes: el ejemplo usa `agent_file`; la herramienta lee `agent_id`.

### 8. Pendientes de endurecimiento

- CORS es `permissive()` (`engine/src/server/mod.rs`): cualquier origen.
- La autenticación es una única clave compartida, sin rotación ni multi-tenant.
- No hay rate limiting ni cuota por cliente.
- `read_only: true` en el contenedor quedó comentado en el compose: falta
  validar que `/tmp` (el motor usa `tempfile`) y `$HOME` no lo rompan.

---

## Qué haría falta para que esto sea un despliegue de verdad

En orden de impacto:

1. **Arrancar el scheduler** (brecha 1). Una línea, desbloquea la ejecución 24/7.
2. **Persistir el registro de agentes** (brecha 3): sin esto, cada reinicio es
   una pérdida de estado y no hay más de una réplica posible.
3. **Un subscriber de tracing** (brecha 4): sin logs no hay operación posible.
4. **Arreglar `logic/condition`** (brecha 5) y la suite que lo tapa (brecha 6).
5. Imagen `-musl` estática para bajar de ~180 MB a decenas, una vez que el
   catálogo de herramientas que dependen del shell esté acotado.
6. Publicar la imagen en un registry y clavar el tag del builder
   (`ARG RUST_VERSION=1.97` en lugar de `1`) para builds reproducibles.
