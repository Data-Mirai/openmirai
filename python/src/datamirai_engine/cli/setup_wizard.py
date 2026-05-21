"""setup_wizard -- interactive configuration when starting Mirai Code.

Detects available models, lets the user pick provider/model/autonomy level,
and returns a complete session configuration.
"""

from __future__ import annotations

import asyncio
import os
import platform
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Any


# ---------------------------------------------------------------------------
# ANSI colors (reuse pattern)
# ---------------------------------------------------------------------------

class _C:
    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"
    CYAN = "\033[36m"
    GREEN = "\033[32m"
    YELLOW = "\033[33m"
    RED = "\033[31m"
    MAGENTA = "\033[35m"
    BLUE = "\033[34m"


# ---------------------------------------------------------------------------
# Data models
# ---------------------------------------------------------------------------

@dataclass
class ModelOption:
    id: str
    name: str
    provider: str
    context_window: int | None = None
    supports_tools: bool = False
    supports_vision: bool = False
    local: bool = False

    @property
    def modality_label(self) -> str:
        if self.supports_vision:
            return "multimodal"
        return "text"

    @property
    def ctx_label(self) -> str:
        if not self.context_window:
            return "?"
        if self.context_window >= 1_000_000:
            return f"{self.context_window // 1_000_000}M"
        return f"{self.context_window // 1000}K"


# Vision model patterns (known multimodal models)
_VISION_PATTERNS = (
    "llava", "vision", "llama-3.2-vision", "bakllava",
    "moondream", "minicpm-v", "cogvlm", "gpt-4o", "gpt-4-turbo",
    "gemini", "claude-3", "pixtral",
)


AUTONOMY_LEVELS = {
    "assisted": {
        "name": "Assisted",
        "description": "Pregunta-respuesta. El humano lidera cada paso.",
        "max_tool_rounds": 1,
        "confirm_writes": True,
    },
    "copilot": {
        "name": "Copilot",
        "description": "Humano pide, agente ejecuta N pasos, humano revisa.",
        "max_tool_rounds": 25,
        "confirm_writes": False,
    },
    "autopilot": {
        "name": "Autopilot",
        "description": "Agente reacciona a eventos, humano supervisa.",
        "max_tool_rounds": 50,
        "confirm_writes": False,
    },
    "self_driving": {
        "name": "Self-Driving",
        "description": "Agente persigue objetivos, humano observa.",
        "max_tool_rounds": 100,
        "confirm_writes": False,
    },
}


@dataclass
class SessionConfig:
    provider: str
    model: str
    autonomy_level: str  # assisted | copilot | autopilot | self_driving
    max_tool_rounds: int
    confirm_writes: bool
    context_window: int | None = None
    supports_vision: bool = False
    temperature: float = 0.3
    max_tokens: int = 4096


# ---------------------------------------------------------------------------
# Model detection
# ---------------------------------------------------------------------------

async def detect_ollama_models(base_url: str = "http://localhost:11434") -> list[ModelOption]:
    """Detect models available in local Ollama instance."""
    try:
        import httpx
        async with httpx.AsyncClient(timeout=5.0) as client:
            resp = await client.get(f"{base_url}/api/tags")
            if resp.status_code != 200:
                return []
            data = resp.json()
    except Exception:
        return []

    models: list[ModelOption] = []
    for m in data.get("models", []):
        name = m.get("name", "")
        details = m.get("details", {})
        param_size = details.get("parameter_size", "")

        # Try to get context window
        ctx = None
        try:
            import httpx as _hx
            async with _hx.AsyncClient(timeout=3.0) as c:
                show = await c.post(f"{base_url}/api/show", json={"name": name})
                if show.status_code == 200:
                    info = show.json().get("model_info", {})
                    for k, v in info.items():
                        if k.endswith(".context_length") and isinstance(v, int):
                            ctx = v
                            break
        except Exception:
            pass

        is_vision = any(p in name.lower() for p in _VISION_PATTERNS)

        display = name
        if param_size:
            display = f"{name} ({param_size})"

        models.append(ModelOption(
            id=name,
            name=display,
            provider="ollama",
            context_window=ctx,
            supports_tools=True,
            supports_vision=is_vision,
            local=True,
        ))

    return models


def detect_remote_providers() -> list[dict[str, Any]]:
    """Detect which remote providers have API keys configured."""
    providers: list[dict[str, Any]] = []

    if os.environ.get("GROQ_API_KEY"):
        providers.append({
            "provider": "groq",
            "label": "Groq (cloud, free tier)",
            "default_model": "qwen-qwq-32b",
        })
    if os.environ.get("NVIDIA_API_KEY"):
        providers.append({
            "provider": "nvidia",
            "label": "NVIDIA NIM (cloud, free tier)",
            "default_model": "meta/llama-3.3-70b-instruct",
        })
    if os.environ.get("OPENAI_API_KEY"):
        providers.append({
            "provider": "openai",
            "label": "OpenAI (cloud, paid)",
            "default_model": "gpt-4o",
        })
    if os.environ.get("OPENROUTER_API_KEY"):
        providers.append({
            "provider": "openrouter",
            "label": "OpenRouter (cloud, multi-model)",
            "default_model": "meta-llama/llama-3.3-70b-instruct",
        })

    return providers


# ---------------------------------------------------------------------------
# User input helpers -- arrow key selection
# ---------------------------------------------------------------------------

def _ask_select(prompt: str, choices: list[dict[str, str]], default: str = "") -> str:
    """Interactive arrow-key selector. Returns the 'value' of the chosen item.

    Each choice: {"name": "display text", "value": "return_value"}
    """
    try:
        import questionary
        from questionary import Style

        style = Style([
            ("qmark", "fg:magenta bold"),
            ("question", "bold"),
            ("pointer", "fg:cyan bold"),
            ("highlighted", "fg:cyan bold"),
            ("selected", "fg:green"),
            ("answer", "fg:green bold"),
        ])

        result = questionary.select(
            prompt,
            choices=[questionary.Choice(title=c["name"], value=c["value"]) for c in choices],
            default=default or choices[0]["value"],
            style=style,
            qmark="  ->",
            instruction="(flechas para navegar, Enter para seleccionar)",
        ).ask()

        if result is None:
            sys.exit(0)
        return result

    except ImportError:
        # Fallback: numbered input if questionary not installed
        print()
        for i, c in enumerate(choices):
            print(f"    {_C.BOLD}{i + 1}.{_C.RESET} {c['name']}")
        print()
        while True:
            try:
                raw = input(f"  {prompt} [1]: ").strip()
            except (KeyboardInterrupt, EOFError):
                print()
                sys.exit(0)
            if not raw:
                return choices[0]["value"]
            try:
                idx = int(raw) - 1
                if 0 <= idx < len(choices):
                    return choices[idx]["value"]
            except ValueError:
                pass
            print(f"  {_C.YELLOW}Pick 1-{len(choices)}{_C.RESET}")


# ---------------------------------------------------------------------------
# Ollama health check (explicit error reporting for the wizard)
# ---------------------------------------------------------------------------

async def _ollama_health(base_url: str = "http://localhost:11434") -> str:
    """Quick Ollama status check.

    Returns ``"ok"`` | ``"not_running"`` | ``"no_models"`` | ``"error:<detail>"``.
    """
    try:
        import httpx
    except ImportError:
        return "error:httpx no instalado (pip install httpx)"
    try:
        async with httpx.AsyncClient(timeout=5.0) as client:
            resp = await client.get(f"{base_url}/api/tags")
        if resp.status_code != 200:
            return f"error:Ollama respondio status {resp.status_code}"
        models = resp.json().get("models", [])
        return "ok" if models else "no_models"
    except httpx.ConnectError:
        return "not_running"
    except httpx.TimeoutException:
        return "error:timeout conectando a Ollama"
    except Exception as exc:
        return f"error:{exc}"


# ---------------------------------------------------------------------------
# Ollama system helpers (install, start, pull)
# ---------------------------------------------------------------------------

_SPINNER = ["*", "o", "O", "@", "*"]


def _is_ollama_installed() -> bool:
    """Check if the ``ollama`` binary is available on PATH."""
    return shutil.which("ollama") is not None


def _try_start_ollama() -> str:
    """Try to start Ollama server in background.

    Returns health status after attempting start.
    """
    try:
        subprocess.Popen(
            ["ollama", "serve"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    except Exception:
        return "not_running"

    # Poll health check -- Ollama needs a moment to bind the port
    for i in range(12):
        time.sleep(0.5)
        frame = _SPINNER[i % len(_SPINNER)]
        sys.stdout.write(
            f"\r  {_C.MAGENTA}{frame}{_C.RESET} "
            f"{_C.DIM}Iniciando Ollama... ({(i + 1) * 0.5:.0f}s){_C.RESET}  "
        )
        sys.stdout.flush()
        status = asyncio.run(_ollama_health())
        if status in ("ok", "no_models"):
            sys.stdout.write("\r" + " " * 50 + "\r")
            sys.stdout.flush()
            return status
    sys.stdout.write("\r" + " " * 50 + "\r")
    sys.stdout.flush()
    return "not_running"


def _install_ollama() -> bool:
    """Attempt to install Ollama. Returns True on success."""
    system = platform.system()

    if system == "Darwin":
        if shutil.which("brew"):
            print(f"\n  {_C.DIM}Ejecutando: brew install ollama{_C.RESET}\n")
            result = subprocess.run(["brew", "install", "ollama"], check=False)
            return result.returncode == 0
        print(f"\n  {_C.YELLOW}Homebrew no encontrado.{_C.RESET}")
        print(f"  {_C.DIM}Descarga Ollama desde: https://ollama.com/download{_C.RESET}")
        return False

    if system == "Linux":
        print(f"\n  {_C.DIM}Ejecutando instalador de Ollama...{_C.RESET}\n")
        result = subprocess.run(
            ["sh", "-c", "curl -fsSL https://ollama.com/install.sh | sh"],
            check=False,
        )
        return result.returncode == 0

    print(f"\n  {_C.YELLOW}Descarga Ollama desde: https://ollama.com/download{_C.RESET}")
    return False


def _pull_ollama_model(model_name: str) -> bool:
    """Pull an Ollama model. Shows download progress. Returns True on success."""
    print(f"\n  {_C.DIM}Descargando {model_name}...{_C.RESET}\n")
    try:
        result = subprocess.run(["ollama", "pull", model_name], check=False)
        return result.returncode == 0
    except Exception:
        return False


# Suggested models for first-time setup (when Ollama has no models)
_SUGGESTED_MODELS = [
    {"name": "qwen3:8b -- 8B, rapido, recomendado (~5 GB)", "value": "qwen3:8b"},
    {"name": "llama3.2 -- 3B, ligero (~2 GB)", "value": "llama3.2"},
    {"name": "gemma4 -- 8B, Google (~5 GB)", "value": "gemma4"},
    {"name": "deepseek-r1:8b -- 8B, razonamiento (~5 GB)", "value": "deepseek-r1:8b"},
]


# ---------------------------------------------------------------------------
# Provider setup -- Local (Ollama)
# ---------------------------------------------------------------------------

def _setup_local_provider() -> ModelOption | None:
    """Guide user through local Ollama setup with graceful error handling.

    Progression: check installed -> check running -> check models -> pick model.
    Each failure state offers to fix the problem automatically.
    """
    while True:
        # --- Check 1: Is Ollama installed? ---
        if not _is_ollama_installed():
            print(f"\n  {_C.RED}x Ollama no esta instalado{_C.RESET}")
            system = platform.system()

            choices: list[dict[str, str]] = []
            if system == "Darwin" and shutil.which("brew"):
                print(f"  {_C.DIM}Se puede instalar con Homebrew{_C.RESET}")
                choices.append({"name": "Instalar Ollama ahora (brew install ollama)", "value": "install"})
            elif system == "Linux":
                print(f"  {_C.DIM}Se puede instalar automaticamente{_C.RESET}")
                choices.append({"name": "Instalar Ollama ahora", "value": "install"})
            else:
                print(f"  {_C.DIM}Descarga: https://ollama.com/download{_C.RESET}")
            choices.extend([
                {"name": "Reintentar (ya lo instale)", "value": "retry"},
                {"name": "Cambiar a Cloud", "value": "cloud"},
                {"name": "Salir", "value": "exit"},
            ])

            action = _ask_select("Siguiente paso", choices)
            if action == "install":
                if _install_ollama():
                    print(f"\n  {_C.GREEN}+ Ollama instalado{_C.RESET}")
                else:
                    print(f"\n  {_C.RED}x Error instalando Ollama{_C.RESET}")
                continue
            if action == "retry":
                continue
            if action == "cloud":
                return _setup_cloud_provider()
            return None

        # --- Check 2: Is Ollama running? ---
        print(f"\n  {_C.DIM}Conectando con Ollama...{_C.RESET}")
        status = asyncio.run(_ollama_health())

        if status == "not_running":
            print(f"  {_C.YELLOW}Ollama instalado pero no esta corriendo{_C.RESET}")
            status = _try_start_ollama()
            if status in ("ok", "no_models"):
                print(f"  {_C.GREEN}+ Ollama iniciado{_C.RESET}")
            else:
                print(f"  {_C.RED}x No se pudo iniciar Ollama{_C.RESET}")
                print(f"  {_C.DIM}Inicialo manualmente: ollama serve{_C.RESET}")
                action = _ask_select("Siguiente paso", [
                    {"name": "Reintentar", "value": "retry"},
                    {"name": "Cambiar a Cloud", "value": "cloud"},
                    {"name": "Salir", "value": "exit"},
                ])
                if action == "retry":
                    continue
                if action == "cloud":
                    return _setup_cloud_provider()
                return None

        # --- Check 3: Does Ollama have models? ---
        if status == "ok":
            models = asyncio.run(_detect_ollama_safe())
            if models:
                n = len(models)
                print(f"  {_C.GREEN}+{_C.RESET} Ollama conectado -- {n} modelo{'s' if n != 1 else ''}")
                return _pick_model(models)
            status = "no_models"

        if status == "no_models":
            print(f"  {_C.GREEN}+{_C.RESET} Ollama conectado")
            print(f"  {_C.YELLOW}No hay modelos descargados{_C.RESET}")

            download_choices: list[dict[str, str]] = list(_SUGGESTED_MODELS) + [
                {"name": "Cambiar a Cloud", "value": "__cloud__"},
                {"name": "Salir", "value": "__exit__"},
            ]
            chosen = _ask_select("Descargar un modelo", download_choices)

            if chosen == "__cloud__":
                return _setup_cloud_provider()
            if chosen == "__exit__":
                return None

            if _pull_ollama_model(chosen):
                print(f"\n  {_C.GREEN}+ {chosen} listo{_C.RESET}")
            else:
                print(f"\n  {_C.RED}x Error descargando {chosen}{_C.RESET}")
            continue  # Re-detect models either way

        # --- Other error ---
        if status.startswith("error:"):
            detail = status.removeprefix("error:")
            print(f"\n  {_C.RED}x {detail}{_C.RESET}")
            action = _ask_select("Siguiente paso", [
                {"name": "Reintentar", "value": "retry"},
                {"name": "Cambiar a Cloud", "value": "cloud"},
                {"name": "Salir", "value": "exit"},
            ])
            if action == "retry":
                continue
            if action == "cloud":
                return _setup_cloud_provider()
            return None


def _pick_model(models: list[ModelOption]) -> ModelOption:
    """Let user pick from a list of detected models."""
    if len(models) == 1:
        print(f"  {_C.DIM}Modelo: {models[0].name}{_C.RESET}")
        return models[0]

    choices: list[dict[str, str]] = []
    for m in models:
        loc = "local" if m.local else "cloud"
        label = f"{m.name}  -- {loc} | {m.modality_label} | {m.ctx_label} ctx"
        choices.append({"name": label, "value": m.id})

    selected_id = _ask_select("Modelo", choices)
    return next(m for m in models if m.id == selected_id)


# ---------------------------------------------------------------------------
# Provider setup -- Cloud
# ---------------------------------------------------------------------------

def _setup_cloud_provider() -> ModelOption | None:
    """Guide user through cloud provider setup. Returns selected model or None."""
    while True:
        print(f"\n  {_C.DIM}Detectando API keys...{_C.RESET}")
        providers = detect_remote_providers()

        if not providers:
            print(f"\n  {_C.RED}x No hay API keys configuradas{_C.RESET}")
            print(f"  {_C.DIM}Configura al menos una:{_C.RESET}")
            print(f"    {_C.DIM}export GROQ_API_KEY=gsk_...{_C.RESET}          {_C.GREEN}(gratis){_C.RESET}")
            print(f"    {_C.DIM}export NVIDIA_API_KEY=nvapi-...{_C.RESET}      {_C.GREEN}(gratis){_C.RESET}")
            print(f"    {_C.DIM}export OPENAI_API_KEY=sk-...{_C.RESET}         {_C.YELLOW}(pago){_C.RESET}")
            print(f"    {_C.DIM}export OPENROUTER_API_KEY=sk-or-...{_C.RESET}  {_C.YELLOW}(multi-modelo){_C.RESET}")

            action = _ask_select("Siguiente paso", [
                {"name": "Reintentar (despues de exportar la key)", "value": "retry"},
                {"name": "Cambiar a Local (Ollama)", "value": "local"},
                {"name": "Salir", "value": "exit"},
            ])
            if action == "retry":
                continue
            if action == "local":
                return _setup_local_provider()
            return None

        if len(providers) == 1:
            chosen = providers[0]
            print(f"  {_C.GREEN}+{_C.RESET} {chosen['label']}")
        else:
            print(f"  {_C.GREEN}+{_C.RESET} {len(providers)} proveedores disponibles")
            provider_choices = [
                {"name": p["label"], "value": p["provider"]} for p in providers
            ]
            chosen_key = _ask_select("Proveedor", provider_choices)
            chosen = next(p for p in providers if p["provider"] == chosen_key)

        is_vision = any(p in chosen["default_model"].lower() for p in _VISION_PATTERNS)
        return ModelOption(
            id=chosen["default_model"],
            name=chosen["label"],
            provider=chosen["provider"],
            context_window=128_000,
            supports_tools=True,
            supports_vision=is_vision,
            local=False,
        )


# ---------------------------------------------------------------------------
# The wizard
# ---------------------------------------------------------------------------

def run_setup_wizard(
    skip_wizard: bool = False,
    provider: str = "",
    model: str = "",
    autonomy: str = "",
) -> SessionConfig:
    """Run the interactive setup wizard. Returns a SessionConfig.

    If skip_wizard=True or all params provided, skips interactive prompts.
    """
    # Quick path: all params provided
    if skip_wizard or (provider and model and autonomy):
        level = AUTONOMY_LEVELS.get(autonomy or "copilot", AUTONOMY_LEVELS["copilot"])
        return SessionConfig(
            provider=provider or "ollama",
            model=model or "qwen3:8b",
            autonomy_level=autonomy or "copilot",
            max_tool_rounds=level["max_tool_rounds"],
            confirm_writes=level["confirm_writes"],
        )

    print(f"\n  {_C.BOLD}{_C.MAGENTA}Mirai Code -- Setup{_C.RESET}\n")

    # --- Step 1: Local or Cloud? ---
    mode = _ask_select("Donde correra el modelo?", [
        {"name": "Local (Ollama -- corre en tu maquina)", "value": "local"},
        {"name": "Cloud (necesita API key)", "value": "cloud"},
    ])

    # --- Step 2: Provider + Model ---
    selected = _setup_local_provider() if mode == "local" else _setup_cloud_provider()
    if selected is None:
        sys.exit(0)

    final_provider = provider or selected.provider
    final_model = model or selected.id

    # --- Step 3: Context window ---
    default_ctx = selected.context_window or 4096

    def _ctx_label(n: int) -> str:
        return f"{n // 1000}K" if n < 1_000_000 else f"{n // 1_000_000}M"

    ctx_choices: list[dict[str, str]] = [
        {"name": f"Default ({_ctx_label(default_ctx)} -- capacidad del modelo)", "value": str(default_ctx)},
        {"name": "Small (4K -- ahorra memoria)", "value": "4096"},
        {"name": "Medium (8K)", "value": "8192"},
        {"name": "Standard (16K)", "value": "16384"},
        {"name": "Large (32K)", "value": "32768"},
        {"name": "Extra Large (64K)", "value": "65536"},
    ]
    seen: set[str] = set()
    unique_ctx: list[dict[str, str]] = []
    for c in ctx_choices:
        if c["value"] not in seen:
            seen.add(c["value"])
            unique_ctx.append(c)
    ctx_choices = unique_ctx

    selected_ctx_str = _ask_select("Ventana de contexto", ctx_choices, default=str(default_ctx))
    selected_ctx = int(selected_ctx_str)

    # --- Step 4: Autonomy level ---
    level_choices: list[dict[str, str]] = []
    for key in AUTONOMY_LEVELS:
        lvl = AUTONOMY_LEVELS[key]
        label = f"{lvl['name']}  -- {lvl['description']}"
        level_choices.append({"name": label, "value": key})

    selected_level_key = _ask_select("Autonomia", level_choices, default="copilot")
    selected_level = AUTONOMY_LEVELS[selected_level_key]

    return SessionConfig(
        provider=final_provider,
        model=final_model,
        autonomy_level=selected_level_key,
        max_tool_rounds=selected_level["max_tool_rounds"],
        confirm_writes=selected_level["confirm_writes"],
        context_window=selected_ctx,
        supports_vision=selected.supports_vision,
    )


async def _detect_ollama_safe() -> list[ModelOption]:
    """Detect Ollama models, return empty list on failure."""
    try:
        return await detect_ollama_models()
    except Exception:
        return []
