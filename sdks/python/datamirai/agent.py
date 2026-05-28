"""Agent — load and manage agent specifications."""

from pathlib import Path
from typing import Any


class Agent:
    """Represents an agent specification (graph + config).

    Agent specs are YAML-only for readability.
    """

    def __init__(self, spec: dict[str, Any]):
        self.spec = spec
        self.name = spec.get("name", "unnamed")

    @classmethod
    def from_file(cls, path: str) -> "Agent":
        """Load an agent from a YAML file (.yaml or .yml)."""
        p = Path(path)
        if p.suffix not in (".yaml", ".yml"):
            raise ValueError(
                f"Unsupported format: {p.suffix} — agent specs must be YAML (.yaml or .yml)"
            )
        content = p.read_text()
        try:
            import yaml
            spec = yaml.safe_load(content)
        except ImportError:
            raise ImportError("PyYAML is required: pip install pyyaml")

        return cls(spec)

    @classmethod
    def from_dict(cls, spec: dict[str, Any]) -> "Agent":
        """Create an agent from a dictionary."""
        return cls(spec)

    def __repr__(self) -> str:
        return f"Agent(name={self.name!r})"
