<!-- BLUEPRINT SEED: infra/INFRA.md — build, CI/CD, release, deploy, env vars. Read the code to document: Cargo manifests, .github/workflows/{ci,release}.yml, RELEASING.md, engine/src/server/mod.rs, cli/src/main.rs, engine/src/tools/builtin/system.rs. -->

# Infrastructure & Deployment

OpenMirai is a **single portable binary** with zero external runtime dependencies. It embeds SQLite for persistence, uses rustls for TLS, and requires no containers, databases, or cloud services to run.

## Build System

OpenMirai uses **Cargo** (Rust package manager) for all builds. The workspace contains two crates:

- `engine/` — Core library (`openmirai-engine`), includes HTTP server (optional, feature-gated)
- `cli/` — CLI binary (`mirai`), depends on the engine

### Build Commands

```bash
# Debug build (development, fast compilation, slow runtime)
cargo build --workspace

# Release build (optimized, slow compilation, fast runtime)
cargo build --release

# With all features enabled (default: server + builtin-tools)
cargo build --workspace --all-features

# Specific target only
cargo build --release --bin mirai
```

The release binary is at `target/release/mirai` (Unix) or `target/release/mirai.exe` (Windows).

### Build Features

**Default features** (`features = ["server", "builtin-tools"]`):

- `server` — HTTP API server (Axum, tower-http, CORS)
- `builtin-tools` — All 50 built-in tools (file, data, ai, logic, system, git, webhook, etc.)

To disable the server and use only the graph runner:

```bash
cargo build --release --no-default-features --features builtin-tools
```

### Toolchain

All builds use **stable Rust** (enforced via `dtolnay/rust-toolchain@stable` in CI):

```bash
# Check your Rust version
rustc --version

# Update (if needed)
rustup update stable
```

## CI/CD Pipeline

All CI gates are defined in `.github/workflows/ci.yml` and run on every `push` to `main` and every `pull_request` against `main`.

### CI Jobs

#### 1. Rustfmt (Format Check)

- Runs on: `ubuntu-latest`
- Command: `cargo fmt --all -- --check`
- Purpose: Ensure consistent code style
- Failure blocks merge

#### 2. Clippy (Linter)

- Runs on: `ubuntu-latest`
- Command: `cargo clippy --workspace --all-targets --all-features`
- Environment: `RUSTFLAGS="-D warnings"` (treat warnings as errors)
- Purpose: Catch common mistakes, performance issues, and style problems
- Failure blocks merge

#### 3. Test Suite

- Runs on: **matrix** (`ubuntu-latest`, `macos-latest`)
- Commands:
  - `cargo build --workspace --all-features`
  - `cargo test --workspace --all-features`
- Purpose: Verify all 712 tests pass on Linux and macOS
- Failure blocks merge

### CI Environment

```yaml
CARGO_TERM_COLOR: always      # Colored output
RUSTFLAGS: "-D warnings"      # Treat warnings as errors
```

### CI Gate Requirements

**All three must pass before merge:**

1. Code is formatted (`cargo fmt`)
2. No clippy warnings (`cargo clippy -D warnings`)
3. All tests pass on Ubuntu and macOS

## Release Pipeline

Releases are triggered by pushing a **semver tag** (`v*`) to the main repository. This kicks off `.github/workflows/release.yml`, which:

1. **Builds** release binaries for **five targets**
2. **Computes** SHA256 checksums for integrity verification
3. **Creates** a GitHub Release with all artifacts and notes

### Release Targets

| Target | Platform | Binary Name | Runner |
|--------|----------|-------------|--------|
| `aarch64-apple-darwin` | macOS 14+ (Apple Silicon) | `mirai-darwin-arm64` | `macos-latest` |
| `x86_64-apple-darwin` | macOS 13+ (Intel) | `mirai-darwin-x86_64` | `macos-13` |
| `x86_64-unknown-linux-gnu` | Linux glibc x86_64 | `mirai-linux-x86_64` | `ubuntu-latest` |
| `aarch64-unknown-linux-gnu` | Linux glibc ARM64 (Graviton, Ampere, RPi) | `mirai-linux-arm64` | `ubuntu-24.04-arm` |
| `x86_64-pc-windows-msvc` | Windows 10+ | `mirai-windows-x86_64.exe` | `windows-latest` |

### Release Artifacts

Each release includes:

- Five precompiled binaries (ready to download and run)
- `SHA256SUMS` file for integrity verification
- Auto-generated release notes (linked to CHANGELOG.md)

### Release Process (Manual)

```bash
# 1. Bump versions in all these files:
#    - VERSION (file contents)
#    - engine/Cargo.toml (version field)
#    - cli/Cargo.toml (version field)
#    - sdks/python/pyproject.toml (version field)
#    - sdks/python/openmirai/__init__.py (__version__)
#    - sdks/typescript/package.json (version field)
#    - CHANGELOG.md (new entry at top)

# 2. Commit the bump
git add -A
git commit -m "chore: bump version to vX.Y.Z"

# 3. Validate locally (must pass before tag)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace

# 4. Create annotated tag
git tag -a vX.Y.Z -m "Release vX.Y.Z — brief description"

# 5. Push (maintainer only)
git push origin main
git push origin vX.Y.Z
```

Once the tag is pushed, GitHub Actions automatically builds and publishes the release. Monitor progress at the [Actions](https://github.com/Data-Mirai/openmirai/actions) tab.

### Version Format

Follows **Semantic Versioning**: `MAJOR.MINOR.PATCH`

- **PATCH** (0.1.1): bug fixes, minor improvements that don't break the API
- **MINOR** (0.2.0): backward-compatible new features
- **MAJOR** (1.0.0): breaking changes to the API or agent spec format

### HTTP Server Version

The HTTP server reads its version at compile time via `env!("CARGO_PKG_VERSION")` — no manual sync needed. The server responds to `GET /version` with the compiled version.

## Deployment

OpenMirai is deployed as a **single self-contained binary** with zero external dependencies:

- SQLite is **bundled** (compiled in via `rusqlite` with feature `bundled`)
- TLS is handled by **rustls** (no system OpenSSL needed)
- No environment containers, no database servers, no sidecar services

### Deployment Models

#### 1. CLI Usage (Local)

Download a binary from [GitHub Releases](https://github.com/Data-Mirai/openmirai/releases), verify the SHA256, and run:

```bash
# macOS (Apple Silicon)
curl -L https://github.com/.../releases/download/v0.6.0/mirai-darwin-arm64 -o mirai
chmod +x mirai
./mirai run my-agent.yaml

# Linux
curl -L https://github.com/.../releases/download/v0.6.0/mirai-linux-x86_64 -o mirai
chmod +x mirai
./mirai run my-agent.yaml

# Windows
# Download mirai-windows-x86_64.exe from Releases and run in PowerShell
```

#### 2. HTTP Server (Remote/Cloud)

```bash
# Start the server on localhost:3000
./mirai serve --port 3000

# Or use environment variable (see Environment Variables below)
MIRAI_API_KEY=secret-key ./mirai serve --port 8080
```

The server exposes the full API at `/api/v1/*` with optional API key authentication.

#### 3. Embedded (Rust Crate)

Include the engine in your Rust app:

```toml
[dependencies]
openmirai-engine = "0.6"
```

#### 4. Python/TypeScript SDKs

Consume the engine's HTTP API:

```python
from openmirai import MiraiClient
client = MiraiClient("http://localhost:3000")
result = client.execute_agent("agent-id", input={"key": "value"})
```

## Environment Variables

OpenMirai respects environment variables for LLM configuration, API keys, and tool injection. **No real API keys should ever be hardcoded; always use environment variables.**

### MIRAI_* Variables (Engine-Managed)

#### `MIRAI_LLM_PROVIDER`

**Type:** String  
**Default:** `ollama`  
**Values:** `ollama`, `openai`, `claude`, `anthropic`, `gemini`, `google`, `groq`, `nvidia`, `openrouter`, or any custom OpenAI-compatible provider name

**Purpose:** Default LLM provider if not specified via `--provider` flag. Overridden by explicit `--provider`.

**Example:**
```bash
export MIRAI_LLM_PROVIDER=claude
./mirai run agent.yaml  # Uses Claude by default
```

#### `MIRAI_LLM_MODEL`

**Type:** String  
**Default:** Provider-specific (e.g., `qwen3:8b` for ollama, `claude-sonnet-4-20250514` for Claude, `gpt-4o` for OpenAI)  
**Purpose:** Default model name if not specified via `--model` flag.

**Example:**
```bash
export MIRAI_LLM_MODEL=gpt-4o
./mirai run agent.yaml  # Uses GPT-4o by default
```

#### `MIRAI_API_KEY`

**Type:** String  
**Default:** None (unauthenticated server if omitted)  
**Purpose:** API key for the HTTP server (`mirai serve`). Clients must include this in the `X-API-Key` header. Note: `/health` and `/version` endpoints bypass authentication.

**Example:**
```bash
export MIRAI_API_KEY=$(openssl rand -hex 32)
./mirai serve --port 3000
# Health check (no key required):
curl http://localhost:3000/health

# Protected endpoints require the key:
curl -H "X-API-Key: $MIRAI_API_KEY" http://localhost:3000/api/v1/agents
```

#### `MIRAI_SCRATCH_DIR`

**Type:** File path  
**Default:** None (tools receive no scratch directory if omitted)  
**Purpose:** Temporary directory injected into bash tool execution. Available to bash scripts via the `$MIRAI_SCRATCH_DIR` environment variable.

**Example:**
```bash
export MIRAI_SCRATCH_DIR=/tmp/agent-workspace
./mirai run agent.yaml  # Bash tool can write to $MIRAI_SCRATCH_DIR
```

#### `MIRAI_BENCHMARK`

**Type:** String (`1` or empty)  
**Default:** None (disabled)  
**Purpose:** Enable benchmarking mode (`mirai run` with `--benchmark` flag or this env var). Prints timing statistics.

**Example:**
```bash
export MIRAI_BENCHMARK=1
./mirai run agent.yaml  # Prints benchmark timings
```

#### `MIRAI_BENCHMARK_FILE`

**Type:** File path  
**Default:** None  
**Purpose:** If set, benchmarking results are written to this file (JSON format).

**Example:**
```bash
export MIRAI_BENCHMARK=1
export MIRAI_BENCHMARK_FILE=benchmarks.json
./mirai run agent.yaml  # Results saved to benchmarks.json
```

#### `MIRAI_<TOOL_INPUT>` (Tool Input Injection)

**Type:** String (any tool input name)  
**Default:** None  
**Purpose:** When executing a bash tool, any non-`command` inputs from the tool's `data_map` are injected as `MIRAI_<key>` environment variables in the bash process.

**Example:**
Agent bash tool with input `{"command": "...", "api_endpoint": "https://api.example.com"}` → bash process receives `MIRAI_api_endpoint=https://api.example.com`.

---

### Provider-Specific API Keys

These are **not** MIRAI_* variables but are still required for their respective providers. The CLI auto-detects them by provider name.

| Provider | Environment Variable | Example |
|----------|----------------------|---------|
| OpenAI | `OPENAI_API_KEY` | `sk-...` |
| Anthropic/Claude | `ANTHROPIC_API_KEY` | `sk-ant-...` |
| Google Gemini | `GOOGLE_API_KEY` | (from Google Cloud Console) |
| Groq | `GROQ_API_KEY` | (from Groq Console) |
| NVIDIA NIM | `NVIDIA_API_KEY` | (from NVIDIA NGC) |
| OpenRouter | `OPENROUTER_API_KEY` | (from OpenRouter) |

**Example:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
./mirai run agent.yaml --provider claude --model claude-sonnet-4-20250514
```

---

### Ollama

#### `OLLAMA_BASE_URL`

**Type:** URL  
**Default:** `http://localhost:11434`  
**Purpose:** Base URL for local Ollama server (used if provider is `ollama`).

**Example:**
```bash
export OLLAMA_BASE_URL=http://192.168.1.100:11434
./mirai run agent.yaml --provider ollama --model llama2
```

---

## Storage & Data

### SQLite (Bundled)

The engine bundles SQLite (no external database server needed). Agent state, memory, and logs are persisted to a local SQLite database.

- **Location:** Configured at runtime (default: in-memory for CLI, persistent for servers)
- **Tables:** Agents, graphs, execution logs, memory snapshots, webhooks
- **Compiled in:** No `sqlite3` binary or libraries required

### Scratch Directory

Tools (especially bash) can write temporary files to a scratch directory, provided via `MIRAI_SCRATCH_DIR` or context.

---

## Performance & Tuning

### Build Optimizations

```bash
# Release build with all optimizations
cargo build --release -j $(nproc)

# For size-optimized binaries
cargo build --release --strip
```

### Server Tuning

```bash
# Tune worker threads (defaults to available CPU cores)
./mirai serve --port 3000
```

The Tokio runtime auto-tunes based on CPU count and is suitable for most deployments.

---

## Troubleshooting

### "No API key configured"

If the HTTP server logs:
```
WARN: Set MIRAI_API_KEY or use --api-key to enable authentication.
```

The server is running without authentication. For production, set `MIRAI_API_KEY`:

```bash
export MIRAI_API_KEY=your-secret-key
./mirai serve --port 3000
```

### Build fails with "cannot find -lssl"

This means your system is missing OpenSSL. OpenMirai uses **rustls** (pure Rust), so you should not need OpenSSL. Check your `rustflags` or remove any `-lssl` linker flags from your build environment.

### LLM provider fails silently

Check that:
1. The provider name is correct (`ollama`, `openai`, `claude`, etc.)
2. The API key environment variable is set and valid
3. The base URL (if custom) is reachable
4. For `ollama`, the local server is running and accessible at `OLLAMA_BASE_URL`

### Tests fail on macOS

Some tests may timeout on slower hardware. Re-run with:
```bash
cargo test --workspace --all-features -- --test-threads=1
```

---

## Version Management

Version is stored in **one source of truth**:

- **`VERSION` file** — contains the semver tag (e.g., `0.6.0`)
- **`engine/Cargo.toml`** — `version` field must match
- **`cli/Cargo.toml`** — `version` field must match
- **HTTP server** — reads version at compile time via `env!("CARGO_PKG_VERSION")`

When bumping version, update all four and commit before tagging.

```bash
# Check current version
cat VERSION

# Bump (manually edit VERSION and Cargo.toml)
# Then verify
cargo build --release
./target/release/mirai --version
```

---

## References

- [CONTRIBUTING.md](../../CONTRIBUTING.md) — Development workflow
- [RELEASING.md](../../RELEASING.md) — Release process detail
- [ARCHITECTURE.md](../ARCHITECTURE.md) — System design
- [GitHub Releases](https://github.com/Data-Mirai/openmirai/releases) — Download binaries
- [GitHub Actions](https://github.com/Data-Mirai/openmirai/actions) — CI/CD status
