# Pipeline Baseline

_Aggregated across 3 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 0.85s (mean 0.87s, min 0.85s, max 0.91s) | fetch-formula-index 738.3ms, persist-formula-index 98.1ms | - |
| install-tree | 0.86s (mean 0.87s, min 0.55s, max 1.21s) | prefetch-payload 788.5ms, download-bottle 778.9ms, resolve-install-list 61.6ms | - |
| install-jq-wget | 4.12s (mean 4.87s, min 3.78s, max 6.72s) | prefetch-payload 2231.3ms, download-bottle 2189.1ms, materialize-payload 1356.0ms | materialize-payload 1006.4ms |
| upgrade-dry-run-jq | 0.07s (mean 0.07s, min 0.07s, max 0.07s) | collect-upgrade-candidates 58.7ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 738.3ms (mean 749.2, min 736.7, max 772.7) | 65.7ms (mean 67.6, min 62.3, max 74.7) | 676.0ms (mean 681.7, min 671.0, max 698.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 98.1ms (mean 103.6, min 97.7, max 115.0) | 98.1ms (mean 103.6, min 97.7, max 115.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 788.5ms (mean 745.6, min 472.4, max 975.8) | 17.4ms (mean 18.6, min 13.5, max 24.8) | 775.0ms (mean 727.0, min 455.0, max 951.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 778.9ms (mean 732.1, min 458.8, max 958.6) | 3.9ms (mean 5.1, min 3.8, max 7.6) | 775.0ms (mean 727.0, min 455.0, max 951.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 61.6ms (mean 113.0, min 59.2, max 218.2) | 23.2ms (mean 23.2, min 23.0, max 23.4) | 38.2ms (mean 89.8, min 36.2, max 195.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 10.7ms (mean 10.7, min 6.9, max 14.5) | 10.7ms (mean 10.7, min 6.9, max 14.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 2.0ms (mean 1.9, min 1.8, max 2.0) | 2.0ms (mean 1.9, min 1.8, max 2.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1.7ms (mean 1.8, min 1.5, max 2.2) | 1.7ms (mean 1.8, min 1.5, max 2.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.2ms (mean 0.2, min 0.2, max 0.2) | 0.2ms (mean 0.2, min 0.2, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.2ms (mean 0.2, min 0.2, max 0.2) | 0.2ms (mean 0.2, min 0.2, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 2231.3ms (mean 2399.3, min 1959.1, max 3007.5) | 1016.2ms (mean 997.5, min 952.2, max 1024.1) | 6064.0ms (mean 6398.0, min 5068.0, max 8062.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 2189.1ms (mean 2204.1, min 1914.0, max 2509.1) | 68.5ms (mean 70.0, min 65.4, max 75.9) | 6064.0ms (mean 6398.7, min 5070.0, max 8062.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1356.0ms (mean 1370.7, min 1330.7, max 1425.5) | 2568.0ms (mean 2548.1, min 2437.0, max 2639.3) | 0.3ms (mean 0.3, min 0.1, max 0.6) | 1006.4ms (mean 1004.6, min 952.4, max 1055.1) |
| extract-bottle | 845.8ms (mean 843.7, min 815.0, max 870.2) | 845.8ms (mean 843.7, min 815.0, max 870.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 318.1ms (mean 312.0, min 270.8, max 347.2) | 318.1ms (mean 312.0, min 270.8, max 347.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 154.4ms (mean 590.1, min 140.6, max 1475.4) | 23.6ms (mean 24.1, min 23.4, max 25.4) | 131.0ms (mean 566.0, min 117.0, max 1450.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 72.3ms (mean 75.7, min 62.1, max 92.7) | 72.3ms (mean 75.7, min 62.0, max 92.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 57.5ms (mean 186.2, min 53.5, max 447.6) | 4.0ms (mean 9.2, min 4.0, max 19.6) | 53.5ms (mean 177.0, min 49.5, max 428.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 55.3ms (mean 178.8, min 51.2, max 429.8) | 1.8ms (mean 1.8, min 1.7, max 1.8) | 53.5ms (mean 177.0, min 49.5, max 428.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.1ms (mean 1.3, min 1.1, max 1.7) | 1.1ms (mean 1.3, min 1.1, max 1.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.7ms (mean 1.6, min 0.7, max 3.6) | 0.7ms (mean 1.6, min 0.7, max 3.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.1ms (mean 0.1, min 0.1, max 0.1) | 0.1ms (mean 0.1, min 0.1, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 58.7ms (mean 59.1, min 55.9, max 62.7) | 24.0ms (mean 24.0, min 23.7, max 24.2) | 34.7ms (mean 35.1, min 32.2, max 38.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
