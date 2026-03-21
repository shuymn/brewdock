<!-- Audience: coding agents. Direct instructions, not tutorials. -->

# ADR 0003: Prefer and ship manifest-targeted relocation before clonefile-first copy

## Context

`docs/pipeline-baseline.md` showed `materialize-payload` as one of the dominant wall-clock phases for multi-formula bottle installs. The next unchecked Theme asked for a spike that compares the current recursive copy plus full relocation walk against two candidate optimizations:

- `clonefile`-first copy with fallback
- relocation targeted by a placeholder manifest derived from extracted bottle payloads

The spike had to preserve the existing fail-closed boundary: extracted store content must stay read-only source material, rollback must keep operating on isolated keg state, and no optimization may let keg mutation alias shared store state.

## Decision

Use manifest-targeted relocation as the next implementation choice, and ship it as the production bottle relocation path. Defer `clonefile`-first copy.

- The 2026-03-22 spike replay on representative `jq` and `wget` extracted bottles selected manifest-targeted relocation as the next production step.
- On the replay captured on 2026-03-22, manifest-targeted relocation reduced `jq` copy/relocate wall time from `533.8ms` to `80.5ms`, while `clonefile`-first plus full relocation measured `94.5ms`.
- On the same replay, `wget` improved from `114.8ms` to `104.8ms` with manifest-targeted relocation, while `clonefile`-first regressed to `131.8ms`.
- `clonefile`-first plus manifest was not materially better than manifest-only on the representative replay, so the extra filesystem-specific fallback surface is not justified yet.
- Hardlink-first copy remains rejected even if future benchmarks look attractive, because writes during relocation/finalize could alias shared store state and break rollback isolation.
- The production path now derives the relocation manifest from extracted payloads before materialization and reuses it during finalize, so the benchmark-only spike harness is removed.

## Rejected Alternatives

- Implement `clonefile`-first copy first
  - Rejected because the measured gain was not consistently better than manifest-only, while rollout would require additional macOS/filesystem-specific fallback handling in the hottest correctness boundary.
- Adopt hardlink-first copy
  - Rejected because it weakens the store-is-read-only invariant by allowing keg writes to alias shared store state.
- Combine both optimizations immediately
  - Rejected because the spike goal was to leave one next implementation choice, not to commit the final architecture in the same Theme.

## Consequence

Bottle installs now derive a relocation manifest from extracted bottle payloads and use it to avoid a second full-tree placeholder scan during materialize/relocate.

`clonefile` remains a deferred follow-up. If future representative replays show a materially better outcome than manifest-only, revisit it as a separate Theme with explicit fallback semantics.

## Revisit trigger

- Representative pipeline replays show manifest-only no longer moves `materialize-payload` enough
- `clonefile`-first plus fallback becomes measurably better than manifest-only across representative bottles
- The extracted-store contract changes in a way that makes precomputed manifests unreliable
