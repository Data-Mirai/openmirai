"""Centralized enumerations — single source of truth for all string constants.

Use StrEnum so values are strings and work seamlessly with existing code:
  ToolType.MANUAL == "trigger/manual"  # True
  f"type is {ToolType.MANUAL}"         # "type is trigger/manual"
"""

from enum import StrEnum


class ToolType(StrEnum):
    """All registered tool types in the engine."""

    # Triggers
    MANUAL = "trigger/manual"
    WEBHOOK = "trigger/webhook"
    SCHEDULE = "trigger/schedule"
    EVENT = "trigger/event"
    HEARTBEAT = "trigger/heartbeat"

    # AI
    LLM_CALL = "ai/llm_call"
    TRANSCRIBE = "ai/transcribe"
    EMBEDDINGS = "ai/embeddings"

    # Data
    WEB_SCRAPE = "data/web_scrape"
    DB_READ = "data/db_read"
    DB_WRITE = "data/db_write"
    STORAGE_READ = "data/storage_read"
    STORAGE_WRITE = "data/storage_write"
    VAULT_READ = "data/vault_read"
    VAULT_WRITE = "data/vault_write"
    HTML_TO_MARKDOWN = "data/html_to_markdown"
    BROWSER_AGENT = "data/browser_agent"
    STEALTH = "data/stealth"

    # Logic
    CONDITION = "logic/condition"
    SWITCH = "logic/switch"
    LOOP = "logic/loop"
    MERGE = "logic/merge"
    WAIT = "logic/wait"
    HUMAN_INPUT = "logic/human_input"

    # Output
    RESPONSE = "output/response"

    # Agent
    RUN_AGENT = "agent/run_agent"

    # MCP
    MCP_CALL = "mcp/call"

    # Entity Tracking
    ENTITY_UPSERT = "data/entity_upsert"
    ENTITY_QUERY = "data/entity_query"

    # Deadline
    DEADLINE = "logic/deadline"


class ToolCategory(StrEnum):
    """Tool categories for grouping in the UI."""

    TRIGGER = "trigger"
    AI = "ai"
    DATA = "data"
    LOGIC = "logic"
    OUTPUT = "output"
    AGENT = "agent"
    MCP = "mcp"


class AgentType(StrEnum):
    """Agent execution model."""

    MANAGED = "managed"
    LIVE = "live"


class ResponseFormat(StrEnum):
    """Output/response node format options."""

    MARKDOWN = "markdown"
    TEXT = "text"
    JSON = "json"
    BULLETS = "bullets"
    REPORT = "report"
    HTML = "html"
    RICH_HTML = "rich_html"


class ScrapeMode(StrEnum):
    """Web scrape execution modes."""

    WINDOWLESS = "windowless"
    TUNNEL_VISION = "tunnel_vision"


class DBWriteMode(StrEnum):
    """Database write operation modes."""

    INSERT = "insert"
    UPSERT = "upsert"


class DBReadMode(StrEnum):
    """Database read operation modes."""

    ONE = "one"
    ALL = "all"


class ComparisonOp(StrEnum):
    """Comparison operators for edge conditions and logic/condition."""

    EQ = "eq"
    NEQ = "neq"
    GT = "gt"
    LT = "lt"
    GTE = "gte"
    LTE = "lte"
    IN = "in"
    CONTAINS = "contains"


class SessionStatus(StrEnum):
    """Session execution status."""

    PENDING = "pending"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    INTERRUPTED = "interrupted"


class AgentStatus(StrEnum):
    """Agent enable/disable status."""

    ENABLED = "enabled"
    DISABLED = "disabled"


class ConfigKey(StrEnum):
    """App configuration keys (app_config table)."""

    LLM_CHAT_MODEL = "llm_chat_model"
    LLM_EMBED_MODEL = "llm_embed_model"
    LLM_NUM_CTX = "llm_num_ctx"
    ANTHROPIC_API_KEY = "anthropic_api_key"
    OPENAI_API_KEY = "openai_api_key"


class LLMProvider(StrEnum):
    """Supported LLM providers."""

    OLLAMA = "ollama"
    ANTHROPIC = "claude"
    OPENAI = "openai"
    GEMINI = "gemini"
    GROQ = "groq"
    OPENROUTER = "openrouter"


class RenderTheme(StrEnum):
    """HTML render themes."""

    DEFAULT = "default"
    DARK = "dark"
    MINIMAL = "minimal"
    REPORT = "report"
