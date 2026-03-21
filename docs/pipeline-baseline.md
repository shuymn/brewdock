# Pipeline Baseline

_Aggregated across 3 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 0.88s (mean 0.96s, min 0.83s, max 1.18s) | fetch-formula-index 771.8ms, persist-formula-index 95.0ms | - |
| install-tree | 0.84s (mean 0.85s, min 0.84s, max 0.86s) | prefetch-payload 774.3ms, download-bottle 764.4ms, resolve-install-list 59.3ms | - |
| install-jq-wget | 4.15s (mean 4.18s, min 3.98s, max 4.40s) | prefetch-payload 2433.4ms, download-bottle 2116.9ms, materialize-payload 1207.5ms | materialize-payload 899.7ms |
| upgrade-dry-run-jq | 0.06s (mean 0.06s, min 0.06s, max 0.07s) | collect-upgrade-candidates 58.5ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 771.8ms (mean 849.1, min 696.9, max 1078.5) | 65.8ms (mean 64.4, min 58.5, max 68.9) | 706.0ms (mean 784.7, min 628.0, max 1020.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 95.0ms (mean 100.9, min 93.7, max 114.0) | 95.0ms (mean 100.9, min 93.7, max 114.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 774.3ms (mean 777.3, min 769.6, max 788.1) | 16.3ms (mean 18.0, min 12.6, max 25.1) | 758.0ms (mean 759.3, min 757.0, max 763.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 764.4ms (mean 765.5, min 760.3, max 771.7) | 6.4ms (mean 5.8, min 3.3, max 7.7) | 758.0ms (mean 759.7, min 757.0, max 764.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 59.3ms (mean 58.0, min 55.2, max 59.6) | 22.4ms (mean 22.3, min 22.0, max 22.6) | 36.7ms (mean 35.7, min 33.2, max 37.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 7.2ms (mean 7.8, min 6.9, max 9.3) | 7.2ms (mean 7.8, min 6.9, max 9.3) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 1.8ms (mean 2.8, min 1.8, max 4.9) | 1.8ms (mean 2.8, min 1.8, max 4.9) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1.6ms (mean 2.3, min 1.5, max 3.6) | 1.6ms (mean 2.2, min 1.5, max 3.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.2ms (mean 0.3, min 0.2, max 0.5) | 0.2ms (mean 0.3, min 0.2, max 0.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.2ms (mean 0.3, min 0.1, max 0.4) | 0.2ms (mean 0.3, min 0.1, max 0.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 2433.4ms (mean 2466.0, min 2291.6, max 2672.9) | 981.1ms (mean 975.8, min 935.0, max 1011.2) | 6418.0ms (mean 6372.3, min 6229.0, max 6470.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 2116.9ms (mean 2100.6, min 1933.4, max 2251.5) | 64.7ms (mean 70.3, min 63.6, max 82.7) | 6418.0ms (mean 6372.7, min 6230.0, max 6470.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1207.5ms (mean 1205.4, min 1189.3, max 1219.4) | 2293.6ms (mean 2326.4, min 2243.3, max 2442.4) | 0.9ms (mean 0.7, min 0.1, max 1.1) | 899.7ms (mean 901.7, min 876.0, max 929.4) |
| extract-bottle | 810.7ms (mean 827.2, min 809.6, max 861.4) | 810.7ms (mean 827.2, min 809.6, max 861.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 295.4ms (mean 289.0, min 266.4, max 305.3) | 295.4ms (mean 289.0, min 266.4, max 305.3) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 150.5ms (mean 148.8, min 145.4, max 150.5) | 23.4ms (mean 23.5, min 22.5, max 24.5) | 126.0ms (mean 125.3, min 122.0, max 128.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 78.6ms (mean 71.6, min 56.0, max 80.0) | 78.6ms (mean 71.6, min 56.0, max 80.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 55.2ms (mean 54.5, min 51.2, max 57.1) | 3.3ms (mean 3.3, min 3.1, max 3.6) | 52.1ms (mean 51.2, min 47.9, max 53.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 53.6ms (mean 52.7, min 49.5, max 55.0) | 1.5ms (mean 1.5, min 1.5, max 1.6) | 52.1ms (mean 51.2, min 47.9, max 53.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.3ms (mean 1.2, min 0.7, max 1.6) | 1.3ms (mean 1.2, min 0.7, max 1.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.5ms (mean 0.6, min 0.5, max 0.7) | 0.5ms (mean 0.6, min 0.5, max 0.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 58.5ms (mean 58.6, min 57.9, max 59.5) | 22.8ms (mean 23.2, min 22.7, max 24.2) | 35.3ms (mean 35.4, min 35.2, max 35.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
