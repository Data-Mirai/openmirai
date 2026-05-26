# Data Mirai Engine — Instrucciones

## Estructura del repositorio

```
Engine/
  Cargo.toml     → Workspace Rust
  engine/        → Crate principal (datamirai-engine)
  cli/           → CLI interactivo (datamirai-cli)
  docs/          → Documentación pública
```

**Motor real = Rust.** Todo el desarrollo va en `engine/` y `cli/`.

## Stack

- Rust (motor principal)
- Tokio (async runtime)
- Axum (HTTP server)
- SQLite / rusqlite (persistencia local)
- Serde (serialización JSON/YAML)

## Reglas absolutas

- Idioma: español
- Tono: senior, directo, sin walls of text
- Git: NUNCA push (PM lo hace). NUNCA force push, reset --hard, clean -f, rebase sin autorización.
- Archivos sagrados: NUNCA editar `.env*`
- Siempre leer documentación antes de proponer cambios

## Testing

```bash
cd engine && cargo test      # 553+ tests
cargo build --release -p datamirai-cli  # binario
```
