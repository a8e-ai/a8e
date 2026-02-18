<div align="center">

# a8e

**Articulate (a8e): The sovereign AI operator for your terminal.**

> **A** r t i c u l a t **E** → **A** + 8 letters + **E** = **a8e**

_Speak Freely. Code Locally._

<p align="center">
  <a href="https://opensource.org/licenses/Apache-2.0">
    <img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg">
  </a>
  <a href="https://crates.io/crates/a8e">
    <img src="https://img.shields.io/crates/v/a8e.svg" alt="crates.io">
  </a>
</p>
</div>

## What is a8e?

**a8e** (Articulate) is a local-first, privacy-respecting AI agent that runs
entirely on your machine. It automates complex development tasks — building
projects from scratch, writing and executing code, debugging failures,
orchestrating workflows, and interacting with external APIs — autonomously.

a8e is a hard-fork of [Goose](https://github.com/block/goose) (Apache 2.0),
rebuilt with a focus on **developer sovereignty**:

- **100% Telemetry Free** — zero PostHog, zero Sentry, zero data collection.
  Your code and conversations never leave your machine.
- **BYOK (Bring Your Own Key)** — works with any LLM provider: OpenRouter,
  Ollama, OpenAI, Anthropic, Google, and more.
- **MCP Native** — first-class Model Context Protocol support for extensible
  tool integrations.
- **Local First** — designed to work offline with local models via Ollama / vLLM.

## Quick Start

```bash
# One-line install (macOS / Linux)
curl -fsSL https://a8e.ai/install.sh | bash

# Or install via Cargo
cargo install a8e
```

```bash
# Configure your provider and model
a8e configure

# Start a session
a8e session
```

## CLI Usage

```text
   __ _  ___ ___
  / _` |( _ ) _ \   Articulate
 | (_| |/ _ \  __/   Speak Freely.
  \__,_| (_) \___|

a8e <command>

Commands:
  session     Start an interactive session
  run         Run a single task
  configure   Configure providers, models, and extensions
  info        Show version and configuration
```

## Architecture

a8e is built in Rust with a modular workspace:

| Crate | Description |
|-------|-------------|
| `a8e` | CLI binary — the main entry point |
| `a8e-core` | Core agent engine, providers, and session management |
| `a8e-mcp` | Built-in MCP tool servers (developer, memory, etc.) |
| `a8e-server` | HTTP/WebSocket server for desktop and API access |
| `a8e-acp` | Agent Communication Protocol server |

## Attribution

a8e is a hard-fork of [Goose](https://github.com/block/goose) by Block, Inc.,
licensed under the Apache License 2.0. See [NOTICE](./NOTICE) and
[LICENSE](./LICENSE) for details.

We stand on the shoulders of giants — and give back where we can.

## License

Apache License 2.0 — see [LICENSE](./LICENSE) for the full text.
</div>
