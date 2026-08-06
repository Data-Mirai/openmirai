# Contributing to OpenMirai

Thanks for helping build OpenMirai — the open-source engine for **decentralized AI agents**. This guide gets you from clone to merged PR.

## TL;DR

```bash
git clone https://github.com/Gabo-TheCreator/openmirai
cd openmirai
cargo build --release          # builds engine + cli (binary: ./target/release/mirai)
cargo test --workspace         # run the full suite (727 tests)
./target/release/mirai run examples/hello-world.yaml
```

Before opening a PR, make sure these three pass (CI enforces them):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
```

## What OpenMirai is

A Rust-native engine that runs agentic workflows defined as **YAML graphs**, compiled to a single portable binary with zero runtime dependencies. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the system map before diving into a non-trivial change.

```
engine/        Core library (openmirai-engine crate)
cli/           CLI binary (mirai)
sdks/python/   Python SDK (openmirai)
sdks/typescript/  TypeScript SDK
examples/      Ready-to-run agent examples
docs/          Technical documentation
```

## Ways to contribute

- **New built-in tools** — add to `engine/src/tools/builtin/<category>/` (category = filesystem, data, logic, ai, system, git, output, agent, mcp, trigger, or state), following an existing tool and registering it in that category's `register_*` fn.
- **New LLM providers** — implement the adapter in `engine/src/llm/`, register in `cli/src/adapter_factory.rs`.
- **Example agents** — a well-commented `examples/*.yaml` is a great first PR.
- **Docs** — clarity fixes, missing explanations, typos.
- **Bugs** — repro + fix + a regression test.

Browse [`good first issue`](https://github.com/Gabo-TheCreator/openmirai/labels/good%20first%20issue) and [`help wanted`](https://github.com/Gabo-TheCreator/openmirai/labels/help%20wanted) to find scoped work.

## Workflow

1. **Find or open an issue** describing the change. Comment "I'll take this" so we don't double up.
2. **Fork & branch**: `git checkout -b feat/short-description`.
3. **Write a test first** when fixing a bug or adding behavior — then make it pass.
4. **Keep PRs focused** — one logical change per PR.
5. **Run fmt + clippy + tests** locally (see TL;DR).
6. **Open the PR** using the template. Link the issue (`Closes #123`).

## Code conventions

- Match the style of the surrounding code; `cargo fmt` is the source of truth.
- No new `clippy` warnings (`--all-targets --all-features`).
- Public APIs get doc comments. New behavior gets tests.
- Agents are YAML; keep the format portable and language-agnostic.

## Adding a tool (example)

1. Create the tool under `engine/src/tools/builtin/<category>/`.
2. Register it in the builtin registry.
3. Add a unit test and, ideally, an `examples/*.yaml` that uses it.
4. Document it in the README "Built-in Tools" table.

## Reporting bugs / requesting features

Use the issue templates. For bugs, include: what you ran, the agent YAML (minimal repro), expected vs actual, and your OS + `mirai --version`.

## Code of Conduct

By participating you agree to our [Code of Conduct](CODE_OF_CONDUCT.md). Be respectful; assume good intent.

## License

By contributing, you agree your contributions are licensed under the [MIT License](LICENSE).
