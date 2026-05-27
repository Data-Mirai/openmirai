# Release Process

## Semver

Formato: `MAJOR.MINOR.PATCH`

- **PATCH** (0.1.1): bug fixes, mejoras menores que no rompen API
- **MINOR** (0.2.0): features nuevas backward-compatible
- **MAJOR** (1.0.0): breaking changes en API/formato de agent specs

## Archivos que llevan versión

Todos deben coincidir antes de tagear:

| Archivo | Campo |
|---------|-------|
| `VERSION` | contenido del archivo |
| `engine/Cargo.toml` | `version` |
| `cli/Cargo.toml` | `version` |
| `engine/src/server/app.rs` | health + version endpoints |
| `sdks/python/pyproject.toml` | `version` |
| `sdks/python/datamirai/__init__.py` | `__version__` |
| `sdks/typescript/package.json` | `version` |

## Proceso de release

```bash
# 1. Bump versión en todos los archivos
#    (usar find/replace o script futuro)

# 2. Commit del bump
git add -A
git commit -m "chore: bump version to vX.Y.Z"

# 3. Build + test
cargo build --release
cargo test --lib
python3 demos/15-showcase/mirai_showcase.py  # E2E

# 4. Tag
git tag -a vX.Y.Z -m "Release vX.Y.Z — [descripción]"

# 5. Copiar binario a releases/
mkdir -p releases/vX.Y.Z
cp target/release/mirai releases/vX.Y.Z/mirai-vX.Y.Z-$(uname -s | tr A-Z a-z)-$(uname -m)

# 6. Push (PM lo hace)
git push origin main --tags
```

## Releases existentes

| Versión | Fecha | Binary | Notas |
|---------|-------|--------|-------|
| v0.1.0 | 2026-05-27 | `releases/v0.1.0/mirai-v0.1.0-darwin-arm64` | First public release. 23 features, 47 tools, 636 tests. |

## Consumidores

Los proyectos que dependen del motor apuntan a una versión específica:
- **Mirai Local**: consume el binary de `releases/vX.Y.Z/`
- **Mirai Cloud**: consume el binary o la lib vía Cargo dependency
- **SDKs**: publican su propia versión alineada al motor

Cuando el motor sube de versión, cada consumidor decide cuándo actualizar.
