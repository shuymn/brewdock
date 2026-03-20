<!-- Audience: coding agents. Direct instructions, not tutorials. -->

# Architecture Baseline

## Goal

Rust CLI (`bd`) that installs Homebrew `homebrew/core` formulae to `/opt/homebrew` on Apple Silicon macOS, preferring compatible stable bottles and falling back to a minimal source build path when needed.

## Constraints

- Apple Silicon macOS + `/opt/homebrew`
- Prefer stable bottle install; if no compatible bottle exists, allow a minimal generic source build fallback
- `post_install` support is restricted to a fail-closed subset parsed from `homebrew/core` Ruby source; no arbitrary Ruby execution
- No cask, external tap, Linux/Intel runtime
- All tests must pass on Linux CI (`ubuntu-latest`)
- `formulae.brew.sh` JSON API (no tap clone)
- Homebrew-compatible file layout, receipt, linking (`/opt/homebrew` paths always flow through `Layout`; Ruby API compatibility is non-goal)
- `unsafe_code` forbidden; `unwrap`/`expect`/`todo`/`dbg!` denied

## Core Boundaries

5-crate workspace:

```
cli → core → {formula, bottle, cellar}
```

- `brewdock-formula`: types, API client, bottle selection, install method planning inputs, dep resolve. No core dependency.
- `brewdock-bottle`: download, SHA256 verify, extract, CAS store. Depends on formula (types only).
- `brewdock-cellar`: materialize, receipt, relocation, linking, SQLite state, restricted `post_install` execution primitives. Depends on formula (types only).
- `brewdock-core`: Layout, platform, lock, orchestration (install/upgrade), install method resolution, source build driver, error aggregation. Depends on formula, bottle, cellar.
- `brewdock-cli`: clap commands, tokio runtime. Depends on core only.

Layout lives in core. Lower crates receive paths as `&Path` arguments, never depend on Layout directly.

Each crate owns a `thiserror` error enum. Core aggregates with `#[from]`.

Test isolation: code never hardcodes `/opt/homebrew`. `Layout::with_root(tempdir)` enables all tests to run on Linux CI.

## Key Tech Decisions

| Concern | Choice | Rationale |
|---------|--------|-----------|
| CLI | clap (derive) | Standard, derive macro reduces boilerplate |
| HTTP | reqwest (rustls-tls, stream) | JSON API, bottle download, source archive fetch, Ruby source fetch without OpenSSL system dep |
| Async | tokio | Network orchestration; blocking I/O and local builds stay isolated |
| SHA256 | sha2 | Pure Rust, streaming chunk update |
| Archive | flate2 + tar | Standard; Homebrew bottles are .tar.gz |
| State | rusqlite (bundled) | No system SQLite dep, works on CI |
| Lock | fs2 | Portable advisory file lock (macOS + Linux) |
| Error (lib) | thiserror | Per-crate typed errors |
| Error (app) | anyhow | CLI context wrapping |
| API abstraction | Generic trait (not trait object) | Static dispatch; mock in tests via generic parameter |
| Logging | tracing + tracing-subscriber | Structured, level-controlled |
| Bottle selection | Compatible tag fallback (`arm64_sequoia -> arm64_sonoma -> arm64_ventura -> all`) | Matches target Homebrew usage without requiring exact host tag parity |
| `post_install` execution | Parse-and-execute restricted DSL subset from `homebrew/core` Ruby source | Removes Ruby runtime dependency while staying fail-closed on unsupported syntax |
| Source fallback | Generic build driver (`cmake`/`configure`/`meson`/`make`) | Enables a small first source path without full Formula DSL compatibility |

## Open Questions

None blocking. Decision record: [ADR 0001](adr/0001-nanobrew-install-method.md).

## Revisit Trigger

- Need to support Linux runtime or Intel Mac
- Need to support cask or external taps
- Formula count exceeds JSON API scalability
- Need Homebrew Formula DSL compatibility beyond the restricted `post_install` subset
- Generic source build fallback cannot cover target formulae without Ruby formula execution
