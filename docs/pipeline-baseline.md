# Pipeline Baseline

| Scenario | Wall | Top Phases |
|---|---:|---|
| update | 2.09s | persist-formula-index 108.0ms, fetch-formula-index 73.8ms |
| install-tree | 1.22s | resolve-install-list 23.5ms, prefetch-payload 16.2ms, download-bottle 6.8ms |
| install-jq-wget | 6.42s | materialize-payload 2905.4ms, prefetch-payload 989.5ms, extract-bottle 834.1ms |
| upgrade-dry-run-jq | 0.07s | collect-upgrade-candidates 23.6ms, discover-installed-kegs 0.0ms, check-post-install-viability 0.0ms |

## Phase Breakdown

### update

| Phase | Busy Time |
|---|---:|
| persist-formula-index | 108.0ms |
| fetch-formula-index | 73.8ms |

### install-tree

| Phase | Busy Time |
|---|---:|
| resolve-install-list | 23.5ms |
| prefetch-payload | 16.2ms |
| download-bottle | 6.8ms |
| store-bottle-blob | 6.6ms |
| extract-bottle | 1.9ms |
| materialize-payload | 1.7ms |
| finalize-install | 0.3ms |
| post-install | 0.1ms |
| check-blob-store | 0.0ms |
| resolve-install-method | 0.0ms |
| fetch-post-install-source | 0.0ms |

### install-jq-wget

| Phase | Busy Time |
|---|---:|
| materialize-payload | 2905.4ms |
| prefetch-payload | 989.5ms |
| extract-bottle | 834.1ms |
| finalize-install | 268.1ms |
| download-bottle | 78.7ms |
| store-bottle-blob | 66.8ms |
| resolve-install-list | 23.7ms |
| post-install | 6.9ms |
| run-post-install | 2.6ms |
| fetch-post-install-source | 1.8ms |
| check-blob-store | 0.0ms |
| resolve-install-method | 0.0ms |

### upgrade-dry-run-jq

| Phase | Busy Time |
|---|---:|
| collect-upgrade-candidates | 23.6ms |
| discover-installed-kegs | 0.0ms |
| check-post-install-viability | 0.0ms |
