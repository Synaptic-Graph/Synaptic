# Synaptic

<p align="center">
  <a href="https://discord.gg/ytX7R2PbNz"><img src="https://img.shields.io/badge/Discord-Join%20the%20community-5865F2?logo=discord&logoColor=white&style=for-the-badge" alt="Join our Discord"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue?style=for-the-badge" alt="License: AGPL-3.0-or-later"></a>
  <a href="https://github.com/ColinVaughn/Synaptic/releases"><img src="https://img.shields.io/github/v/release/ColinVaughn/Synaptic?style=for-the-badge" alt="Latest release"></a>
</p>

**Give your coding assistant a map of the codebase, not a pile of files.**

Synaptic maps a repository once so you and your coding assistant do not have to rediscover it
every session. Ask how a system works, what depends on a symbol, or what a change may break.
The answers come from a persistent, source-grounded knowledge graph, available from the CLI
or through MCP. Synaptic can also remember past changes and plan small, reviewable API and
dependency repairs.

Code extraction is local and deterministic. Synaptic ships as a single Rust binary with no
runtime, database, account, or API key required.

[Documentation](https://github.com/ColinVaughn/Synaptic/wiki) |
[Quickstart](https://github.com/ColinVaughn/Synaptic/wiki/Quickstart) |
[Benchmarks](BENCHMARKS.md) |
[Discord](https://discord.gg/ytX7R2PbNz)

## Get started

Install the latest checksummed release:

```sh
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/ColinVaughn/Synaptic/main/install.sh | sh

# Windows PowerShell
irm https://raw.githubusercontent.com/ColinVaughn/Synaptic/main/install.ps1 | iex
```

Then run Synaptic from a repository:

```sh
synaptic extract .                         # build synaptic-out/graph.json
synaptic query "how does authentication work?"
synaptic affected parse_config             # what could this change break?
```

Extraction honors `.gitignore` and `.synapticignore`, skips common secret files, and stays
offline unless you explicitly enable a network-backed feature.

## Connect your coding assistant

```sh
synaptic install claude                    # Claude Code
synaptic install codex                     # Codex CLI
synaptic install codex --global            # Codex desktop app
```

Claude and Codex load only a small set of tools at startup, then find the rest on demand. This
keeps prompts smaller without hiding capabilities. Gemini, Cursor, Copilot, OpenCode, Kilo,
and generic `AGENTS.md` clients are supported too. See
[Assistant Integration](https://github.com/ColinVaughn/Synaptic/wiki/Assistant-Integration).

## What it does

| Need | What Synaptic provides |
|---|---|
| Understand the code | Symbols, calls, imports, inheritance, resources, SQL, and cross-language boundaries in one graph |
| Find impact | Callers, references, reverse dependencies, dynamic-dispatch hazards, and affected tests |
| Change code safely | Change forecasts, refactor plans, architecture diffs, and optional verification in an isolated worktree |
| Remember the past | Source-linked history from commits, decisions, incidents, reviews, CI, and previous attempts |
| Maintain dependencies | API contract tracking and vulnerability evidence with small, reviewable repair workflows |

Synaptic supports dozens of languages, incremental updates, multi-repository federation,
structural search, SQL auditing, graph-aware PR review, and exports for GraphML, Cypher,
Graphviz, Obsidian, and Markdown. Detailed capabilities live in the
[documentation](https://github.com/ColinVaughn/Synaptic/wiki), keeping this page focused on getting started.

## See the architecture

```sh
synaptic chart
```

This creates a self-contained architecture map from the graph. Open a subsystem, select a
symbol, and follow its real incoming and outgoing relationships without a server.

<p align="center">
  <img src="assets/readme/synaptic-chart-overview.gif" alt="Synaptic architecture chart switching themes and opening a subsystem" width="1200">
</p>

## Built to save context

Synaptic returns the relevant slice of a graph instead of loading whole source files into an
assistant's context. In the checked-in token benchmark, full graph answers used 27-38x fewer
tokens than reading the source files referenced by those answers. That is a context-compression
measurement, not a promise about total task cost. The methodology and raw results are in
[BENCHMARKS.md](BENCHMARKS.md#agent-token-efficiency-and-standard-retrieval).

## Common commands

| Command | Purpose |
|---|---|
| `synaptic extract .` | Build the graph |
| `synaptic update` / `synaptic watch` | Keep it current |
| `synaptic query "..."` | Find the relevant subgraph for a question |
| `synaptic affected <symbol>` | Trace reverse impact |
| `synaptic predict <files>` | Forecast risk and select tests |
| `synaptic speculate <files>` | Verify a change in a throwaway worktree |
| `synaptic memory search "..."` | Find relevant repository history |
| `synaptic serve` | Run the MCP server |

Run `synaptic <command> --help` for flags or use the complete
[command reference](https://github.com/ColinVaughn/Synaptic/wiki/Commands).

## Pick a workflow

- **CLI:** the fastest path for local extraction, queries, automation, and MCP.
- **Desktop:** run `synaptic-ui` for visual repository setup, federation, assistant
  connection, updates, and the complete command catalog.
- **Hosted:** [Synaptic Cloud](https://synapticgraph.com/) provides a managed MCP service.
  See the [GitHub automation guide](https://synapticgraph.com/docs/github-automation) for
  commit-triggered graph sync and verified repair workflows.

Update a release installation with `synaptic self-update`.

## Documentation

- [Installation](https://github.com/ColinVaughn/Synaptic/wiki/Installation) and
  [Quickstart](https://github.com/ColinVaughn/Synaptic/wiki/Quickstart)
- [Querying](https://github.com/ColinVaughn/Synaptic/wiki/Querying),
  [languages](https://github.com/ColinVaughn/Synaptic/wiki/Languages), and
  [output formats](https://github.com/ColinVaughn/Synaptic/wiki/Output-Formats)
- [MCP server](https://github.com/ColinVaughn/Synaptic/wiki/MCP-Server) and
  [assistant integration](https://github.com/ColinVaughn/Synaptic/wiki/Assistant-Integration)
- [Repository memory](https://github.com/ColinVaughn/Synaptic/wiki/Repository-Memory),
  [API maintenance](https://github.com/ColinVaughn/Synaptic/wiki/Commands#api), and
  [vulnerability management](https://github.com/ColinVaughn/Synaptic/wiki/Vulnerability-Management)
- [Workspaces and federation](https://github.com/ColinVaughn/Synaptic/wiki/Workspaces-and-Federation),
  [configuration](https://github.com/ColinVaughn/Synaptic/wiki/Configuration), and
  [development](https://github.com/ColinVaughn/Synaptic/wiki/Development)

## Build from source

The repository pins Rust 1.97.1.

```sh
cargo install --path bin/synaptic
cargo install --path bin/synaptic-ui       # optional desktop app
```

For development and architecture notes, see the
[development guide](https://github.com/ColinVaughn/Synaptic/wiki/Development).

## Community and license

Questions, ideas, or something you built? Join the
[Discord community](https://discord.gg/ytX7R2PbNz).

Synaptic is licensed under `AGPL-3.0-or-later`; see [LICENSE](LICENSE) and [NOTICE](NOTICE).
If you modify Synaptic and make it available over a network, you must offer those users the
corresponding source. The separately maintained Synaptic Cloud service is proprietary and is
not covered by this repository's license.
