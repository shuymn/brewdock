# Pipeline Baseline

_Aggregated across 10 run(s); scenario wall and phase timings use median, with mean/min/max shown for spread._

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 0.84s (mean 1.02s, min 0.79s, max 2.31s) | fetch-formula-index 719.0ms, persist-formula-index 96.7ms | - |
| install-tree | 0.88s (mean 0.90s, min 0.81s, max 1.17s) | acquire-payload 800.8ms, download-bottle 779.1ms, resolve-install-list 60.2ms | - |
| install-jq-wget | 3.09s (mean 3.46s, min 2.87s, max 5.79s) | acquire-payload 2616.4ms, download-bottle 2324.7ms, extract-bottle 785.3ms | materialize-payload 54.3ms |
| upgrade-dry-run-jq | 0.07s (mean 0.07s, min 0.06s, max 0.08s) | collect-upgrade-candidates 58.2ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| fetch-formula-index | 719.0ms (mean 895.3, min 680.2, max 2185.3) | 65.6ms (mean 66.6, min 59.8, max 75.0) | 647.0ms (mean 828.7, min 620.0, max 2120.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| persist-formula-index | 96.7ms (mean 107.2, min 89.5, max 176.0) | 96.7ms (mean 107.2, min 89.5, max 176.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| acquire-payload | 800.8ms (mean 810.7, min 748.0, max 928.0) | 29.6ms (mean 27.6, min 14.0, max 37.4) | 768.0ms (mean 783.1, min 734.0, max 906.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 779.1ms (mean 791.8, min 737.8, max 914.3) | 10.5ms (mean 10.2, min 3.8, max 15.7) | 766.5ms (mean 781.6, min 734.0, max 905.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 60.2ms (mean 77.8, min 56.5, max 232.6) | 23.9ms (mean 24.9, min 21.3, max 36.6) | 35.9ms (mean 53.0, min 33.4, max 209.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 12.6ms (mean 11.8, min 7.8, max 14.6) | 12.6ms (mean 11.8, min 7.8, max 14.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 3.4ms (mean 3.7, min 1.6, max 6.0) | 3.4ms (mean 3.7, min 1.6, max 6.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 1.4ms (mean 1.5, min 0.6, max 2.9) | 1.4ms (mean 1.5, min 0.6, max 2.8) | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 0.5ms (mean 0.4, min 0.2, max 0.5) | 0.5ms (mean 0.4, min 0.2, max 0.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| finalize-install | 0.4ms (mean 0.4, min 0.2, max 0.5) | 0.4ms (mean 0.4, min 0.2, max 0.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 0.2ms (mean 0.2, min 0.1, max 0.5) | 0.2ms (mean 0.2, min 0.1, max 0.5) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| acquire-payload | 2616.4ms (mean 2763.0, min 2418.9, max 3584.9) | 995.6ms (mean 1012.3, min 924.2, max 1131.1) | 7337.5ms (mean 7421.4, min 6921.0, max 8178.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| download-bottle | 2324.7ms (mean 2344.3, min 2006.6, max 2876.7) | 79.6ms (mean 81.9, min 67.6, max 105.2) | 6380.0ms (mean 6540.0, min 6077.0, max 7427.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| extract-bottle | 785.3ms (mean 802.8, min 757.8, max 918.9) | 785.3ms (mean 802.8, min 757.8, max 918.9) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| materialize-payload | 588.2ms (mean 611.7, min 544.8, max 776.4) | 867.4ms (mean 850.3, min 677.7, max 942.2) | 0.2ms (mean 0.2, min 0.1, max 0.5) | 54.3ms (mean 53.9, min 47.3, max 58.1) |
| finalize-install | 254.6ms (mean 255.4, min 230.1, max 280.8) | 254.6ms (mean 255.4, min 230.1, max 280.7) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-list | 149.1ms (mean 331.3, min 144.7, max 1973.3) | 23.1ms (mean 23.3, min 22.0, max 25.7) | 126.0ms (mean 308.0, min 119.0, max 1950.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| store-bottle-blob | 126.7ms (mean 118.7, min 82.4, max 153.6) | 126.7ms (mean 118.7, min 82.4, max 153.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| post-install | 53.4ms (mean 97.0, min 49.8, max 481.5) | 3.4ms (mean 4.0, min 3.1, max 6.9) | 49.2ms (mean 93.1, min 46.5, max 476.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| fetch-post-install-source | 50.6ms (mean 94.6, min 48.0, max 477.5) | 1.5ms (mean 1.5, min 1.4, max 1.6) | 49.2ms (mean 93.1, min 46.5, max 476.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| plan-execution | 1.6ms (mean 1.7, min 1.0, max 2.3) | 1.6ms (mean 1.7, min 1.0, max 2.3) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| run-post-install | 0.5ms (mean 1.1, min 0.5, max 4.0) | 0.5ms (mean 1.1, min 0.5, max 4.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-blob-store | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| resolve-install-method | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---|---|---|---|
| collect-upgrade-candidates | 58.2ms (mean 59.1, min 55.1, max 66.5) | 23.6ms (mean 23.9, min 22.7, max 25.1) | 35.0ms (mean 35.2, min 31.3, max 41.6) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| discover-installed-kegs | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.1) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
| check-post-install-viability | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) | 0.0ms (mean 0.0, min 0.0, max 0.0) |
