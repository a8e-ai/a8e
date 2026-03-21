# Release Process

Standard verification and release workflow for the a8e CLI.

## Prerequisites

- Rust toolchain installed (`rustup`)
- GitHub CLI (`gh`) installed and authenticated
- crates.io token configured (`cargo login`)
- Push access to the repository

## 1. Verify Code Quality

```bash
# Run formatter check
cargo fmt -- --check

# Run clippy lints (at minimum the CLI and core crates)
cargo clippy -p a8e-core -p a8e

# Run tests (some provider tests may be skipped without credentials)
cargo test -p a8e-core -p a8e
```

## 2. Build & Smoke Test

```bash
# Debug build (faster)
cargo build -p a8e

# Release build (CLI binary only — full workspace release may fail
# due to optional native dependencies like V8 that aren't needed for the CLI)
cargo build --release -p a8e

# Verify the binary works
./target/release/a8e --version
```

## 3. Bump Version

Update the workspace version in `Cargo.toml`:

```toml
[workspace.package]
version = "X.Y.Z"
```

Then update all inter-crate version references:

```bash
# Files that reference specific versions of workspace crates:
#   crates/a8e-cli/Cargo.toml
#   crates/a8e-core/Cargo.toml
#   crates/a8e-acp/Cargo.toml
#   crates/a8e-server/Cargo.toml

# Update Cargo.lock
cargo update -w
```

Verify the build still compiles after the version bump:

```bash
cargo build -p a8e
```

## 4. Commit & Push

```bash
git add -A
git commit -m "fix/feat/chore: <description>

<detailed explanation of changes>"
git push
```

## 5. Monitor CI

```bash
# List recent workflow runs
gh run list --limit 5

# The following workflows run on push to main:
#   - CI          — build + test across platforms
#   - Canary      — canary release
#   - Cargo Deny  — dependency audit (advisory failures are non-blocking)
```

Wait for **CI** to pass before proceeding.

## 6. Create Release Tag

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

This triggers the **Release** workflow which:
- Builds platform binaries (macOS aarch64/x86_64, Linux x86_64/aarch64, Windows)
- Creates a GitHub Release with attached binaries
- Publishes install scripts

Monitor the release:

```bash
gh run list --limit 5
gh release view vX.Y.Z
```

## 7. Publish Crates to crates.io

Publish in dependency order:

```bash
cargo publish -p a8e-mcp
cargo publish -p a8e-test-support
cargo publish -p a8e-core
cargo publish -p a8e-acp
cargo publish -p a8e-server
```

> **Note:** Each crate must wait for its dependencies to be indexed on
> crates.io before publishing. The `cargo publish` command handles this
> automatically.

## 8. Verify Installation

```bash
# Update local installation using the built-in updater
a8e update

# Confirm the new version is installed
a8e --version
```

## Workspace Structure

| Crate | Description | Publish Order |
|-------|-------------|:---:|
| `a8e-mcp` | MCP extensions, cron scheduler | 1 |
| `a8e-test-support` | Shared test utilities | 2 |
| `a8e-core` | Agent engine, providers, session management | 3 |
| `a8e-acp` | Agent Communication Protocol server | 4 |
| `a8e-server` | HTTP/tunnel server | 5 |
| `a8e` (CLI) | CLI binary (not published to crates.io) | — |

## Troubleshooting

- **Cargo Deny failures:** These are dependency security advisories and
  are typically non-blocking. Review the advisory details and update
  affected dependencies when possible.
- **Release build fails with `rusty_v8`:** Build individual crates
  (`cargo build --release -p a8e`) instead of the full workspace. The
  `a8e-test` crate has optional V8/Deno dependencies that may require
  additional native libraries.
- **crates.io version conflict:** If a crate fails to publish due to a
  dependency version mismatch, publish the dependency crate first and
  wait for indexing.
