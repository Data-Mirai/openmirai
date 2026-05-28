"""
datamirai — Python SDK for Mirai Engine.

Thin wrapper that communicates with the Mirai Engine via HTTP API or CLI binary.

Usage:
    from datamirai import Engine, Agent

    engine = Engine(provider="openai", model="gpt-4")
    agent = Agent.from_file("my-agent.yaml")
    result = engine.run(agent, input={"query": "hello"})
    print(result.output)

    # Streaming
    for event in engine.stream(agent, input={"query": "hello"}):
        print(event.type, event.data)
"""

__version__ = "0.4.0"

from datamirai.engine import Engine
from datamirai.agent import Agent
from datamirai.types import ExecutionResult, StreamEvent

__all__ = ["Engine", "Agent", "ExecutionResult", "StreamEvent"]
