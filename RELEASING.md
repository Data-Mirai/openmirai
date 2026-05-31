# Release Process

## Semver

Format: `MAJOR.MINOR.PATCH`

- **PATCH** (0.1.1): bug fixes, minor improvements that don't break the API.
- **MINOR** (0.2.0): backward-compatible new features.
- **MAJOR** (1.0.0): breaking changes to the API or agent spec format.

## Files that carry a version

Bump all of these before tagging:

| File | Field |
|------|-------|
| `VERSION` | file contents |
| `engine/Cargo.toml` | `version` |
| `cli/Cargo.toml` | `version` |
| `sdks/python/pyproject.toml` | `version` |
| `sdks/python/openmirai/__init__.py` | `__version__` |
| `sdks/typescript/package.json` | `version` |
| `CHANGELOG.md` | new entry at the top |

The HTTP server reads its version from `CARGO_PKG_VERSION` at compile time — no manual sync needed.

## Process

```bash
# 1. Bump versions in every file listed above
# 2. Commit the bump
git add -A
git commit -m "chore: bump version to vX.Y.Z"

# 3. Local validation
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace

# 4. Tag (annotated)
git tag -a vX.Y.Z -m "Release vX.Y.Z — short description"

# 5. Push (maintainer does this manually)
git push origin main
git push origin vX.Y.Z
```

## What GitHub Actions does on tag push

When a tag matching `v*` is pushed, `.github/workflows/release.yml` automatically:

1. Builds release binaries for five targets:
   - `mirai-darwin-arm64` (macOS Apple Silicon)
   - `mirai-darwin-x86_64` (macOS Intel)
   - `mirai-linux-x86_64` (Linux glibc, x86_64)
   - `mirai-linux-arm64` (Linux glibc, aarch64 — ARM servers, Graviton/Ampere, Raspberry Pi)
   - `mirai-windows-x86_64.exe` (Windows)
2. Computes `SHA256SUMS` for all artifacts.
3. Creates a GitHub Release with auto-generated notes and uploads every artifact.

End users download the binary for their platform from the [Releases page](https://github.com/Gabo-TheCreator/openmirai/releases).

## Publishing the SDKs

The Python and TypeScript SDKs are independent of the binary release. Publish manually when their version changes:

```bash
# Python (PyPI)
cd sdks/python
python -m build
python -m twine upload dist/*

# TypeScript (npm)
cd sdks/typescript
npm publish
```

## Consumers

Projects that depend on the engine pin to a specific version:

- **Mirai Local**: ships a bundled `mirai` binary from a GitHub Release.
- **Mirai Cloud**: consumes the lib via Cargo or runs the binary in containers.
- **SDKs**: track the engine version they target.

Each consumer decides when to upgrade.
