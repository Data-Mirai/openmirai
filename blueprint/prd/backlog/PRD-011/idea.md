# PRD-011 — SOLID Cleanup: DRY + Single Source of Truth

| Campo | Valor |
|-------|-------|
| **ID** | PRD-011 |
| **Fecha** | 2026-05-28 |
| **Estado** | designed |
| **Branch** | prd/PRD-011 |
| **Target** | v0.5.1 |

---

## Problema

- **Tipo**: mejora (code quality)
- **Resumen**: El codebase tiene 3 violaciones DRY identificadas en el analisis post-Kondo. Cada una tiene duplicacion que aumenta el riesgo de divergencia y reduce la legibilidad narrativa del dominio.
- **Actores**: Developer (lee y mantiene el codigo), Engine (runtime)
- **Que cambia**:
  - HOY: 10 copias de `field()`, 3 copias de `value_type_label`, base64 data clonada innecesariamente
  - DESPUES: 1 fuente canonica para cada concepto, memoria optimizada para archivos grandes

---

## Analisis de cada hallazgo

### Hallazgo 1: `field()` helper — 10 copias identicas

```
engine/src/tools/builtin/ai.rs:18
engine/src/tools/builtin/system.rs:18
engine/src/tools/builtin/mcp.rs:19
engine/src/tools/builtin/data/mod.rs:19
engine/src/tools/builtin/agent.rs:16
engine/src/tools/builtin/output.rs:16
engine/src/tools/builtin/trigger.rs:17
engine/src/tools/builtin/git.rs:17
engine/src/tools/builtin/filesystem/mod.rs:20
engine/src/tools/builtin/logic.rs:17
```

Las 10 copias son byte-for-byte identicas:
```rust
fn field(name: &str, field_type: FieldType, required: bool, desc: &str) -> ToolField {
    ToolField {
        name: name.into(),
        field_type,
        required,
        description: if desc.is_empty() { None } else { Some(desc.into()) },
        default: None,
    }
}
```

**Impacto de NO arreglar**: Si alguien agrega un campo a ToolField (ej: `deprecated: bool`), tiene que actualizar 10 archivos. Si olvida uno, compila porque cada `field()` es local.

**Impacto de arreglar**: Un solo cambio en `tools/base.rs` propaga a todos. El codigo de cada tool module se lee mas limpio — no empieza con boilerplate.

**Decision: SI implementar.**
- Riesgo: cero (funcion pura, sin side effects)
- Esfuerzo: bajo (mover + buscar/reemplazar imports)
- Beneficio: alto (SRP — base.rs es dueno de ToolField, deberia ser dueno de su constructor)

**Operacion**:
1. Mover `field()` a `tools/base.rs` como `pub fn field()`
2. En los 10 tool modules: eliminar la copia local, agregar `use crate::tools::base::field`
3. Verificar que los macros `ai_tool!` y `system_tool!` siguen compilando (usan `field()` por nombre)

---

### Hallazgo 2: `value_type_label` — 3 copias, 1 inconsistente

```
engine/src/core/value_type.rs:98  → pub fn value_type_label() → "text" para strings
engine/src/tools/base.rs:139     → fn value_type_label()     → "string" para strings
engine/src/tools/builtin/ai.rs:680 → fn json_type_name()     → "string" para strings
```

La inconsistencia: `core/value_type.rs` usa "text" (vocabulario del dominio — coincide con InputType::Text del agent spec). Las otras dos usan "string" (vocabulario tecnico de serde_json).

**Impacto de NO arreglar**: Mensajes de error inconsistentes. Un developer ve `expected text, got number` en un lugar y `expected string, got number` en otro para el mismo concepto.

**Impacto de arreglar**: Un solo vocabulario. El mensaje de error siempre dice lo mismo.

**Decision: SI implementar.**
- Riesgo: bajo (mensajes de error cambian — no afecta logica, solo texto)
- Esfuerzo: bajo (eliminar 2 copias, importar la canonica)
- Beneficio: medio (DDD — vocabulario consistente del dominio)

**Pregunta de producto para el PM**: que vocabulario usar? "text" (dominio) o "string" (tecnico)?

**Operacion**:
1. Elegir vocabulario (propuesta: "string" — es lo que FieldType::String dice, es lo que serde_json usa, es lo que el developer ve en `mirai tools`)
2. Actualizar `core/value_type.rs` para usar "string" en vez de "text"
3. Eliminar la copia en `tools/base.rs` — importar desde `core/value_type.rs`
4. Eliminar `json_type_name` en `ai.rs` — importar la canonica
5. Buscar cualquier test que asserte contra "text" y actualizar

---

### Hallazgo 3: Base64 data clonada en convert_messages

Flujo actual para un archivo de 20MB:
```
read_media_file()
  → bytes: Vec<u8>     20MB (scoped, se dropea)
  → b64: String         27MB (en MediaContent.data)

convert_messages(&[Message])  ← borrow, no puede mover
  → json!({"data": mc.data})  27MB CLONE (serde_json copia el String)

reqwest::json(&payload)
  → serialized body      27MB (reqwest serializa el Value)

Peak: 27MB (MediaContent) + 27MB (JSON clone) + 27MB (HTTP body) = ~81MB
```

Si `convert_messages` tomara `Vec<Message>` (ownership), podria mover el String:
```
convert_messages(messages: Vec<Message>)  ← ownership
  → json!({"data": std::mem::take(&mut mc.data)})  MOVE, no clone

Peak: 27MB (JSON con data movido) + 27MB (HTTP body) = ~54MB
```

**Impacto de NO arreglar**: Para archivos de <5MB (99% de los casos reales), la diferencia es ~15MB → irrelevante. Para archivos de 20MB, son ~27MB extra en peak memory. No es un crash — es overhead.

**Impacto de arreglar**: Requiere cambiar la firma de `convert_messages` en 4 adapters + todos sus tests. Invasivo. El beneficio solo se materializa con archivos grandes.

**Decision: NO implementar ahora.**
- Riesgo: medio (cambio de firma en 4 adapters, 15+ tests afectados)
- Esfuerzo: medio-alto
- Beneficio: bajo (la mayoria de archivos son <5MB; el limite es 20MB)
- Alternativa futura: streaming base64 encoder que nunca carga todo en memoria

---

## Resumen de decisiones

| Hallazgo | Implementar? | Razon |
|----------|-------------|-------|
| `field()` x10 | SI | Zero riesgo, alto beneficio SRP |
| `value_type_label` x3 | SI | Vocabulario consistente del dominio |
| Base64 clone en adapters | NO | Costo/beneficio no justifica la invasion |

---

## Operaciones

### unificar_field_helper

- **Actor**: Engine (compile-time)
- **Input**: 10 archivos con copias locales de `field()`
- **Logica**:
  1. Mover la funcion a `tools/base.rs` como `pub fn field()`
  2. En cada tool module: eliminar `fn field()` local
  3. Agregar `use crate::tools::base::field` donde se use directamente
  4. Verificar que los macros (`ai_tool!`, `system_tool!`) resuelven `field` correctamente
- **Output**: 10 archivos mas limpios, 1 fuente canonica
- **Errores**: si un macro no resuelve `field()`, error de compilacion (detectado inmediatamente)

### unificar_value_type_label

- **Actor**: Engine (compile-time + runtime mensajes de error)
- **Input**: 3 archivos con copias de la misma funcion
- **Logica**:
  1. Hacer `pub` la funcion canonica en `core/value_type.rs`
  2. Actualizar String label a "string" (o "text" — decision del PM)
  3. En `tools/base.rs`: eliminar `fn value_type_label`, importar desde `core/value_type`
  4. En `tools/builtin/ai.rs`: eliminar `fn json_type_name`, importar la canonica
  5. Actualizar tests que asserten contra el label viejo
- **Output**: 1 fuente canonica, mensajes de error consistentes
- **Errores**: tests que asserten contra "text" fallaran si cambiamos a "string" (detectado en CI)

---

## Reglas de Negocio

### zero-regression

- **Invariante**: Todos los 712 tests existentes deben pasar sin modificacion (excepto los que asserten contra el label "text" → se actualizan).
- **Cuando se verifica**: despues de cada cambio
- **Si se viola**: revertir y diagnosticar

### no-comportamiento-nuevo

- **Invariante**: Este PRD no agrega funcionalidad nueva. Solo mueve codigo existente. El output del engine para cualquier input debe ser identico antes y despues.
- **Cuando se verifica**: comparar output de `mirai run examples/hello-world.yaml` antes/despues
- **Si se viola**: el refactor introdujo un bug — revertir

---

## Escenarios GWT

TEST-133: field() movida a base.rs — compila y tools se registran
  Given: 10 tool modules importan field() desde base.rs
  When: cargo build
  Then: Compila sin errores. `mirai tools` lista los mismos 49 tools.

TEST-134: field() eliminada de tool modules — no queda copia local
  Given: Refactor completado
  When: grep "^fn field(" en engine/src/tools/
  Then: Solo aparece en base.rs. Cero copias locales.

TEST-135: value_type_label unificada — vocabulario consistente
  Given: Solo existe una version en core/value_type.rs
  When: Un tool recibe input de tipo incorrecto (ej: number donde esperaba string)
  Then: Error message dice "expected string, got number" (no "text").

TEST-136: value_type_label eliminada de base.rs y ai.rs
  Given: Refactor completado
  When: grep "fn value_type_label\|fn json_type_name" en engine/src/
  Then: Solo aparece en core/value_type.rs. Cero copias.

TEST-137: Suite completa pasa sin regresiones
  Given: Refactor completado
  When: cargo test
  Then: 712+ tests pasan. 0 fallos.

TEST-138: Output identico pre/post refactor
  Given: Output guardado de `mirai run examples/hello-world.yaml` antes del refactor
  When: Mismo comando despues del refactor
  Then: JSON output identico (salvo timestamps).

---

## Fuera de Alcance

- **Base64 memory optimization en adapters**: Decidido NO implementar. Costo/beneficio no justifica. Documento aqui para referencia futura.
- **Refactor de macros ai_tool!/system_tool!**: Los macros funcionan. Tocarlos es riesgoso sin beneficio claro.
- **HTTP boilerplate en adapters**: Cada adapter tiene su propio request/response parsing. Es correcto por provider — no hay abstraccion que valga la pena.

---

## Orden de implementacion

```
1. Mover field() a tools/base.rs como pub fn
2. Eliminar 10 copias locales, agregar imports
3. cargo build + cargo test → verificar 0 regresiones
4. Unificar value_type_label → core/value_type.rs canonica
5. Eliminar copias en base.rs y ai.rs
6. Actualizar tests que asserten contra label viejo
7. cargo test → verificar 0 regresiones
8. Commit
```
