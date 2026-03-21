<!-- Audience: coding agents. Direct instructions, not tutorials. -->

# ADR 0002: Separate user-facing progress output from tracing subscribers

## Context

`bd` originally exposed raw `tracing_subscriber::fmt` output during normal CLI runs. That preserved internal phase visibility, but it leaked implementation-oriented logs directly into the user experience and made non-TTY output noisy. The benchmark pipeline still depends on JSON `tracing` output written via `BREWDOCK_BENCHMARK_FILE`, so removing tracing entirely is not acceptable.

The CLI needs a stable, non-interactive progress experience for long-running commands and consistent static formatting for result-oriented commands, while keeping structured tracing available for diagnostics and benchmark scripts.

## Decision

Introduce an explicit user-facing progress event layer in `brewdock-core` and make `brewdock-cli` render those events.

- `brewdock-core` emits operation, phase, formula, warning, and failure events through an `OperationProgressSink`.
- The default sink is a no-op so library-style callers do not inherit terminal behavior.
- `brewdock-cli` provides the sink implementation and renders long-running commands with `indicatif` when stderr is a TTY.
- Non-TTY runs fall back to plain line-oriented progress output with no spinner control sequences.
- Result-oriented commands keep static formatted summaries and do not depend on progress rendering.
- Raw `tracing` output is suppressed during normal CLI runs; `tracing` stays enabled for benchmark JSON capture and internal diagnostics.

## Rejected Alternatives

- Keep deriving user output from `tracing_subscriber`
  - Rejected because phase names, warning text, and verbosity policy would remain coupled to diagnostics internals.
- Replace the CLI with a full `ratatui` interface
  - Rejected because the target UX is non-interactive progress, not a stateful terminal application.
- Disable tracing entirely outside benchmarks
  - Rejected because internal diagnostics and benchmark contracts still need spans and structured events.

## Consequence

User-facing terminal text becomes an API owned by the CLI/output layer rather than an accidental side effect of tracing configuration.

`brewdock-core` gains a small observer interface and tests that lock event ordering for install, update, and upgrade flows.

Benchmark scripts keep consuming JSON tracing output unchanged, while normal users see concise progress and summary output.

## Revisit trigger

- Need interactive terminal controls, panes, or live tables
- Multiple frontends besides the CLI need different progress presentations from the same core events
- Benchmark scripts stop depending on tracing-derived phase spans
