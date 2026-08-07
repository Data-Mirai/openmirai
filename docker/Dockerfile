# syntax=docker/dockerfile:1.7
#
# OpenMirai — imagen del motor (prototipo de dockerización para la nube).
#
# Tres etapas:
#   builder → compila el binario `mirai` (release)
#   runtime → imagen final que se despliega: solo el binario + lo mínimo
#   tester  → imagen de e2e que golpea la API HTTP del runtime (docker/smoke.sh)
#
# Contexto de build: la RAÍZ del repo (no docker/).
#   docker build -f docker/Dockerfile -t openmirai/engine:0.6.0 .

ARG RUST_VERSION=1
ARG DEBIAN_RELEASE=bookworm

# ---------------------------------------------------------------------------
# builder
# ---------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_RELEASE} AS builder

WORKDIR /src

# No instalamos nada: `rust:slim` ya trae gcc + libc6-dev, que es todo lo que
# necesita `rusqlite` con feature `bundled` para compilar sqlite3.c.
# reqwest usa rustls (no OpenSSL), así que tampoco hace falta pkg-config.

COPY Cargo.toml Cargo.lock ./
COPY engine/Cargo.toml engine/Cargo.toml
COPY cli/Cargo.toml cli/Cargo.toml
COPY engine/src engine/src
COPY cli/src cli/src

# Cache mounts de BuildKit: el registry y target/ sobreviven entre builds sin
# quedar en la capa. Por eso el binario se copia a /out DENTRO del mismo RUN
# (al terminar, el mount de target/ desaparece).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --bin mirai \
    && install -Dm755 target/release/mirai /out/mirai \
    && /out/mirai version

# ---------------------------------------------------------------------------
# runtime — la imagen que se despliega
# ---------------------------------------------------------------------------
FROM debian:${DEBIAN_RELEASE}-slim AS runtime

# ca-certificates → TLS hacia las APIs de LLM (Claude, Gemini, OpenAI…)
# curl           → HEALTHCHECK del contenedor
# git            → herramientas git/* del catálogo (status, diff, log, commit)
# tmux           → orquestador de sesiones de 0.7.0 (/api/v1/orchestrator/*).
#                  Sin tmux el motor arranca igual —initialize() solo avisa—
#                  pero esos endpoints fallan al spawnear.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git tmux \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out/mirai /usr/local/bin/mirai
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Usuario sin privilegios. El motor expone system/bash y filesystem/*: correr
# como root le daría a cualquier YAML control total del contenedor.
RUN useradd --system --create-home --uid 10001 --shell /bin/bash mirai \
    && mkdir -p /data /opt/openmirai/flows \
    && chown -R mirai:mirai /data /opt/openmirai

# /data es el workdir: las rutas relativas de filesystem/write_file caen acá,
# que es el volumen persistente. /opt/openmirai/flows son los specs (read-only).
WORKDIR /data
USER mirai

ENV MIRAI_HOST=0.0.0.0 \
    MIRAI_PORT=3000 \
    MIRAI_FLOWS_DIR=/opt/openmirai/flows

EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=5 \
    CMD curl -fsS "http://127.0.0.1:${MIRAI_PORT}/health" || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["serve"]

# ---------------------------------------------------------------------------
# tester — corre la suite e2e contra el runtime (no se despliega)
# ---------------------------------------------------------------------------
FROM debian:${DEBIAN_RELEASE}-slim AS tester

# python3-yaml viene de apt (sin pip, sin red extra): convierte los flows YAML
# a JSON, que es lo que acepta POST /api/v1/agents/from-spec.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl jq python3 python3-yaml \
    && rm -rf /var/lib/apt/lists/*

COPY docker/smoke.sh /usr/local/bin/smoke.sh
COPY docker/persistencia.sh /usr/local/bin/persistencia.sh
COPY docker/flows /opt/openmirai/flows
RUN chmod +x /usr/local/bin/smoke.sh /usr/local/bin/persistencia.sh

ENV OPENMIRAI_URL=http://engine:3000 \
    MIRAI_FLOWS_DIR=/opt/openmirai/flows

ENTRYPOINT ["/usr/local/bin/smoke.sh"]
