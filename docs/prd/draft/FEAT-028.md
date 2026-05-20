# FEAT-028 — NVIDIA NIM Provider (100+ Modelos Subsidiados)

**Estado**: Draft
**Fecha**: 2026-05-18
**Depende de**: FEAT-027 (Energy Metering — costo $0 pero metering sigue aplicando)

---

## Problem Statement

**Tipo**: Feature nueva (provider LLM adicional)
**Actor**: OWNER/ADMIN que configura recursos LLM, EDITOR que selecciona modelos en grafos.

Hoy Data Mirai Engine soporta 6 providers LLM: Ollama, OpenAI, Claude, Gemini, Groq, OpenRouter. NVIDIA NIM ofrece 100+ modelos de múltiples vendors (Kimi, Minimax, DeepSeek, Llama, Qwen, Mistral, etc.) vía API OpenAI-compatible con tier gratuito subsidiado por NVIDIA.

Oportunidad estratégica:
1. **Local**: usuarios acceden a modelos world-class a costo $0 → tracción
2. **Cloud**: Datamirai cobra energía sobre inferencia que cuesta $0 → margen puro
3. **Catálogo**: 100+ modelos disponibles sin integraciones adicionales
4. **Ventana temporal**: NVIDIA subsidia como funnel de ventas → puede cambiar

---

## Objetivo

Cuando esté implementado:
1. Usuario configura provider `nvidia_nim` con API key `nvapi-xxx`
2. `list_models()` retorna catálogo dinámico de modelos disponibles en NIM
3. Nodos LLM Call seleccionan cualquier modelo NIM (formato `provider/model-name`)
4. Ejecución idéntica a OpenAI — streaming, tool calling, embeddings
5. Energy metering registra `LLM_CALL` con `actual_cost = 0` (subsidiado) + `platform_margin` configurable

---

## Diseño Técnico

### Adapter: `NvidiaAdapter` hereda `OpenAIAdapter`

Patrón idéntico a `GroqAdapter` y `OpenRouterAdapter`:

| Propiedad | Valor |
|---|---|
| Clase base | `OpenAIAdapter` |
| `provider_name` | `nvidia_nim` |
| `base_url` default | `https://integrate.api.nvidia.com/v1` |
| Auth header | `Authorization: Bearer nvapi-xxx` (estándar OpenAI) |
| Model ID format | `provider/model-name` (ej. `moonshotai/kimi-k2-instruct`) |
| Streaming | Soportado (heredado) |
| Tool calling | Soportado (heredado) |
| Embeddings | Soportado (modelos que lo ofrezcan) |

### Override: `list_models()`

NIM tiene 100+ modelos. Override para:
1. Llamar a API de catálogo NIM (si existe endpoint)
2. Fallback: lista curada de modelos populares hardcodeada como baseline
3. Cachear resultado (modelos no cambian cada minuto)

### Modelos Iniciales (baseline hardcoded)

| Provider | Modelo | ID API | Tipo |
|---|---|---|---|
| Moonshot AI | Kimi K2 Instruct | `moonshotai/kimi-k2-instruct` | Chat (1T MoE) |
| Moonshot AI | Kimi K2.5 | `moonshotai/kimi-k2.5` | Chat multimodal |
| MiniMax | M2.7 | `minimaxai/minimax-m2.7` | Chat (230B MoE) |
| DeepSeek | V4 Flash | `deepseek-ai/deepseek-v4-flash` | Chat (1M context) |
| Meta | Llama 3.3 70B | `meta/llama-3.3-70b-instruct` | Chat |
| Meta | Llama 4 | `meta/llama-4-maverick-17b-128e-instruct` | Chat |
| Qwen | Qwen 3 | `qwen/qwen3-235b-a22b` | Chat |
| Mistral | Mistral Large | `mistralai/mistral-large-2-instruct` | Chat |
| NVIDIA | Nemotron | `nvidia/llama-3.1-nemotron-ultra-253b-v1` | Chat |
| GPT-OSS | GPT-OSS 120B | `gpt-oss/gpt-oss-120b` | Chat |

### Registro

Agregar a `_ensure_adapters_registered()` en `llm_providers.py`:
```
("nvidia_nim", NvidiaAdapter)
```

### Rate Limits

- ~40 req/min (tier gratuito)
- Implementar: retry con backoff exponencial (heredado de OpenAIAdapter)
- Opcional futuro: rate limiter por provider para no quemar créditos

---

## UI/UX — Configuración de Provider

### Settings Page: Agregar NVIDIA NIM al Grid

Agregar `nvidia_nim` al grid de provider types existente con:
- Icono/color: verde NVIDIA (#76B900)
- Label: "NVIDIA NIM"
- Sublabel: "100+ modelos gratuitos"

### Campos del formulario (condicional a `nvidia_nim`)

| Campo | Tipo | Requerido | Notas |
|---|---|---|---|
| Display Name | text | sí | Default: "NVIDIA NIM" |
| API Key | password | sí | Placeholder: `nvapi-xxx` |
| Base URL | text | no | Default: `https://integrate.api.nvidia.com/v1` (oculto, solo visible en modo avanzado) |
| Default Model | select | sí | Populated dinámicamente via `list_models()`. Fallback: text input |
| Embedding Model | text | no | Opcional |

### Onboarding Helper (solo local app)

Cuando usuario selecciona `nvidia_nim` como provider type, mostrar banner/card de ayuda:

```
🟢 NVIDIA NIM — Modelos gratuitos
Accede a 100+ modelos de IA (Kimi, DeepSeek, Llama, Qwen, Mistral y más) sin costo.

1. Crea una cuenta gratuita en NVIDIA Developer
2. Genera tu API key (formato nvapi-xxx)
3. Pégala aquí y selecciona un modelo

[Obtener API Key →]  (link externo: https://build.nvidia.com/settings/api-keys)
```

### Test Connection

Botón "Probar conexión" (ya existente en patrón UI). Para NIM:
- Llama a `test_connection()` → valida API key + conectividad
- Success: muestra modelo usado + "Conexión exitosa"
- Error: muestra mensaje descriptivo (key inválida, rate limit, etc.)

### Model Selector en NodeConfigPanel

Cuando provider seleccionado es NIM:
- Dropdown muestra modelos agrupados por vendor (Moonshot AI, MiniMax, Meta, etc.)
- Formato display: `Kimi K2 Instruct (moonshotai/kimi-k2-instruct)`
- Hint: "100+ modelos disponibles — selecciona el que mejor se ajuste a tu caso"

---

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Tier gratuito desaparece | Adapter sigue funcionando con API key de pago. Energía se ajusta |
| Model IDs cambian frecuentemente | `list_models()` dinámico + baseline actualizable |
| Rate limits bajan | Retry con backoff ya implementado. Rate limiter futuro |
| Créditos se agotan rápido en modelos grandes | Documentar consumo por modelo. UI muestra warning si test_connection falla |

---

## Alcance

**In scope**:
- `NvidiaAdapter` class (hereda OpenAI)
- Registro en factory
- UI: `nvidia_nim` como opción de provider en Settings
- Onboarding helper con link a NVIDIA Developer
- Test de conexión + list_models dinámico
- Model selector agrupado por vendor en NodeConfigPanel
- Tests E2E

**Out of scope**:
- Self-hosted NIM (contenedores Docker) — futuro
- Rate limiter por provider — FEAT separada si se necesita
- Agrupación visual por vendor en catálogo (v1 = lista plana)
- Precarga de modelos populares en UI (v1 = dinámico del API)

---

## Esfuerzo Estimado

Mínimo. Herencia directa de `OpenAIAdapter`. ~50 líneas de código nuevo + registro + tests.
Comparable a lo que tomó agregar Groq o OpenRouter.
