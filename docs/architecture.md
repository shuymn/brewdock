<!-- Audience: coding agents. Direct instructions, not tutorials. -->

# Architecture Baseline

## Goal

Rust CLI (`bd`) that installs Homebrew `homebrew/core` stable bottles to `/opt/homebrew` on Apple Silicon macOS.

## Constraints

- Apple Silicon macOS + `/opt/homebrew` + stable bottle only
- No cask, external tap, source build, post_install hook, Linux/Intel runtime
- All tests must pass on Linux CI (`ubuntu-latest`)
- `formulae.brew.sh` JSON API (no tap clone)
- Homebrew-compatible file layout, receipt, linking (Ruby API compatibility is non-goal)
- `unsafe_code` forbidden; `unwrap`/`expect`/`todo`/`dbg!` denied

## Core Boundaries

5-crate workspace:

```
cli → core → {formula, bottle, cellar}
```

- `brewdock-formula`: types, API client, supportability, dep resolve. No core dependency.
- `brewdock-bottle`: download, SHA256 verify, extract, CAS store. Depends on formula (types only).
- `brewdock-cellar`: materialize, receipt, linking, SQLite state. Depends on formula (types only).
- `brewdock-core`: Layout, platform, lock, orchestration (install/upgrade), error aggregation. Depends on formula, bottle, cellar.
- `brewdock-cli`: clap commands, tokio runtime. Depends on core only.

Layout lives in core. Lower crates receive paths as `&Path` arguments, never depend on Layout directly.

Each crate owns a `thiserror` error enum. Core aggregates with `#[from]`.

Test isolation: code never hardcodes `/opt/homebrew`. `Layout::with_root(tempdir)` enables all tests to run on Linux CI.

## Key Tech Decisions

| Concern | Choice | Rationale |
|---------|--------|-----------|
| CLI | clap (derive) | Standard, derive macro reduces boilerplate |
| HTTP | reqwest (rustls-tls, stream) | Streaming download, no OpenSSL system dep |
| Async | tokio | Bottle parallel download via JoinSet; blocking I/O via spawn_blocking |
| SHA256 | sha2 | Pure Rust, streaming chunk update |
| Archive | flate2 + tar | Standard; Homebrew bottles are .tar.gz |
| State | rusqlite (bundled) | No system SQLite dep, works on CI |
| Lock | fs2 | Portable advisory file lock (macOS + Linux) |
| Error (lib) | thiserror | Per-crate typed errors |
| Error (app) | anyhow | CLI context wrapping |
| API abstraction | Generic trait (not trait object) | Static dispatch; mock in tests via generic parameter |
| Logging | tracing + tracing-subscriber | Structured, level-controlled |

## Open Questions

None blocking.

## Revisit Trigger

- Need to support Linux runtime or Intel Mac
- Need to support cask or external taps
- Formula count exceeds JSON API scalability
- Need post_install hook support
