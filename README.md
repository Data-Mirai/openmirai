# Data Mirai Engine

Open source motor de ejecucion de grafos agentivos. Alternativa a LangGraph y Google ADK.

## Instalacion

```bash
# Solo el motor (como libreria en tu proyecto)
pip install datamirai-engine

# Motor + server standalone con editor visual
pip install datamirai-engine[server]
```

## Uso como libreria

```python
from datamirai_engine import (
    GraphDef, NodeDef, EdgeDef,
    GraphRunner, RegistryExecutor, BlockRegistry,
    SimpleExecutionContext, AuthContext,
)

# 1. Registrar bloques
registry = BlockRegistry()
registry.discover("datamirai_engine.blocks.builtin.logic.condition")
registry.discover("datamirai_engine.blocks.builtin.logic.merge")
registry.discover("datamirai_engine.blocks.builtin.ai.llm_call")
registry.discover("datamirai_engine.blocks.builtin.data.db_read")
registry.discover("datamirai_engine.blocks.builtin.trigger.triggers")

# 2. Definir grafo
graph = GraphDef(
    id="my-graph",
    name="Mi primer grafo",
    nodes=[
        NodeDef(id="t1", block_type="trigger/webhook"),
        NodeDef(id="n1", block_type="ai/llm_call", config={"model": "claude"}),
    ],
    edges=[
        EdgeDef(id="e1", source="t1", target="n1"),
    ],
)

# 3. Crear contexto con tus recursos
context = SimpleExecutionContext.default()  # InMemory para dev
# O con recursos reales:
# context = SimpleExecutionContext(
#     db=tu_postgres, vector=tu_pgvector, storage=tu_s3,
#     llm=tu_llm_client,
#     auth=AuthContext(user_id="user-1", role="ADMIN"),
# )

# 4. Ejecutar
runner = GraphRunner(executor=RegistryExecutor(registry))
result = await runner.run(graph, context=context, entry_node_id="t1")

print(result.status)  # "completed"
print(result.state.snapshot())  # outputs de cada nodo
print(result.trace)  # pasos ejecutados
```

## Uso standalone (con editor visual)

```bash
pip install datamirai-engine[server]
datamirai serve
# http://localhost:8000
```

## Uso con Docker

```bash
docker compose up
# engine + postgres/pgvector + minio
```

## Bloques incluidos

| Categoria | Bloques |
|-----------|---------|
| Trigger | webhook, manual, schedule, event |
| Logic | condition, switch, loop, merge, wait |
| AI | llm_call, transcribe, embeddings |
| Data | db_read, db_write, storage_read, storage_write |

## Crear bloques custom

```python
from datamirai_engine import BaseBlock, BlockSpec, BlockInput, BlockOutput

class MiBloque(BaseBlock):
    spec = BlockSpec(
        block_type="custom/mi_bloque",
        version="1.0.0",
        display_name="Mi Bloque",
        description="Hace algo custom",
        category="custom",
        inputs=[BlockInput(name="data", type="object", required=True)],
        outputs=[BlockOutput(name="result", type="string")],
    )

    async def execute(self, inputs, config, context):
        return {"result": f"procesado: {inputs['data']}"}

# Registrar
registry.register(MiBloque)
```

## Resource Protocols

Para conectar tus propios recursos, implementa estos protocols:

```python
from datamirai_engine import DBResource, VectorResource, StorageResource, LLMResource

class MiPostgres:
    """Implementa DBResource protocol."""
    async def execute(self, query, params=None): ...
    async def fetch_one(self, query, params=None): ...
    async def fetch_all(self, query, params=None): ...

class MiLLM:
    """Implementa LLMResource protocol."""
    async def call(self, *, model, prompt, context=None, **kwargs): ...
    async def embed(self, text, *, model=None): ...
```

## Licencia

MIT
