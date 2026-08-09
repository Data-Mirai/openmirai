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

# VERSION y los build.rs no son opcionales: desde 0.7.0 los scripts de build de
# ambos crates emiten MIRAI_VERSION / MIRAI_BUILD / MIRAI_GIT_SHA / MIRAI_BUILD_TS,
# y el código las lee con env!(), que falla en compilación si no están. Sin los
# build.rs en el contexto, Cargo ni siquiera ejecuta los scripts y el build muere
# con "environment variable MIRAI_VERSION not defined at compile time".
# rust-toolchain.toml se copia para respetar el toolchain que fija el repo.
COPY Cargo.toml Cargo.lock rust-toolchain.toml VERSION ./
COPY engine/Cargo.toml engine/build.rs engine/
COPY cli/Cargo.toml cli/build.rs cli/
COPY engine/src engine/src
COPY cli/src cli/src

# Identificación del build. El sha queda en "unknown": .git no viaja al contexto
# —pesa y no hace falta para compilar— y el build.rs lo resuelve ejecutando git.
# Para trazar una imagen concreta, pasá MIRAI_BUILD_NUMBER.
ARG MIRAI_BUILD_NUMBER=docker
ARG MIRAI_BUILD_TS=unknown
ENV MIRAI_BUILD_NUMBER=${MIRAI_BUILD_NUMBER} \
    MIRAI_BUILD_TS=${MIRAI_BUILD_TS}

# Cache mounts de BuildKit: el registry y target/ sobreviven entre builds sin
# quedar en la capa. Por eso el binario se copia a /out DENTRO del mismo RUN
# (al terminar, el mount de target/ desaparece).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --bin mirai \
    && install -Dm755 target/release/mirai /out/mirai \
    && /out/mirai version

# ---------------------------------------------------------------------------
# runtime — perfil `restricted` del contrato de runtime
#
# docs/infra/CONTAINER-RUNTIME-CONTRACT.md define dos perfiles. Este es el
# acotado: motor, CA certificates y poco más. El contrato pide que `restricted`
# no traiga shell ni `ps`; debian-slim siempre trae `sh`, así que cumplirlo del
# todo pide una base distroless — anotado como divergencia en docker/README.md.
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

# El contrato ubica el binario y los archivos inmutables en /app, que con
# `read_only: true` queda de solo lectura sin permisos especiales.
COPY --from=builder /out/mirai /app/mirai
COPY docker/entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/mirai /app/entrypoint.sh \
    && ln -s /app/mirai /usr/local/bin/mirai

# Usuario sin privilegios. El motor expone system/bash y filesystem/*: correr
# como root le daría a cualquier YAML control total del contenedor.
#
# Los directorios se crean con dueño `mirai` para que, al montar volúmenes
# vacíos encima, Docker herede ese dueño y el proceso pueda escribir.
RUN useradd --system --create-home --uid 10001 --shell /bin/bash mirai \
    && mkdir -p /data/openmirai /workspace /opt/openmirai/flows \
    && chown -R mirai:mirai /data /workspace /opt/openmirai /home/mirai

# /workspace es el árbol de trabajo del contrato; /data/openmirai guarda la base.
WORKDIR /workspace
USER mirai

ENV MIRAI_HOST=0.0.0.0 \
    MIRAI_PORT=3000 \
    MIRAI_DB_PATH=/data/openmirai/engine.db \
    MIRAI_FLOWS_DIR=/opt/openmirai/flows \
    HOME=/home/mirai \
    TMPDIR=/tmp

EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=5 \
    CMD curl -fsS "http://127.0.0.1:${MIRAI_PORT}/health" || exit 1

ENTRYPOINT ["/app/entrypoint.sh"]
CMD ["serve"]

# ---------------------------------------------------------------------------
# runtime-full — perfil `full-trusted`, el que se despliega primero
#
# El contrato lo describe como la imagen inicial: shell y `ps`, Git y SSH,
# Python y Node, navegador headless, tmux y Claude CLI, más clientes MCP por
# stdio (que salen de npx). Pesa ~1 GB contra los ~200 MB del acotado; es el
# precio de validar estabilidad antes que tamaño.
# ---------------------------------------------------------------------------
FROM runtime AS runtime-full

USER root

# procps          → `ps`, que necesita la herramienta system/process_list
# openssh-client  → clonar y empujar por SSH con credenciales de repositorio
# python3 / nodejs → runtimes del perfil; npx habilita los MCP por stdio
# chromium        → navegador headless
# fonts-liberation → sin fuentes, el navegador renderiza cuadrados
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        procps openssh-client python3 nodejs npm chromium fonts-liberation \
    && rm -rf /var/lib/apt/lists/*

# Claude CLI. Va con ARG para poder clavar la versión una vez que la primera
# build registre cuál resolvió, y para poder saltearlo si el registro de npm
# no está disponible en el entorno de build.
ARG INSTALL_CLAUDE_CLI=true
ARG CLAUDE_CLI_VERSION=latest
RUN if [ "$INSTALL_CLAUDE_CLI" = "true" ]; then \
        npm install -g "@anthropic-ai/claude-code@${CLAUDE_CLI_VERSION}" \
        && npm cache clean --force; \
    else \
        echo "Claude CLI omitido por INSTALL_CLAUDE_CLI=false"; \
    fi

USER mirai

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
