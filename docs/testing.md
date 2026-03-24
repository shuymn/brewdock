# Testing Conventions

Read this file before writing or modifying tests in this repository.

## Running Tests

- Use `task test` for the full suite (unit, integration, and doctests via `cargo test`).
- Use `task check` for CI-equivalent local verification (includes `cargo doc` without dependency docs).
- For focused runs: `cargo test -p brewdock-core <filter>`, `cargo test -p brewdock-formula <filter>`, etc. Use `-- --nocapture` for output.
- Use [`tests/vm-smoke-test.sh`](../tests/vm-smoke-test.sh) for destructive install smoke tests that must not touch the local `/opt/homebrew` environment. It runs inside a disposable Tart macOS VM: `./tests/vm-smoke-test.sh [--keep] [--formula <name> ...]`.
- Use [`tests/vm-benchmark.sh`](../tests/vm-benchmark.sh) for comparative install benchmarks in a disposable Tart macOS VM. It installs Homebrew, brewdock, zerobrew, and nanobrew inside the VM and prints a Markdown summary table. Single-package rows use `--formula <name>` and multi-package one-command rows use `--formula-set <a,b,...>`: `./tests/vm-benchmark.sh [--keep] [--formula <name> ...] [--formula-set <a,b,...> ...] [--manager <name> ...]`.
- Use [`tests/vm-pipeline-baseline.sh`](../tests/vm-pipeline-baseline.sh) to capture brewdock-only `update` / `install` / `upgrade --dry-run` baselines with tracing-derived phase breakdowns inside a disposable Tart macOS VM. When variance is high, pass `--runs 3` (or at most `--runs 5`) so the generated Markdown aggregates scenario wall and per-phase `wall` / `busy` / `idle` / `child` timings with median plus mean/min/max spread: `./tests/vm-pipeline-baseline.sh [--keep] [--runs 3] [--output docs/pipeline-baseline.md]`.
- Before running the VM scripts, satisfy the shared prerequisites: install `tart`, pull the image configured in [`tests/vm-config.sh`](../tests/vm-config.sh), and build the release CLI with `cargo build --release -p brewdock-cli`.
- The benchmark and pipeline scripts use the same VM image configuration as the smoke test. `vm-benchmark.sh` also requires outbound network access inside the VM for the Homebrew / zerobrew / nanobrew installers.

## Suite Expectations

- Tests should be deterministic and avoid reliance on execution order unless explicitly serialized.
- Prefer small, fast unit tests; use integration tests under `tests/` for boundary behavior.
- Doctests validate examples in `///` comments; keep them minimal and runnable.

## Test Organization

- Unit tests go in a `#[cfg(test)] mod tests` submodule at the bottom of the file.
- Use `use super::*` in test modules to access private items.
- Integration tests under `tests/` test public API only.
- Shared test helpers go in `tests/common/mod.rs` (not `tests/common.rs`, which Cargo treats as a test binary).
- For large unit-test modules, move reusable mocks and archive/setup helpers into crate-local test support modules before splitting scenarios across files.

## Writing Tests

- Use `#[test]` functions that return `Result<(), E>` with `?` for cleaner error propagation instead of scattering `unwrap()`.
- Use `assert_eq!(actual, expected)` and `assert_ne!` — they show both values on failure. Include a message argument when the assertion is not self-explanatory.
- Test error paths and edge cases, not just happy paths.
- Name test functions descriptively: `test_parse_returns_error_on_empty_input`, not `test1`.
- For tests that need setup/teardown, use helper functions or RAII guards (Drop-based cleanup).
- Avoid `#[ignore]` without a comment explaining why and when the test should be un-ignored.

## Doc Tests

- Use `?` instead of `unwrap()` in doc examples.
- Use `# ` prefix to hide boilerplate (imports, main wrapper) while keeping examples compilable.
- Use `no_run` for examples that require external resources; `compile_fail` to demonstrate invalid usage.
