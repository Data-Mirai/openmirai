-- Data Mirai Engine — Database Schema
-- PostgreSQL 16+ with pgvector extension

CREATE EXTENSION IF NOT EXISTS "pgvector";
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ── Universe ─────────────────────────────────────────────────

CREATE TABLE universes (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name        TEXT NOT NULL,
    slug        TEXT UNIQUE NOT NULL,
    mode        TEXT NOT NULL DEFAULT 'self-hosted',  -- self-hosted | cloud
    config      JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ── Environments ─────────────────────────────────────────────

CREATE TABLE environments (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    universe_id  UUID NOT NULL REFERENCES universes(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    stack_order  INT NOT NULL DEFAULT 0,
    mode         TEXT NOT NULL DEFAULT 'self-hosted',  -- self-hosted | cloud
    config       JSONB NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(universe_id, name)
);

CREATE INDEX idx_environments_universe ON environments(universe_id);

-- ── Resources ────────────────────────────────────────────────

CREATE TABLE resources (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    environment_id  UUID NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    type            TEXT NOT NULL,  -- db | vector | storage | llm
    mode            TEXT NOT NULL DEFAULT 'self-hosted',  -- self-hosted | hybrid | cloud
    status          TEXT NOT NULL DEFAULT 'disconnected',  -- ok | error | degraded | disconnected
    host            TEXT,
    config          JSONB NOT NULL DEFAULT '{}',
    credentials     JSONB NOT NULL DEFAULT '{}',  -- encrypted in production
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(environment_id, name)
);

CREATE INDEX idx_resources_env ON resources(environment_id);

-- ── Graphs ───────────────────────────────────────────────────

CREATE TABLE graphs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    environment_id  UUID NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    version         TEXT NOT NULL DEFAULT '0.0.1',
    status          TEXT NOT NULL DEFAULT 'draft',  -- draft | published | deprecated
    nodes           JSONB NOT NULL DEFAULT '[]',
    edges           JSONB NOT NULL DEFAULT '[]',
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_by      UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_graphs_env ON graphs(environment_id);
CREATE INDEX idx_graphs_status ON graphs(environment_id, status);

-- ── Agents ───────────────────────────────────────────────────

CREATE TABLE agents (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    environment_id  UUID NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    graph_id        UUID NOT NULL REFERENCES graphs(id),
    name            TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'disabled',  -- enabled | disabled
    triggers        JSONB NOT NULL DEFAULT '[]',
    config          JSONB NOT NULL DEFAULT '{}',  -- max_iterations, retry policy, etc.
    created_by      UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(environment_id, name)
);

CREATE INDEX idx_agents_env ON agents(environment_id);
CREATE INDEX idx_agents_status ON agents(environment_id, status);

-- ── Sessions ─────────────────────────────────────────────────

CREATE TABLE sessions (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    agent_id    UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    graph_id    UUID NOT NULL REFERENCES graphs(id),
    status      TEXT NOT NULL DEFAULT 'pending',  -- pending | running | completed | failed | timeout
    trigger_type TEXT,  -- webhook | schedule | event | manual | agent_call
    trigger_data JSONB NOT NULL DEFAULT '{}',
    trace       JSONB NOT NULL DEFAULT '[]',
    state       JSONB NOT NULL DEFAULT '{}',
    error       TEXT,
    duration_ms FLOAT,
    started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);

CREATE INDEX idx_sessions_agent ON sessions(agent_id);
CREATE INDEX idx_sessions_status ON sessions(agent_id, status);
CREATE INDEX idx_sessions_started ON sessions(started_at DESC);

-- ── Memory: Long-term learnings ──────────────────────────────

CREATE TABLE agent_memory (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    agent_id    UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    session_id  UUID REFERENCES sessions(id),
    summary     TEXT NOT NULL,
    decisions   JSONB NOT NULL DEFAULT '[]',
    tags        TEXT[] NOT NULL DEFAULT '{}',
    embedding   vector(1536),  -- for semantic search
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_memory_agent ON agent_memory(agent_id);
CREATE INDEX idx_memory_embedding ON agent_memory USING hnsw (embedding vector_cosine_ops);

-- ── Memory: Shared log (bitacora) ────────────────────────────

CREATE TABLE shared_log (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    environment_id  UUID NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    agent_id        UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    session_id      UUID REFERENCES sessions(id),
    message         TEXT NOT NULL,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_shared_log_env ON shared_log(environment_id);
CREATE INDEX idx_shared_log_agent ON shared_log(agent_id);
CREATE INDEX idx_shared_log_created ON shared_log(created_at DESC);

-- ── Users & Roles ────────────────────────────────────────────

CREATE TABLE users (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    universe_id UUID NOT NULL REFERENCES universes(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    email       TEXT NOT NULL,
    avatar      TEXT,  -- initial letter or URL
    color       TEXT,  -- avatar background color
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(universe_id, email)
);

CREATE TABLE user_roles (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    environment_id  UUID NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,  -- OWNER | ADMIN | EDITOR | VIEWER
    UNIQUE(user_id, environment_id)
);

CREATE INDEX idx_user_roles_user ON user_roles(user_id);
CREATE INDEX idx_user_roles_env ON user_roles(environment_id);

-- ── Settings (key-value per universe) ────────────────────────

CREATE TABLE settings (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    universe_id UUID NOT NULL REFERENCES universes(id) ON DELETE CASCADE,
    group_name  TEXT NOT NULL,  -- universe | defaults | api | auth | distribution
    key         TEXT NOT NULL,
    value       TEXT,
    UNIQUE(universe_id, group_name, key)
);

CREATE INDEX idx_settings_universe ON settings(universe_id);

-- ── Seed: default universe + dev environment ─────────────────

INSERT INTO universes (id, name, slug, mode) VALUES
    ('00000000-0000-0000-0000-000000000001', 'Data Mirai Engine', 'datamirai-engine', 'self-hosted');

INSERT INTO environments (id, universe_id, name, stack_order, mode) VALUES
    ('00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000001', 'dev', 0, 'self-hosted'),
    ('00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-000000000001', 'staging', 1, 'self-hosted'),
    ('00000000-0000-0000-0000-000000000012', '00000000-0000-0000-0000-000000000001', 'prod', 2, 'cloud');

INSERT INTO settings (universe_id, group_name, key, value) VALUES
    ('00000000-0000-0000-0000-000000000001', 'defaults', 'retry.max_retries', '3'),
    ('00000000-0000-0000-0000-000000000001', 'defaults', 'retry.backoff', 'exponential'),
    ('00000000-0000-0000-0000-000000000001', 'defaults', 'max_iterations', '100'),
    ('00000000-0000-0000-0000-000000000001', 'defaults', 'timeout_ms', '300000'),
    ('00000000-0000-0000-0000-000000000001', 'api', 'endpoint_base', 'http://localhost:8000'),
    ('00000000-0000-0000-0000-000000000001', 'api', 'webhook_signing', 'HMAC-SHA256'),
    ('00000000-0000-0000-0000-000000000001', 'auth', 'mode', 'single-user'),
    ('00000000-0000-0000-0000-000000000001', 'auth', 'provider', 'none');
