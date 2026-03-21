# Pipeline Baseline

_Aggregated across 3 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 1.20s (mean 1.46s, min 0.90s, max 2.29s) | fetch-formula-index 1091.4ms, persist-formula-index 94.7ms | - |
| install-tree | 0.53s (mean 0.64s, min 0.52s, max 0.88s) | prefetch-payload 455.8ms, download-bottle 443.8ms, resolve-install-list 58.7ms | - |
| install-jq-wget | 5.32s (mean 4.85s, min 3.80s, max 5.44s) | prefetch-payload 2036.7ms, download-bottle 1988.9ms, materialize-payload 1168.7ms | materialize-payload 845.7ms |
| upgrade-dry-run-jq | 0.07s (mean 0.07s, min 0.07s, max 0.07s) | collect-upgrade-candidates 59.2ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 1091.4ms (mean 1352.6, min 803.2, max 2163.2) | 71.4ms (mean 68.3, min 60.2, max 73.2) | 1020.0ms (mean 1284.3, min 743.0, max 2090.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 94.7ms (mean 97.0, min 89.3, max 107.0) | 94.7ms (mean 97.0, min 89.3, max 107.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 455.8ms (mean 521.1, min 455.0, max 652.6) | 20.6ms (mean 20.1, min 15.8, max 24.0) | 440.0ms (mean 501.0, min 431.0, max 632.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 443.8ms (mean 506.2, min 436.4, max 638.6) | 5.4ms (mean 5.2, min 3.8, max 6.6) | 440.0ms (mean 501.0, min 431.0, max 632.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 58.7ms (mean 109.7, min 54.0, max 216.4) | 22.4ms (mean 22.2, min 21.8, max 22.4) | 36.3ms (mean 87.5, min 32.2, max 194.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 9.9ms (mean 10.2, min 8.0, max 12.7) | 9.9ms (mean 10.2, min 8.0, max 12.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 3.4ms (mean 3.5, min 3.0, max 4.1) | 3.4ms (mean 3.5, min 3.0, max 4.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 3.0ms (mean 3.3, min 2.6, max 4.3) | 3.0ms (mean 3.2, min 2.5, max 4.1) | 0.0ms (mean 0.1, min 0.0, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.2ms (mean 0.2, min 0.2, max 0.4) | 0.2ms (mean 0.2, min 0.2, max 0.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.2ms (mean 0.2, min 0.1, max 0.2) | 0.2ms (mean 0.2, min 0.1, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.1ms (mean 0.1, min 0.1, max 0.2) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| prefetch-payload | 2036.7ms (mean 2525.5, min 1933.1, max 3606.9) | 967.1ms (mean 950.2, min 901.8, max 981.6) | 5525.0ms (mean 5777.7, min 4476.0, max 7332.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 1988.9ms (mean 2184.2, min 1451.3, max 3112.4) | 72.5ms (mean 72.7, min 63.3, max 82.1) | 5526.0ms (mean 5778.0, min 4476.0, max 7332.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1168.7ms (mean 1192.7, min 1150.0, max 1259.3) | 2273.1ms (mean 2259.2, min 2152.3, max 2352.2) | 0.2ms (mean 0.2, min 0.2, max 0.2) | 845.7ms (mean 859.1, min 813.5, max 918.2) |
| extract-bottle | 792.6ms (mean 792.3, min 767.4, max 816.9) | 792.6ms (mean 792.3, min 767.4, max 816.8) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 299.8ms (mean 282.3, min 242.6, max 304.4) | 299.8ms (mean 282.3, min 242.6, max 304.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 263.4ms (mean 257.3, min 52.9, max 455.6) | 3.6ms (mean 4.6, min 3.4, max 6.6) | 260.0ms (mean 252.8, min 49.3, max 449.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 261.3ms (mean 254.1, min 50.8, max 450.3) | 1.3ms (mean 1.4, min 1.3, max 1.5) | 260.0ms (mean 252.8, min 49.3, max 449.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 146.8ms (mean 581.6, min 144.5, max 1453.5) | 23.5ms (mean 23.3, min 22.5, max 23.8) | 123.0ms (mean 558.3, min 122.0, max 1430.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 70.8ms (mean 77.1, min 65.4, max 95.0) | 70.8ms (mean 77.1, min 65.4, max 95.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.3ms (mean 1.1, min 0.8, max 1.4) | 1.3ms (mean 1.1, min 0.8, max 1.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.6ms (mean 1.7, min 0.6, max 3.9) | 0.6ms (mean 1.7, min 0.6, max 3.9) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.1ms (mean 0.0, min 0.0, max 0.1) | 0.1ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 59.2ms (mean 59.4, min 58.9, max 60.0) | 22.2ms (mean 22.5, min 22.1, max 23.1) | 36.7ms (mean 36.9, min 36.1, max 37.9) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
