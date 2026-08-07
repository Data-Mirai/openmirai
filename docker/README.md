# OpenMirai en contenedores — prototipo

Empaqueta el motor en una imagen, lo levanta como servicio HTTP y lo somete a
una suite e2e que ejercita capacidades distintas del runtime. El objetivo no es
un despliegue productivo: es **medir qué tan lista está la v0.7.0 para correr en
la nube** y dejar el diagnóstico por escrito.

Resultado de la corrida de referencia **contra el contenedor** —imagen
`openmirai/engine:0.6.0-proto`, no un `mirai serve` nativo—: **22
verificaciones en verde, 1 brecha del motor** (ver
[Brechas para producción](#brechas-para-producción)).

Esa distinción no es cosmética: hay dos verificaciones que solo significan algo
cuando la suite corre contra la imagen. El flujo 07 reporta `usuario: mirai`
—el usuario sin privilegios que crea el Dockerfile, no el de la máquina— y el
06 escribe en `/data/smoke/06-persistencia.txt`, o sea el volumen, no el
directorio desde donde alguien lanzó el motor. Corriendo contra un binario
local ambas pasan sin probar nada.

Lo que trajo v0.7.0 y cambia este diagnóstico: los **runs ya se persisten** en
SQLite, **CORS pasó a loopback-only**, `mirai serve` **bindea 127.0.0.1 por
defecto** y **se niega a arrancar fuera de loopback sin API key** — el
contenedor bindea `0.0.0.0`, así que `MIRAI_API_KEY` es obligatoria. También
quedó arreglado `logic/condition`, que en 0.6.0 no podía comparar strings.

Del diagnóstico salieron además dos cambios en el motor, ambos en el CHANGELOG:

- **Los agentes live no ejecutaban un solo ciclo** (`scheduler.rs`). El flujo 08
  lo verifica de punta a punta: dentro del contenedor el agente cicla solo,
  conserva memoria entre ciclos y vuelve a aceptar `play` al terminar.
- **El registro de agentes ahora se persiste** (`db/agent_store.rs`), sobre la
  **misma base** que los runs de 0.7.0 — un solo archivo, un solo `--db-path`.
  Con `MIRAI_DB_PATH` apuntando al volumen, los agentes sobreviven al reinicio
  del contenedor con el mismo id, y los que estaban ciclando se relanzan solos.
  Se verifica con [`docker/persistencia.sh`](#persistencia-del-registro).

Queda sin probar el apagado limpio (`docker stop`: que el SIGTERM llegue al
PID 1 sin cortar ejecuciones en vuelo) y que el volumen sobreviva un
`docker compose down && up`.

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
| `docker/persistencia.sh` | Verifica que los agentes sobrevivan al reinicio (dos fases, con el reinicio en el medio). |
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
| `08-agente-live` | Ciclado autónomo: `play` → cicla solo → memoria persistida entre ciclos → se desregistra al agotar `max_cycles` y vuelve a aceptar `play`. |
| `09-subagente` | Composición de agentes. **Brecha: devuelve un placeholder.** |

Más SSE (`/stream` emite el ciclo completo de eventos), autenticación
(401 sin clave, `/health` público) y los endpoints `/sessions` y `/metrics`.

### Persistencia del registro

El smoke corre contra un motor ya levantado, así que no puede probar lo único
que importa del registro persistente: que los agentes sobrevivan a que el
proceso muera. Esa prueba va aparte, con el reinicio en el medio:

```bash
docker compose run --rm --entrypoint /usr/local/bin/persistencia.sh smoke preparar
docker compose restart engine
docker compose run --rm --entrypoint /usr/local/bin/persistencia.sh smoke verificar
```

Registra un agente normal y uno live, lo pone a ciclar, y después del reinicio
comprueba cuatro cosas: que los dos agentes sigan ahí **con el mismo id**, que
el restaurado siga siendo ejecutable (no solo consultable) y que el live haya
vuelto a ciclar sin que nadie llamara a `/play`.

Fuera de Docker, pasándole cómo reiniciar el motor:

```bash
REINICIO_CMD="tu-comando-de-reinicio" OPENMIRAI_URL=http://127.0.0.1:4321 \
  MIRAI_API_KEY=... MIRAI_FLOWS_DIR=./docker/flows ./docker/persistencia.sh auto
```

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
la v0.7.0, con la reproducción al lado.

Dos brechas del diagnóstico original ya no están: `logic/condition` no podía
comparar strings (0.7.0 le puso `FieldType::Any` al comparando y normaliza
`equals`→`eq`, así que el flujo 02 volvió a usar el nodo real en vez de un
rodeo con `bash`), y el CORS `permissive()` pasó a loopback-only.

### 1. Los sub-agentes no se ejecutan

`agent/run_agent` valida profundidad de anidamiento y referencias circulares,
pero su `execute` devuelve un placeholder: *"the real execution happens in the
app layer"* (`engine/src/tools/builtin/agent.rs:154`). Ni el servidor HTTP ni el
CLI implementan esa capa.

El grafo completa con `status: Completed` y el nodo devuelve
`status: "placeholder"`, `result: {}`. Verde falso si nadie mira el nodo.

### 2. Queda estado sin persistir

Los runs (0.7.0) y el registro de agentes ya sobreviven al reinicio. Lo que no:

- La **memoria de agentes** (`AgentMemoryStore`) vive en memoria. Un agente
  live retoma su ciclado tras un reinicio, pero arranca sin recuerdos: lo que
  declara `graph.memory` vuelve a sus valores iniciales.
- `data/db_read`, `data/db_write` y `data/storage_*` corren contra
  `InMemoryDBResource` e `InMemoryStorageResource`
  (`engine/src/server/helpers.rs`), así que no persisten entre ejecuciones
  aunque el motor traiga un `SqliteDBResource` en `adapters/`.
- **Escalar horizontalmente sigue sin funcionar.** Dos réplicas sobre el mismo
  volumen comparten el archivo SQLite, pero cada una levanta su propio
  scheduler: un agente live marcado como ciclando se relanzaría en **todas**,
  ejecutando el mismo ciclo N veces. Hoy es una sola instancia.

### 3. El motor no emite un solo log

El código está instrumentado con `tracing` (`info!`, `warn!`), pero **no hay
ningún subscriber**: `tracing-subscriber` no figura en las dependencias de
`engine/` ni de `cli/`. Los `tracing::info!("listening on…")` y los avisos de
ciclo del scheduler no salen por ningún lado.

En la práctica: el contenedor imprime el banner del entrypoint y después
silencio. Sin logs de acceso, sin errores, sin trazas. Para diagnosticar en la
nube hay que agregar un subscriber (una dependencia y una línea en `main`).

### 4. La suite del repo reporta verdes falsos

`test/run_all.sh` decide si un test pasó buscando la palabra `Completed` en
cualquier parte de la salida. Esa palabra aparece en la traza de cada nodo que
sí funcionó (`"Completed trigger/manual in 0ms"`), así que una ejecución con
`status: Failed` se reporta como **PASS**. En 0.6.0 eso tapaba un flujo roto de
verdad (`test_02_conditional_equals` fallaba y la suite decía `11 passed, 0
failed`). En 0.7.0 ese test quedó arreglado, así que hoy el problema no se ve
—pero el mecanismo sigue igual y volverá a tapar la próxima rotura.

Por eso `docker/smoke.sh` afirma sobre el campo `status` del JSON y sobre
`state`/`trace`, nunca por `grep` de texto.

### 5. `USAGE.md` sigue desactualizado

- Tipos de entrada: documenta `integer`, `array` y `object`; el motor solo
  acepta `text`, `number`, `boolean`, `json`, `file`
  (`InputType`, `engine/src/core/agent_spec.rs:351`). Un YAML con
  `type: integer` no valida.
- Sub-agentes: el ejemplo usa `agent_file`; la herramienta lee `agent_id`.

(0.7.0 ya corrigió la descripción de `logic/condition`, que era el tercer punto
de esta lista.)

### 6. Pendientes de endurecimiento

- La autenticación es una única clave compartida, sin rotación ni multi-tenant.
- No hay rate limiting ni cuota por cliente.
- `read_only: true` en el contenedor quedó comentado en el compose: falta
  validar que `/tmp` (el motor usa `tempfile`) y `$HOME` no lo rompan.

---

## Qué haría falta para que esto sea un despliegue de verdad

En orden de impacto:

1. **Un subscriber de tracing** (brecha 3): sin logs no hay operación posible.
   El scheduler ya emite `Cycle started` / `Cycle completed` por `tracing`, así
   que basta con conectar un subscriber para tener visibilidad de los agentes
   live.
2. **Persistir sesiones y memoria de agentes** (brecha 2): el registro de
   agentes ya sobrevive, pero el historial de ejecuciones y los recuerdos de un
   live no. Es lo que falta para poder correr más de una réplica.
3. **Arreglar el criterio de `test/run_all.sh`** (brecha 4): mientras decida por
   `grep`, la próxima rotura vuelve a pasar inadvertida.
4. Imagen `-musl` estática para bajar de ~200 MB a decenas, una vez que el
   catálogo de herramientas que dependen del shell (y de tmux) esté acotado.
5. Publicar la imagen en un registry y clavar el tag del builder
   (`ARG RUST_VERSION=1.97` en lugar de `1`) para builds reproducibles.
