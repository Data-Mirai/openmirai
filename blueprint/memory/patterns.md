# Patterns — Data Mirai Engine

## Stack

- Python 3.12+ / FastAPI / async
- Next.js 15 + TS + React Flow (editor)
- PostgreSQL 16+ + pgvector
- S3-compatible storage

## Distribución

- Un artefacto PyPI: `datamirai-engine`
- Frontend compilado a static → servido por FastAPI
- Modos: standalone (`datamirai serve`), embebido (import SDK + mount editor), cloud (dep interna)

## Testing

- pytest (Python)
- vitest (frontend)
- Playwright (E2E UI)

## Linting

- ruff (Python lint+format)
- eslint + prettier (TS)

## Deps

- pyproject.toml (pip/uv)
- npm + package.json

## Convenciones

- Python: snake_case archivos/funciones, PascalCase clases
- TS: PascalCase componentes, camelCase funciones/utils
- block_type: `categoria/nombre_snake`
- Interfaces agnósticas para recursos (DB, vector, storage, LLM)
- ExecutionContext inyecta acceso — engine no sabe dónde viven recursos físicos

## Decisiones clave

- Monorepo: editor + engine en mismo repo
- Single process: FastAPI sirve API + static assets del editor
- No vendor lock-in: interfaces abstractas para todo recurso externo
- Cloud = wrapper de orquestación, no infra provider
