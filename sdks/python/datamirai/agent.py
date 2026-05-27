"""Agent — load and manage agent specifications."""

import json
from pathlib import Path
from typing import Any, Optional


class Agent:
    """Represents an agent specification (graph + config)."""

    def __init__(self, spec: dict[str, Any]):
        self.spec = spec
        self.name = spec.get("name", "unnamed")

    @classmethod
    def from_file(cls, path: str) -> "Agent":
        """Load an agent from a JSON or YAML file."""
        p = Path(path)
        content = p.read_text()

        if p.suffix in (".yaml", ".yml"):
            try:
                import yaml
                spec = yaml.safe_load(content)
            except ImportError:
                raise ImportError("PyYAML is required for YAML files: pip install pyyaml")
        elif p.suffix == ".json":
            spec = json.loads(content)
        else:
            raise ValueError(f"Unsupported file format: {p.suffix}")

        return cls(spec)

    @classmethod
    def from_dict(cls, spec: dict[str, Any]) -> "Agent":
        """Create an agent from a dictionary."""
        return cls(spec)

    @classmethod
    def from_template(cls, template_id: str, **overrides: Any) -> "Agent":
        """Create an agent from a built-in template.

        Requires the engine server to be running.
        """
        # This would call the engine API to get the template.
        # For now, return a placeholder.
        spec = {"name": template_id, "template": template_id, **overrides}
        return cls(spec)

    def to_json(self) -> str:
        """Serialize the agent spec to JSON."""
        return json.dumps(self.spec, indent=2)

    def __repr__(self) -> str:
        return f"Agent(name={self.name!r})"
