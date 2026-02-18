# a8e (Articulate) - Agent Guidelines

This is a Rust workspace with the following crates:

| Crate | Description |
|-------|-------------|
| `a8e` (crates/a8e-cli) | CLI binary - the main entry point |
| `a8e-core` (crates/a8e-core) | Core agent engine, providers, session management |
| `a8e-mcp` (crates/a8e-mcp) | Built-in MCP tool servers |
| `a8e-server` (crates/a8e-server) | HTTP/WebSocket server |
| `a8e-acp` (crates/a8e-acp) | Agent Communication Protocol server |

## Build

```bash
cargo build           # Build all crates
cargo build -p a8e    # Build CLI only
cargo test            # Run all tests
```

## Key Design Principles

- **Zero telemetry**: No PostHog, no Sentry, no data collection.
- **Local-first**: Designed to work offline with local models.
- **BYOK**: Bring your own key — any LLM provider works.
- **MCP-native**: Model Context Protocol for tool extensibility.
