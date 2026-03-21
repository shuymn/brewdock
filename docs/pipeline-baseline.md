# Pipeline Baseline

| Scenario | Wall | Top Wall Phases | Top Child Phases |
|---|---:|---|---|
| update | 1.26s | fetch-formula-index 1137.9ms, persist-formula-index 105.0ms | - |
| install-tree | 0.53s | prefetch-payload 460.8ms, download-bottle 441.4ms, resolve-install-list 58.0ms | - |
| install-jq-wget | 3.81s | prefetch-payload 1839.0ms, download-bottle 1557.9ms, materialize-payload 1410.1ms | materialize-payload 1034.9ms |
| upgrade-dry-run-jq | 0.07s | collect-upgrade-candidates 57.8ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms | - |

## Phase Breakdown

### update

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---:|---:|---:|---:|
| fetch-formula-index | 1137.9ms | 67.9ms | 1070.0ms | 0.0ms |
| persist-formula-index | 105.0ms | 105.0ms | 0.0ms | 0.0ms |

### install-tree

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---:|---:|---:|---:|
| prefetch-payload | 460.8ms | 25.8ms | 435.0ms | 0.0ms |
| download-bottle | 441.4ms | 6.4ms | 435.0ms | 0.0ms |
| resolve-install-list | 58.0ms | 23.3ms | 34.7ms | 0.0ms |
| store-bottle-blob | 16.8ms | 16.8ms | 0.0ms | 0.0ms |
| materialize-payload | 2.1ms | 2.1ms | 0.0ms | 0.0ms |
| extract-bottle | 1.7ms | 1.6ms | 0.0ms | 0.0ms |
| finalize-install | 0.2ms | 0.2ms | 0.0ms | 0.0ms |
| post-install | 0.1ms | 0.1ms | 0.0ms | 0.0ms |
| check-blob-store | 0.0ms | 0.0ms | 0.0ms | 0.0ms |
| resolve-install-method | 0.0ms | 0.0ms | 0.0ms | 0.0ms |
| fetch-post-install-source | 0.0ms | 0.0ms | 0.0ms | 0.0ms |

### install-jq-wget

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---:|---:|---:|---:|
| prefetch-payload | 1839.0ms | 1069.1ms | 5615.0ms | 0.0ms |
| download-bottle | 1557.9ms | 81.2ms | 5615.0ms | 0.0ms |
| materialize-payload | 1410.1ms | 3014.4ms | 2.5ms | 1034.9ms |
| extract-bottle | 901.5ms | 901.5ms | 0.0ms | 0.0ms |
| finalize-install | 335.8ms | 335.8ms | 0.0ms | 0.0ms |
| resolve-install-list | 151.7ms | 24.7ms | 127.0ms | 0.0ms |
| store-bottle-blob | 69.5ms | 69.5ms | 0.0ms | 0.0ms |
| post-install | 64.0ms | 6.7ms | 57.3ms | 0.0ms |
| fetch-post-install-source | 59.2ms | 1.8ms | 57.4ms | 0.0ms |
| run-post-install | 2.9ms | 2.9ms | 0.0ms | 0.0ms |
| check-blob-store | 0.0ms | 0.0ms | 0.0ms | 0.0ms |
| resolve-install-method | 0.0ms | 0.0ms | 0.0ms | 0.0ms |

### upgrade-dry-run-jq

| Phase | Wall Time | Busy Time | Idle Time | Child Process |
|---|---:|---:|---:|---:|
| collect-upgrade-candidates | 57.8ms | 23.8ms | 34.0ms | 0.0ms |
| discover-installed-kegs | 0.0ms | 0.0ms | 0.0ms | 0.0ms |
| check-post-install-viability | 0.0ms | 0.0ms | 0.0ms | 0.0ms |
