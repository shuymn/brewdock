# Pipeline Baseline

_Aggregated across 10 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 0.87s (mean 1.09s, min 0.79s, max 2.63s) | fetch-formula-index 757.2ms, persist-formula-index 94.8ms | - |
| install-tree | 0.86s (mean 0.83s, min 0.51s, max 0.88s) | prefetch-payload 785.1ms, download-bottle 773.7ms, resolve-install-list 58.0ms | - |
| install-jq-wget | 4.10s (mean 4.34s, min 3.76s, max 6.57s) | prefetch-payload 2326.7ms, download-bottle 2002.9ms, materialize-payload 1249.8ms | materialize-payload 908.5ms |
| upgrade-dry-run-jq | 0.07s (mean 0.07s, min 0.06s, max 0.07s) | collect-upgrade-candidates 58.3ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 757.2ms (mean 970.4, min 682.3, max 2522.5) | 63.3ms (mean 62.4, min 57.7, max 67.9) | 692.5ms (mean 908.0, min 618.0, max 2460.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 94.8ms (mean 106.4, min 90.1, max 201.0) | 94.8ms (mean 106.4, min 90.1, max 201.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 785.1ms (mean 741.3, min 445.1, max 810.4) | 16.5ms (mean 17.0, min 13.1, max 24.6) | 766.5ms (mean 724.3, min 432.0, max 795.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 773.7ms (mean 729.4, min 434.9, max 798.8) | 4.7ms (mean 5.1, min 2.9, max 7.4) | 766.5ms (mean 724.3, min 432.0, max 795.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 58.0ms (mean 74.3, min 55.3, max 221.0) | 23.0ms (mean 22.9, min 22.0, max 24.0) | 35.1ms (mean 51.4, min 32.1, max 199.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 7.7ms (mean 7.8, min 6.8, max 9.6) | 7.7ms (mean 7.8, min 6.8, max 9.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 3.1ms (mean 3.0, min 1.4, max 6.2) | 3.1ms (mean 3.0, min 1.4, max 6.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 2.7ms (mean 2.8, min 1.6, max 5.5) | 2.7ms (mean 2.8, min 1.6, max 5.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.3ms (mean 0.3, min 0.2, max 0.5) | 0.3ms (mean 0.3, min 0.2, max 0.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.2ms (mean 0.3, min 0.2, max 0.4) | 0.2ms (mean 0.3, min 0.2, max 0.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 2326.7ms (mean 2360.1, min 2013.3, max 2691.5) | 961.9ms (mean 977.0, min 950.6, max 1067.9) | 6111.0ms (mean 6110.1, min 4554.0, max 7271.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 2002.9ms (mean 2047.8, min 1489.7, max 2654.3) | 78.8ms (mean 78.9, min 68.6, max 90.9) | 6111.0ms (mean 6110.3, min 4554.0, max 7271.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1249.8ms (mean 1266.4, min 1157.7, max 1438.8) | 2322.6ms (mean 2339.1, min 2179.2, max 2532.3) | 0.2ms (mean 0.4, min 0.1, max 1.0) | 908.5ms (mean 924.0, min 829.2, max 1064.8) |
| extract-bottle | 810.5ms (mean 821.4, min 793.4, max 885.5) | 810.5ms (mean 821.4, min 793.4, max 885.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 274.6ms (mean 277.2, min 261.4, max 305.4) | 274.6ms (mean 277.2, min 261.4, max 305.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 150.5ms (mean 283.6, min 144.0, max 1472.2) | 23.8ms (mean 23.8, min 22.2, max 26.7) | 126.0ms (mean 259.8, min 121.0, max 1450.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 67.9ms (mean 68.0, min 55.6, max 84.2) | 67.9ms (mean 68.0, min 55.6, max 84.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 50.5ms (mean 139.5, min 49.0, max 931.0) | 4.1ms (mean 4.1, min 3.3, max 5.0) | 46.7ms (mean 135.4, min 45.3, max 926.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 48.2ms (mean 137.0, min 46.9, max 927.3) | 1.6ms (mean 1.6, min 1.3, max 1.8) | 46.7ms (mean 135.4, min 45.3, max 926.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.4ms (mean 1.4, min 0.8, max 1.7) | 1.4ms (mean 1.4, min 0.8, max 1.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.6ms (mean 0.8, min 0.5, max 2.3) | 0.6ms (mean 0.8, min 0.5, max 2.3) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.1ms (mean 0.1, min 0.0, max 0.1) | 0.1ms (mean 0.1, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 58.3ms (mean 58.7, min 56.3, max 61.2) | 23.7ms (mean 23.8, min 23.2, max 24.6) | 34.4ms (mean 34.9, min 32.7, max 38.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.1, min 0.0, max 0.1) | 0.0ms (mean 0.1, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

[1;34m==> Running host-side copy-strategy spike benchmark[0m
## Copy Strategy Spike

_Representative extracted bottles copied from the VM store after `bd install jq wget`; medians are aggregated on the host across the configured spike runs._

### jq

Files: 18. Placeholder-bearing files: 3.

| Strategy | Median | Delta vs current |
|---|---:|---:|
| current recursive copy + full relocation walk | 79.8ms | +0.0ms |
| recursive copy + manifest-targeted relocation | 76.8ms | -3.0ms |
| clonefile-first copy + full relocation walk | 80.0ms | +0.2ms |
| clonefile-first copy + manifest-targeted relocation | 80.6ms | +0.8ms |

Chosen next implementation choice: `manifest-targeted`.

Rejected from next-step consideration: hardlink-first copy would let keg writes alias shared store state, which breaks the fail-closed rollback boundary even if it benchmarks well.

## Copy Strategy Spike

_Representative extracted bottles copied from the VM store after `bd install jq wget`; medians are aggregated on the host across the configured spike runs._

### wget

Files: 91. Placeholder-bearing files: 1.

| Strategy | Median | Delta vs current |
|---|---:|---:|
| current recursive copy + full relocation walk | 101.9ms | +0.0ms |
| recursive copy + manifest-targeted relocation | 90.7ms | -11.3ms |
| clonefile-first copy + full relocation walk | 116.6ms | +14.6ms |
| clonefile-first copy + manifest-targeted relocation | 107.2ms | +5.3ms |

Chosen next implementation choice: `manifest-targeted`.

Rejected from next-step consideration: hardlink-first copy would let keg writes alias shared store state, which breaks the fail-closed rollback boundary even if it benchmarks well.
