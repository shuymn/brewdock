# Pipeline Baseline

| Scenario | Wall | Top Phases |
|---|---:|---|
| update | 2.27s | persist-formula-index 93.5ms, fetch-formula-index 61.1ms |
| install-tree | 1.17s | resolve-install-list 21.9ms, prefetch-payload 17.4ms, store-bottle-blob 8.2ms |
| install-jq-wget | 6.15s | materialize-payload 2759.9ms, prefetch-payload 1009.2ms, extract-bottle 858.8ms |
| upgrade-dry-run-jq | 0.07s | collect-upgrade-candidates 22.6ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms |

## Phase Breakdown

### update

| Phase | Busy Time |
|---|---:|
| persist-formula-index | 93.5ms |
| fetch-formula-index | 61.1ms |

### install-tree

| Phase | Busy Time |
|---|---:|
| resolve-install-list | 21.9ms |
| prefetch-payload | 17.4ms |
| store-bottle-blob | 8.2ms |
| download-bottle | 6.7ms |
| extract-bottle | 1.8ms |
| materialize-payload | 1.8ms |
| finalize-install | 0.2ms |
| post-install | 0.1ms |
| check-blob-store | 0.0ms |
| resolve-install-method | 0.0ms |
| fetch-post-install-source | 0.0ms |

### install-jq-wget

| Phase | Busy Time |
|---|---:|
| materialize-payload | 2759.9ms |
| prefetch-payload | 1009.2ms |
| extract-bottle | 858.8ms |
| finalize-install | 294.3ms |
| download-bottle | 82.0ms |
| store-bottle-blob | 58.8ms |
| resolve-install-list | 22.5ms |
| post-install | 7.4ms |
| run-post-install | 4.5ms |
| fetch-post-install-source | 1.5ms |
| check-blob-store | 0.0ms |
| resolve-install-method | 0.0ms |

### upgrade-dry-run-jq

| Phase | Busy Time |
|---|---:|
| collect-upgrade-candidates | 22.6ms |
| discover-installed-kegs | 0.0ms |
| check-post-install-viability | 0.0ms |
