# brewdock

An experimental Rust CLI for fast Homebrew bottle installation on Apple Silicon macOS. It installs `homebrew/core` formulae directly into `/opt/homebrew` and coexists with an existing Homebrew environment.

> [!WARNING]
> This is a hobby project.
> It may break, desynchronize, or otherwise damage your Homebrew environment.
> Do not use it for practical, production, or machine-critical purposes.
> If you care about keeping your Homebrew installation healthy, use Homebrew itself.

## What it does

brewdock (`bd`) installs and manages `homebrew/core` formulae without going through Homebrew itself. It reuses Homebrew's ecosystem — the JSON API, pre-built bottles, and on-disk layout — while reimplementing the install pipeline natively in Rust for speed.

- **Bottle-first install**: Fetches, verifies, and extracts pre-built binaries with host-tag fallback. Falls back to generic source builds (cmake/configure/meson/make) when no compatible bottle exists.
- **Homebrew coexistence**: Installs land in the same Cellar/opt/bin layout Homebrew uses. Formulae installed by `bd` are visible to `brew` and vice versa.
- **Ruby-free post_install**: Parses formula Ruby source via `ruby-prism` AST analysis and executes only allowlisted operations natively — no Ruby runtime dependency.
- **Staged pipeline**: Executes in three phases — network acquire, local prepare, finalize — with bounded concurrency for the first two. Homebrew-visible mutations happen only at finalize.
- **Content-addressable storage**: Manages downloaded bottles by SHA256, providing a foundation for deduplication and warm-path optimization.

## Scope

- Target: Apple Silicon macOS (`/opt/homebrew`)
- `homebrew/core` formulae only
- Non-goals: casks, external taps, Linux/Intel runtime, full Homebrew Formula DSL compatibility

## Usage

```bash
bd install jq wget    # Install formulae
bd update             # Update formula index
bd upgrade            # Upgrade all installed formulae
bd outdated           # Show outdated formulae
bd search <pattern>   # Search available formulae
bd info <formula>     # Show formula details
bd list               # List installed formulae
bd cleanup            # Remove stale caches
bd doctor             # Check for problems
```

`--dry-run`, `--verbose`, `--quiet` flags are available globally.

## Local Development

This repository uses the toolchain pinned in `rust-toolchain.toml`.

```bash
task run -- --help
task build
task test
task lint
task fmt
task check          # fmt + lint + test + doc + build
task check:fast     # fmt + lint + build (no tests/docs)
```

Rust-native equivalents:

```bash
cargo build --workspace --locked
cargo test --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo fmt --all -- --check
```

Optional: install [Lefthook](https://github.com/evilmartians/lefthook) for git hooks (`lefthook install`).

VM-based validation (keeps local `/opt/homebrew` untouched):

```bash
./tests/vm-smoke-test.sh --formula jq
./tests/vm-benchmark.sh --formula tree --manager brewdock --manager homebrew
task bench:pipeline -- --runs 3 --output docs/pipeline-baseline.md
```

## Repository Layout

5-crate workspace: `cli → core → {formula, bottle, cellar}`

- `crates/cli`: CLI entrypoint (`bd`), argument parsing, progress rendering
- `crates/core`: orchestration, layout, platform detection, install/upgrade flow
- `crates/formula`: formula types, Homebrew JSON API client, bottle selection, dependency resolution, metadata cache
- `crates/bottle`: bottle download, SHA256 verification, tar.gz extraction, content-addressable blob store
- `crates/cellar`: keg materialization, binary relocation, symlink linking, install receipts, post_install execution, SQLite state

## Docs

- [docs/architecture.md](docs/architecture.md) — crate boundaries, design decisions, constraints
- [docs/coding.md](docs/coding.md) — Rust conventions, error handling, API design
- [docs/testing.md](docs/testing.md) — test organization, VM scripts
- [docs/tooling.md](docs/tooling.md) — Task interface, CI, hooks, Clippy policy
- [docs/review.md](docs/review.md) — code review checklist
