"""CLI — `datamirai serve` and `datamirai agent` commands."""

from __future__ import annotations

import argparse
import sys


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        prog="datamirai",
        description="Data Mirai Engine — Open source motor for agentic graphs",
    )
    subparsers = parser.add_subparsers(dest="command")

    # serve
    serve_parser = subparsers.add_parser("serve", help="Start the engine server")
    serve_parser.add_argument("--host", default="0.0.0.0", help="Bind host (default: 0.0.0.0)")
    serve_parser.add_argument("--port", type=int, default=8000, help="Bind port (default: 8000)")
    serve_parser.add_argument("--reload", action="store_true", help="Enable auto-reload (dev)")

    # version
    subparsers.add_parser("version", help="Show version")

    # db
    db_parser = subparsers.add_parser("db", help="Database management")
    db_sub = db_parser.add_subparsers(dest="db_command")
    db_init_parser = db_sub.add_parser("init", help="Initialize database schema")
    db_init_parser.add_argument("--dsn", help="PostgreSQL connection string (or set DATABASE_URL)")
    db_status_parser = db_sub.add_parser("status", help="Check database connection and schema")
    db_status_parser.add_argument(
        "--dsn", help="PostgreSQL connection string (or set DATABASE_URL)",
    )

    # agent
    agent_parser = subparsers.add_parser("agent", help="Manage agents")
    agent_sub = agent_parser.add_subparsers(dest="agent_command")

    # agent load
    load_parser = agent_sub.add_parser("load", help="Import agent from YAML file")
    load_parser.add_argument("file", help="Path to YAML agent definition")
    load_parser.add_argument("--server", default="http://localhost:8000", help="Server URL")

    # agent export
    export_parser = agent_sub.add_parser("export", help="Export agent to YAML")
    export_parser.add_argument("agent_id", help="Agent ID to export")
    export_parser.add_argument("--server", default="http://localhost:8000", help="Server URL")
    export_parser.add_argument("--format", choices=["yaml", "json"], default="yaml", help="Output format")

    # agent list
    list_parser = agent_sub.add_parser("list", help="List agents")
    list_parser.add_argument("--server", default="http://localhost:8000", help="Server URL")

    args = parser.parse_args(argv)

    if args.command == "serve":
        _serve(args.host, args.port, args.reload)
    elif args.command == "version":
        from datamirai_engine import __version__
        print(f"datamirai-engine v{__version__}")
    elif args.command == "db":
        if args.db_command == "init":
            _db_init(args.dsn)
        elif args.db_command == "status":
            _db_status(args.dsn)
        else:
            db_parser.print_help()
            sys.exit(1)
    elif args.command == "agent":
        if args.agent_command == "load":
            _agent_load(args.file, args.server)
        elif args.agent_command == "export":
            _agent_export(args.agent_id, args.server, args.format)
        elif args.agent_command == "list":
            _agent_list(args.server)
        else:
            agent_parser.print_help()
            sys.exit(1)
    else:
        parser.print_help()
        sys.exit(1)


def _db_init(dsn: str | None) -> None:
    import asyncio

    async def _run():
        from datamirai_engine.db.connection import close_pool, create_pool
        from datamirai_engine.db.migrations import get_schema_version, run_migrations

        try:
            pool = await create_pool(dsn)
        except (RuntimeError, ValueError, ConnectionError) as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        try:
            await run_migrations(pool)
            version = await get_schema_version(pool)
            print(f"Database initialized (schema v{version})")
        finally:
            await close_pool(pool)

    asyncio.run(_run())


def _db_status(dsn: str | None) -> None:
    import asyncio

    async def _run():
        from datamirai_engine.db.connection import close_pool, create_pool
        from datamirai_engine.db.migrations import get_schema_version, get_table_counts

        try:
            pool = await create_pool(dsn)
        except (RuntimeError, ValueError, ConnectionError) as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        try:
            version = await get_schema_version(pool)
            if version is None:
                print("Schema not initialized. Run: datamirai db init")
                return

            counts = await get_table_counts(pool)
            print(f"Database connected (schema v{version})")
            print(f"  graphs:   {counts.get('graphs', '?')}")
            print(f"  agents:   {counts.get('agents', '?')}")
            print(f"  sessions: {counts.get('sessions', '?')}")
        finally:
            await close_pool(pool)

    asyncio.run(_run())


def _serve(host: str, port: int, reload: bool) -> None:
    import uvicorn
    print(f"Starting Data Mirai Engine on {host}:{port}")
    uvicorn.run(
        "datamirai_engine.server.app:create_app",
        host=host,
        port=port,
        reload=reload,
        factory=True,
    )


def _agent_load(file_path: str, server: str) -> None:
    import urllib.error
    import urllib.request

    try:
        with open(file_path) as f:
            yaml_content = f.read()
    except FileNotFoundError:
        print(f"Error: file not found: {file_path}", file=sys.stderr)
        sys.exit(1)

    # Validate locally first
    from datamirai_engine.core.agent_spec import AgentSpec
    try:
        spec = AgentSpec.from_yaml(yaml_content)
    except Exception as e:
        print(f"Error: invalid YAML: {e}", file=sys.stderr)
        sys.exit(1)

    # POST to server
    req = urllib.request.Request(
        f"{server}/api/agents/import",
        data=yaml_content.encode("utf-8"),
        headers={"Content-Type": "application/x-yaml"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req) as resp:
            import json
            data = json.loads(resp.read())
            print(f"Agent imported: {data['name']} (id: {data['id']})")
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        print(f"Error {e.code}: {body}", file=sys.stderr)
        sys.exit(1)
    except urllib.error.URLError as e:
        print(f"Error: cannot connect to {server}: {e.reason}", file=sys.stderr)
        sys.exit(1)


def _agent_export(agent_id: str, server: str, fmt: str) -> None:
    import urllib.error
    import urllib.request

    accept = "application/x-yaml" if fmt == "yaml" else "application/json"
    req = urllib.request.Request(
        f"{server}/api/agents/{agent_id}/spec",
        headers={"Accept": accept},
    )
    try:
        with urllib.request.urlopen(req) as resp:
            print(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        print(f"Error {e.code}: {body}", file=sys.stderr)
        sys.exit(1)
    except urllib.error.URLError as e:
        print(f"Error: cannot connect to {server}: {e.reason}", file=sys.stderr)
        sys.exit(1)


def _agent_list(server: str) -> None:
    import json
    import urllib.error
    import urllib.request

    req = urllib.request.Request(f"{server}/api/agents")
    try:
        with urllib.request.urlopen(req) as resp:
            agents = json.loads(resp.read())
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        print(f"Error {e.code}: {body}", file=sys.stderr)
        sys.exit(1)
    except urllib.error.URLError as e:
        print(f"Error: cannot connect to {server}: {e.reason}", file=sys.stderr)
        sys.exit(1)

    if not agents:
        print("No agents found.")
        return

    # Table output
    print(f"{'ID':<12} {'NAME':<30} {'STATUS':<12} {'SESSIONS':<10}")
    print("-" * 64)
    for a in agents:
        print(f"{a['id']:<12} {a['name']:<30} {a['status']:<12} {a['sessions_total']:<10}")
