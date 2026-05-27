# PRD-006 — Implementation Log

| Campo | Valor |
|-------|-------|
| **Branch** | prd/PRD-006 |
| **Orden** | Fase 3 → Fase 2 → Fase 1 (arreglar adentro, después la fachada) |
| **Inicio** | 2026-05-27 |

---

## Progreso

| Sección | Estado | Commit | Notas |
|---------|--------|--------|-------|
| S03 | completado | cbf5253 | voice.rs + channels.rs eliminados. AgentRuntime.execute_agent() preservado (scheduler depende de él) |
| S14 | completado | cbf5253 | RetryPolicy::default() alineado con AgentRetryConfig (3 retries, exponential) |
| S15 | completado | cbf5253 | resources/ → adapters/ + backward-compat module |
| S11 | completado | cbf5253 | core/well_known.rs con 14 constantes, cero magic strings en runner.rs |
| S10 | parcial | cbf5253 | map_reqwest_error → llm/error.rs (4→1). auto_generate_edge_ids redundante eliminada. Falta: test fixtures compartidos |
| S04 | completado | 9d7e96a | X-API-Key middleware, MIRAI_API_KEY env var, /health y /version siempre públicos |
| S05 | completado | 9d7e96a | Graceful shutdown con SIGTERM + Ctrl+C via tokio::signal |
| S01 | completado | b589556 | README completo en inglés, quick start funcional, estructura correcta |
| S02 | completado | b589556 | 4 examples: hello-world.yaml, conditional-routing.yaml, data-pipeline.yaml, python-quickstart.py |
| S12 | completado | 4c7060b | SimpleExecutionContext → DefaultExecutionContext, SharedState → ExecutionState (con aliases) |
| S06 | pendiente | | Partir runner.rs en 8 archivos |
| S07 | pendiente | | Partir app.rs en handlers/ |
| S08 | pendiente | | Partir data.rs + filesystem.rs |
| S09 | pendiente | | Unificar InputType + FieldType → ValueType |
| S13 | pendiente | | Session eviction + request timeout |
| S20 | pendiente | | run_from() → 50 líneas |
| S17 | pendiente | | Evaluar consolidación LLM |
| S19 | pendiente | | API versioning /api/v1/ |
| S16 | pendiente | | Feature flags |
| S18 | pendiente | | Python SDK mejorado |

## Tests

647 tests pasan en todos los commits. Cero regresiones.

## Decisiones

1. **AgentRuntime.execute_agent() preservado**: El Scheduler depende de él. No es exactamente "código muerto" — es un stub funcional que el scheduler usa. Documentado pero no eliminado.
2. **Backward-compat via type aliases**: Los renames (SharedState, SimpleExecutionContext) usan `pub type OldName = NewName` para no romper la API pública.
3. **Backward-compat via module alias**: `pub mod resources { pub use crate::adapters::*; }` preserva imports existentes del CLI.
