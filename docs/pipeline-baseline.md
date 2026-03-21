# Pipeline Baseline

_Aggregated across 10 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 0.87s (mean 1.11s, min 0.79s, max 2.79s) | fetch-formula-index 753.8ms, persist-formula-index 95.2ms | - |
| install-tree | 0.54s (mean 7.36s, min 0.52s, max 67.85s) | prefetch-payload 463.8ms, download-bottle 452.0ms, resolve-install-list 58.6ms | - |
| install-jq-wget | 3.82s (mean 4.03s, min 3.50s, max 5.49s) | prefetch-payload 2150.9ms, download-bottle 1932.0ms, materialize-payload 1108.1ms | materialize-payload 930.4ms |
| upgrade-dry-run-jq | 0.07s (mean 0.07s, min 0.06s, max 0.07s) | collect-upgrade-candidates 59.0ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 753.8ms (mean 999.4, min 682.7, max 2668.3) | 60.4ms (mean 64.9, min 59.2, max 88.3) | 693.5ms (mean 934.5, min 623.0, max 2580.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 95.2ms (mean 96.2, min 91.5, max 108.0) | 95.2ms (mean 96.2, min 91.5, max 108.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 463.8ms (mean 7274.2, min 448.9, max 67816.7) | 15.0ms (mean 15.7, min 9.9, max 23.4) | 446.5ms (mean 7258.5, min 435.0, max 67800.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 452.0ms (mean 7263.1, min 438.6, max 67805.5) | 3.7ms (mean 4.5, min 3.4, max 7.7) | 447.0ms (mean 7258.6, min 435.0, max 67800.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 58.6ms (mean 74.9, min 57.7, max 222.1) | 23.0ms (mean 23.0, min 22.5, max 23.6) | 35.7ms (mean 51.9, min 34.4, max 199.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 7.4ms (mean 6.9, min 4.0, max 7.7) | 7.4ms (mean 6.9, min 4.0, max 7.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 2.2ms (mean 2.9, min 1.5, max 5.6) | 2.2ms (mean 2.9, min 1.5, max 5.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1.1ms (mean 1.2, min 0.9, max 1.6) | 1.0ms (mean 1.1, min 0.9, max 1.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.3ms (mean 0.3, min 0.2, max 0.5) | 0.3ms (mean 0.3, min 0.2, max 0.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.2ms (mean 0.3, min 0.2, max 0.7) | 0.2ms (mean 0.3, min 0.2, max 0.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 2150.9ms (mean 2254.8, min 1945.2, max 3362.4) | 1009.8ms (mean 1015.4, min 944.9, max 1081.5) | 5038.5ms (mean 5527.7, min 4904.0, max 8562.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 1932.0ms (mean 2018.1, min 1697.6, max 2857.6) | 73.4ms (mean 74.7, min 69.2, max 85.6) | 5038.5ms (mean 5527.8, min 4904.0, max 8562.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1108.1ms (mean 1119.2, min 1030.8, max 1278.0) | 1893.3ms (mean 1937.7, min 1767.6, max 2418.3) | 0.3ms (mean 0.7, min 0.1, max 3.5) | 930.4ms (mean 933.8, min 834.5, max 1107.4) |
| extract-bottle | 854.7ms (mean 855.6, min 801.0, max 929.7) | 854.7ms (mean 855.5, min 801.0, max 929.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 280.1ms (mean 298.2, min 273.5, max 402.2) | 280.1ms (mean 298.2, min 273.5, max 402.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 154.0ms (mean 251.8, min 141.9, max 1123.6) | 23.5ms (mean 24.3, min 22.7, max 32.8) | 130.5ms (mean 227.5, min 119.0, max 1100.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 66.0ms (mean 72.1, min 60.9, max 122.7) | 66.0ms (mean 72.1, min 60.9, max 122.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 54.2ms (mean 93.0, min 51.0, max 409.5) | 3.9ms (mean 4.3, min 3.6, max 6.5) | 50.4ms (mean 88.7, min 46.8, max 403.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 52.0ms (mean 90.4, min 48.6, max 404.6) | 1.7ms (mean 1.7, min 1.5, max 1.8) | 50.4ms (mean 88.7, min 46.8, max 403.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.4ms (mean 1.7, min 0.8, max 4.4) | 1.4ms (mean 1.7, min 0.8, max 4.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.7ms (mean 0.9, min 0.5, max 3.2) | 0.7ms (mean 0.9, min 0.5, max 3.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.1ms (mean 0.1, min 0.0, max 0.1) | 0.1ms (mean 0.1, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 59.0ms (mean 59.2, min 55.7, max 64.2) | 23.6ms (mean 24.3, min 23.2, max 29.7) | 34.6ms (mean 34.8, min 32.2, max 37.9) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
