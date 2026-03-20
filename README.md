# brewdock

`brewdock` is an experimental Rust CLI (`bd`) for installing Homebrew `homebrew/core` formulae into `/opt/homebrew` on Apple Silicon macOS. It prefers stable bottles and falls back to a minimal source build path when needed.

> [!WARNING]
> This is a hobby project.
> It may break, desynchronize, or otherwise damage your Homebrew environment.
> Do not use it for practical, production, or machine-critical purposes.
> If you care about keeping your Homebrew installation healthy, use Homebrew itself.

## Status

- Target platform: Apple Silicon macOS with `/opt/homebrew`
- Scope: `homebrew/core` formulae only
- Non-goals: casks, external taps, Linux runtime, Intel Mac runtime, compatibility with full Homebrew Formula DSL

## Usage

Once `bd` is available on your `PATH`, use it directly:

```bash
bd --help
```

Main commands:

```bash
# Update formula index
bd update

# Install formulae
bd install jq wget

# Upgrade everything currently installed by brewdock
bd upgrade

# Upgrade specific formulae
bd upgrade jq
```

Useful global flags:

```bash
# Preview actions without executing
bd --dry-run install jq

# More logs
bd --verbose install jq

# Errors only
bd --quiet install jq
```

## Local Development

This repository uses the toolchain pinned in `rust-toolchain.toml`.

For development, run the CLI via Task or Cargo:

```bash
task run -- --help
cargo run -p brewdock-cli -- --help
```

```bash
task build
task test
task lint
task fmt
task check
```

Rust-native equivalents:

```bash
cargo build --workspace --locked
cargo test --workspace --all-targets --all-features --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo doc --workspace --no-deps
```

Optional: install [Lefthook](https://github.com/evilmartians/lefthook) and enable git hooks.

```bash
lefthook install
```

## Repository Layout

- `crates/cli`: CLI entrypoint
- `crates/core`: orchestration, layout, install flow
- `crates/formula`: formula metadata and resolution
- `crates/bottle`: bottle download, verification, extraction
- `crates/cellar`: cellar materialization, linking, receipts, state

## Docs

- [docs/architecture.md](docs/architecture.md)
- [docs/coding.md](docs/coding.md)
- [docs/testing.md](docs/testing.md)
- [docs/tooling.md](docs/tooling.md)
