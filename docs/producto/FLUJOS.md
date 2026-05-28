# FLUJOS.md

## 1. Flujos de Negocio

### flujo-run-agent {#flujo-run-agent}
**Descripcion:** Ejecutar un agente desde CLI o API.
**Actor principal:** → DOMINIO.md#rol-product-engineer

**Pasos:**
1. PE proporciona archivo YAML + input (opcional)
2. CLI/Server parsea AgentSpec
3. Valida inputs contra contrato (si definido)
4. Convierte AgentSpec → GraphDef
5. Engine ejecuta el grafo (→ #flujo-graph-execution)
6. Retorna ExecutionResult (status, state, trace, transcript)

**Resultado esperado:** ExecutionResult con status=Completed y outputs del ultimo nodo.
**Errores posibles:**
- Archivo YAML invalido → error de parsing
- Input validation failed → lista de campos faltantes/invalidos
- Grafo invalido → error de validacion (nodos huerfanos, ciclos)

---

### flujo-serve-http {#flujo-serve-http}
**Descripcion:** Iniciar servidor HTTP para recibir requests.
**Actor principal:** → DOMINIO.md#rol-product-engineer

**Pasos:**
1. PE ejecuta `mirai serve --port 3000`
2. Server bind al puerto
3. Registra middleware de auth (si MIRAI_API_KEY definido)
4. Registra rutas /api/v1/*
5. Acepta requests hasta SIGTERM/Ctrl+C
6. Graceful shutdown: espera requests en vuelo, cierra

**Resultado esperado:** Servidor escuchando en el puerto configurado.
**Errores posibles:**
- Puerto ocupado → error de bind
- Timeout de request (300s) → 408

---

## 2. Maquinas de Estado

### maquina-execution {#maquina-execution}
**Entidad:** ExecutionResult
**Estados:** Running | Completed | Failed | Timeout | Interrupted

```
                    ┌────────────┐
                    │  Running   │
                    └─────┬──────┘
                          │
          ┌───────────────┼───────────────┐──────────────┐
          ▼               ▼               ▼              ▼
   ┌────────────┐  ┌───────────┐  ┌────────────┐  ┌──────────────┐
   │ Completed  │  │  Failed   │  │  Timeout   │  │ Interrupted  │
   └────────────┘  └───────────┘  └────────────┘  └──────────────┘
```

| De | A | Guard (condicion) | Quien puede |
|---|---|---|---|
| Running | Completed | Ultimo nodo ejecutado sin error | Engine |
| Running | Failed | Nodo fallo + on_failure=stop + retries agotados | Engine |
| Running | Timeout | Tiempo total excede timeout_ms | Engine |
| Running | Interrupted | human_input requiere decision o pause_interrupt activo | Engine / PE (resume) |

**Side-effects:**
- Running → Completed: sesion guardada, evento `graph.completed` emitido
- Running → Failed: error registrado en trace, evento `graph.error` emitido
- Running → Interrupted: checkpoint guardado, InterruptInfo disponible para resume

---

### maquina-node-execution {#maquina-node-execution}
**Entidad:** TraceEntry (por nodo)
**Estados:** Pending | Executing | Ok | Error | Skipped

```
   ┌──────────┐
   │ Pending  │
   └────┬─────┘
        ▼
   ┌──────────┐
   │Executing │
   └────┬─────┘
        │
   ┌────┼────┐
   ▼    ▼    ▼
 ┌────┐┌───┐┌─────────┐
 │ Ok ││Err││ Skipped │
 └────┘└─┬─┘└─────────┘
         │
    retry loop
    (si retries > 0)
```

| De | A | Guard (condicion) | Quien puede |
|---|---|---|---|
| Pending | Executing | Nodos predecesores completados + condicion de edge evaluada | Engine |
| Executing | Ok | Tool retorna sin error | Engine |
| Executing | Error | Tool falla + retries agotados | Engine |
| Executing | Skipped | Hook pre_block_exec retorna Skip, o on_failure=skip | Engine |
| Error | Executing | Retry policy activa + intentos disponibles | Engine |

**Side-effects:**
- Executing → Ok: output guardado en state, evento `node.completed` emitido
- Executing → Error: error en trace, hook on_error ejecutado
- Error + retry: backoff aplicado (none/linear/exponential), contador incrementado

---

### maquina-edge-evaluation {#maquina-edge-evaluation}
**Entidad:** Edge (por edge saliente de un nodo)
**Descripcion:** Cuando un nodo completa, el engine evalua sus edges salientes.

**Algoritmo:**
1. Recoger todos los edges salientes del nodo
2. Separar en condicionales e incondicionales
3. Si hay condicionales: evaluar cada uno en orden
   - Primer match → seguir ese edge (exclusivo)
   - Si ninguno matchea → usar edges incondicionales como fallback
4. Si solo hay incondicionales: seguir TODOS (fan-out)
5. Si multiples edges van a un mismo nodo destino y ese nodo tiene multiples inputs → fan-in (espera todos)

| Operador | Evaluacion |
|---|---|
| equals / eq | `actual == expected` |
| not_equals / neq | `actual != expected` |
| greater_than / gt | `actual > expected` (numeros) |
| less_than / lt | `actual < expected` (numeros) |
| greater_or_equal / gte | `actual >= expected` |
| less_or_equal / lte | `actual <= expected` |
| contains | `actual.contains(expected)` (strings) o `actual.includes(expected)` (arrays) |
| in | `expected.includes(actual)` (expected es array) |

---

## 3. Reglas de Negocio Globales

### REGLA-01 {#regla-01}
**Descripcion:** YAML-only agent specs.
**Condicion:** Siempre.
**Efecto:** `AgentSpec::from_file()` rechaza archivos `.json`. Solo acepta `.yaml` y `.yml`.

### REGLA-02 {#regla-02}
**Descripcion:** Mocks solo en unit tests.
**Condicion:** Codigo bajo `#[cfg(test)]`.
**Efecto:** Server, CLI e integration tests usan implementaciones reales (Ollama, SQLite, filesystem). MockLLMResource solo existe para tests unitarios.

### REGLA-03 {#regla-03}
**Descripcion:** Zero silent failures.
**Condicion:** Cualquier error en runtime.
**Efecto:** Todo error se propaga hacia arriba o se loguea a nivel error. Ningun unwrap silencioso. El trace siempre refleja que paso.

### REGLA-04 {#regla-04}
**Descripcion:** Input validation en boundary.
**Condicion:** AgentSpec define `inputs:` con campos required.
**Efecto:** El runner valida antes de ejecutar. Si falta un campo required, retorna error claro sin ejecutar ningun nodo.

### REGLA-05 {#regla-05}
**Descripcion:** Sub-agent depth limit.
**Condicion:** `agent/run_agent` se ejecuta.
**Efecto:** Maximo 3 niveles de anidamiento. Si se excede, el nodo falla con error de depth limit.

### REGLA-06 {#regla-06}
**Descripcion:** Session eviction FIFO.
**Condicion:** Server alcanza 10K sesiones en memoria.
**Efecto:** La sesion mas antigua se elimina para hacer espacio. Previene memory leak.

### REGLA-07 {#regla-07}
**Descripcion:** Request timeout.
**Condicion:** Request de ejecucion en el server.
**Efecto:** Timeout de 300 segundos. Si se excede, retorna 408 y la ejecucion se aborta.

### REGLA-08 {#regla-08}
**Descripcion:** Provider resolution order.
**Condicion:** Se necesita un LLM provider.
**Efecto:** Orden: `--provider` flag → env var → auto-detect Ollama → default Ollama. Primer match gana.

### REGLA-09 {#regla-09}
**Descripcion:** Conditional edge exclusivity.
**Condicion:** Nodo tiene edges condicionales salientes.
**Efecto:** Solo el primer edge cuya condicion matchea se activa. Los demas se ignoran. Si ninguno matchea, se usa el edge incondicional como fallback.

### REGLA-10 {#regla-10}
**Descripcion:** Max iterations per node.
**Condicion:** Un nodo se visita mas de `max_iterations` veces (config del agent).
**Efecto:** Ejecucion se aborta con error de ciclo infinito detectado.
