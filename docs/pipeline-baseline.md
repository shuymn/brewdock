# Pipeline Baseline

_Aggregated across 10 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 0.89s (mean 1.08s, min 0.80s, max 2.61s) | fetch-formula-index 762.0ms, persist-formula-index 97.5ms | - |
| install-tree | 0.88s (mean 0.98s, min 0.85s, max 1.52s) | acquire-payload 797.7ms, download-bottle 777.7ms, resolve-install-list 60.4ms | - |
| install-jq-wget | 3.52s (mean 3.84s, min 3.25s, max 5.44s) | acquire-payload 3005.0ms, download-bottle 2477.6ms, materialize-payload 1641.9ms | materialize-payload 1146.6ms |
| upgrade-dry-run-jq | 0.07s (mean 0.07s, min 0.06s, max 0.07s) | collect-upgrade-candidates 58.1ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 762.0ms (mean 960.9, min 693.4, max 2508.5) | 67.4ms (mean 67.0, min 60.4, max 74.8) | 696.0ms (mean 893.9, min 626.0, max 2440.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 97.5ms (mean 104.7, min 95.0, max 167.0) | 97.4ms (mean 104.6, min 95.0, max 167.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| acquire-payload | 797.7ms (mean 828.1, min 771.8, max 977.7) | 24.5ms (mean 24.2, min 12.1, max 34.7) | 770.0ms (mean 803.9, min 749.0, max 957.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 777.7ms (mean 811.3, min 758.4, max 962.0) | 9.6ms (mean 9.1, min 3.7, max 14.5) | 768.5ms (mean 802.2, min 748.0, max 955.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 60.4ms (mean 142.6, min 58.0, max 727.2) | 23.6ms (mean 23.7, min 22.8, max 24.7) | 36.6ms (mean 118.8, min 34.6, max 704.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 8.2ms (mean 9.4, min 6.0, max 13.5) | 8.2ms (mean 9.4, min 6.0, max 13.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 3.9ms (mean 3.7, min 1.7, max 5.4) | 3.9ms (mean 3.7, min 1.7, max 5.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1.1ms (mean 1.4, min 0.9, max 2.6) | 1.1ms (mean 1.4, min 0.9, max 2.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.4ms (mean 0.4, min 0.2, max 0.5) | 0.4ms (mean 0.4, min 0.2, max 0.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.4ms (mean 0.4, min 0.2, max 0.6) | 0.4ms (mean 0.4, min 0.2, max 0.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.1ms (mean 0.2, min 0.1, max 0.3) | 0.1ms (mean 0.2, min 0.1, max 0.3) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| acquire-payload | 3005.0ms (mean 3090.2, min 2752.8, max 3960.2) | 1026.5ms (mean 1032.7, min 1002.1, max 1069.9) | 8536.5ms (mean 8741.1, min 7959.0, max 10101.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 2477.6ms (mean 2498.0, min 2036.0, max 2933.1) | 73.5ms (mean 76.1, min 64.3, max 97.5) | 6670.0ms (mean 6767.4, min 5946.0, max 7473.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1641.9ms (mean 1603.3, min 1345.7, max 1712.7) | 1819.0ms (mean 1825.1, min 1724.0, max 1976.7) | 0.2ms (mean 0.2, min 0.2, max 0.5) | 1146.6ms (mean 1159.0, min 1085.4, max 1264.2) |
| extract-bottle | 857.2ms (mean 860.0, min 840.4, max 881.4) | 857.2ms (mean 860.0, min 840.4, max 881.4) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 279.8ms (mean 289.6, min 273.9, max 324.5) | 279.8ms (mean 289.6, min 273.9, max 324.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 154.0ms (mean 351.4, min 143.3, max 1163.1) | 23.8ms (mean 24.0, min 23.1, max 25.7) | 129.5ms (mean 327.4, min 120.0, max 1140.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 86.8ms (mean 86.6, min 68.4, max 106.1) | 86.7ms (mean 86.6, min 68.4, max 106.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 52.6ms (mean 88.4, min 49.9, max 413.3) | 4.0ms (mean 4.0, min 3.5, max 4.5) | 48.8ms (mean 84.4, min 46.4, max 409.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 50.4ms (mean 86.1, min 48.0, max 410.7) | 1.7ms (mean 1.7, min 1.5, max 2.0) | 48.8ms (mean 84.4, min 46.4, max 409.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.3ms (mean 1.4, min 1.1, max 1.8) | 1.3ms (mean 1.4, min 1.1, max 1.8) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.7ms (mean 0.7, min 0.5, max 0.8) | 0.7ms (mean 0.7, min 0.5, max 0.8) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.1ms (mean 0.1, min 0.1, max 0.1) | 0.1ms (mean 0.1, min 0.1, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 58.1ms (mean 58.7, min 56.1, max 62.6) | 24.0ms (mean 24.2, min 23.4, max 25.8) | 34.0ms (mean 34.5, min 32.1, max 38.3) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
