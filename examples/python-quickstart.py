"""
Python Quick Start — Run an agent from Python in 10 lines.

Prerequisites:
    pip install openmirai
    mirai serve --port 3000  (in another terminal)

Run:
    python examples/python-quickstart.py
"""

from openmirai import Engine, Agent

# 1. Create engine (defaults to Ollama on localhost)
engine = Engine(provider="ollama")

# 2. Load agent from YAML
agent = Agent.from_file("examples/hello-world.yaml")

# 3. Run it
result = engine.run(agent, input={"query": "What makes Rust special?"})

# 4. Print the result
if result.succeeded:
    print("Agent output:", result.output)
else:
    print("Agent failed:", result.error)
