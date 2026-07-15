"""
openmirai — Python SDK for OpenMirai.

Thin wrapper that communicates with OpenMirai via HTTP API or CLI binary.

Usage:
    from openmirai import Engine, Agent

    engine = Engine(provider="openai", model="gpt-4")
    agent = Agent.from_file("my-agent.yaml")
    result = engine.run(agent, input={"query": "hello"})
    print(result.output)

    # Streaming
    for event in engine.stream(agent, input={"query": "hello"}):
        print(event.type, event.data)
"""

__version__ = "0.7.0"

from openmirai.engine import Engine
from openmirai.agent import Agent
from openmirai.types import ExecutionResult, StreamEvent

__all__ = ["Engine", "Agent", "ExecutionResult", "StreamEvent"]
