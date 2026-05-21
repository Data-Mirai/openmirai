"""Helper script to launch Mirai Code for E2E tests.

Usage: python _run_mirai_code.py --provider ollama --model qwen3:8b --cwd /tmp
"""
import argparse
import sys
import os

# Ensure framework src is importable
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "src"))

from datamirai_engine.cli.interactive_agent_terminal import start_interactive_terminal


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", default="ollama")
    parser.add_argument("--model", default="qwen3:8b")
    parser.add_argument("--cwd", default="/tmp")
    parser.add_argument("--autonomy", default="copilot")
    parser.add_argument("--base-url", default="")
    args = parser.parse_args()

    start_interactive_terminal(
        provider=args.provider,
        model=args.model,
        cwd=args.cwd,
        autonomy_level=args.autonomy,
        base_url=args.base_url,
    )


if __name__ == "__main__":
    main()
