# Pipeline Baseline

_Aggregated across 3 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 0.87s (mean 0.92s, min 0.82s, max 1.07s) | fetch-formula-index 765.6ms, persist-formula-index 97.8ms | - |
| install-tree | 0.83s (mean 0.90s, min 0.82s, max 1.05s) | prefetch-payload 765.6ms, download-bottle 755.5ms, resolve-install-list 58.6ms | - |
| install-jq-wget | 4.23s (mean 4.49s, min 4.10s, max 5.13s) | prefetch-payload 2454.1ms, download-bottle 2160.3ms, materialize-payload 1324.5ms | materialize-payload 961.6ms |
| upgrade-dry-run-jq | 0.07s (mean 0.07s, min 0.07s, max 0.07s) | collect-upgrade-candidates 59.0ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 765.6ms (mean 755.9, min 706.2, max 796.0) | 60.6ms (mean 64.6, min 59.2, max 74.0) | 705.0ms (mean 691.3, min 647.0, max 722.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 97.8ms (mean 147.8, min 93.6, max 252.0) | 97.8ms (mean 147.8, min 93.6, max 252.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 765.6ms (mean 834.6, min 752.6, max 985.5) | 14.6ms (mean 17.9, min 12.6, max 26.5) | 751.0ms (mean 816.7, min 740.0, max 959.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 755.5ms (mean 822.2, min 743.6, max 967.5) | 4.5ms (mean 5.5, min 3.6, max 8.5) | 751.0ms (mean 816.7, min 740.0, max 959.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 58.6ms (mean 58.4, min 57.0, max 59.6) | 23.0ms (mean 22.9, min 22.4, max 23.4) | 35.6ms (mean 35.5, min 33.6, max 37.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 6.7ms (mean 9.3, min 6.2, max 15.0) | 6.7ms (mean 9.3, min 6.2, max 15.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 2.2ms (mean 2.1, min 1.8, max 2.2) | 2.2ms (mean 2.1, min 1.8, max 2.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1.8ms (mean 1.8, min 1.8, max 1.8) | 1.8ms (mean 1.8, min 1.8, max 1.8) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.3ms (mean 0.3, min 0.1, max 0.6) | 0.3ms (mean 0.3, min 0.1, max 0.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.3ms (mean 0.3, min 0.2, max 0.3) | 0.3ms (mean 0.3, min 0.2, max 0.3) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.2ms (mean 0.2, min 0.1, max 0.2) | 0.2ms (mean 0.2, min 0.1, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 2454.1ms (mean 2639.4, min 2244.0, max 3220.0) | 1012.6ms (mean 1018.0, min 1006.6, max 1034.8) | 8640.0ms (mean 9469.0, min 8158.0, max 11609.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 2160.3ms (mean 2187.6, min 1698.5, max 2703.8) | 77.2ms (mean 78.2, min 74.9, max 82.5) | 8640.0ms (mean 9469.0, min 8158.0, max 11609.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1324.5ms (mean 1305.9, min 1253.9, max 1339.1) | 2541.4ms (mean 2508.6, min 2413.1, max 2571.4) | 0.2ms (mean 0.3, min 0.2, max 0.7) | 961.6ms (mean 965.2, min 923.6, max 1010.4) |
| extract-bottle | 843.4ms (mean 844.4, min 836.8, max 853.0) | 843.4ms (mean 844.4, min 836.8, max 853.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 306.3ms (mean 316.4, min 277.5, max 365.5) | 306.3ms (mean 316.4, min 277.5, max 365.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 148.4ms (mean 152.7, min 147.0, max 162.7) | 23.4ms (mean 23.4, min 23.0, max 23.7) | 125.0ms (mean 129.3, min 124.0, max 139.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 86.6ms (mean 86.1, min 83.5, max 88.2) | 86.6ms (mean 86.1, min 83.5, max 88.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 58.3ms (mean 57.3, min 52.1, max 61.6) | 4.0ms (mean 4.2, min 3.3, max 5.5) | 55.0ms (mean 53.1, min 48.1, max 56.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 56.5ms (mean 54.6, min 49.7, max 57.7) | 1.6ms (mean 1.6, min 1.5, max 1.6) | 55.0ms (mean 53.1, min 48.1, max 56.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.5ms (mean 1.5, min 1.1, max 1.7) | 1.5ms (mean 1.5, min 1.1, max 1.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.6ms (mean 1.2, min 0.5, max 2.5) | 0.6ms (mean 1.2, min 0.5, max 2.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 59.0ms (mean 58.5, min 56.9, max 59.5) | 23.5ms (mean 23.4, min 23.1, max 23.6) | 35.4ms (mean 35.1, min 33.8, max 36.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
