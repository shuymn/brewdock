# Pipeline Baseline

| Scenario | Wall | Top Phases |
|---|---:|---|
| update | 2.09s | fetch-formula-index 69.7ms, persist-formula-index 19.4ms |
| install-tree | 1.20s | resolve-install-list 22.5ms, prefetch-payload 21.4ms, store-bottle-blob 11.5ms |
| install-jq-wget | 5.58s | materialize-payload 2844.3ms, prefetch-payload 962.2ms, extract-bottle 822.3ms |
| upgrade-dry-run-jq | 0.07s | collect-upgrade-candidates 23.6ms, fetch-formula-metadata 22.6ms, discover-installed-kegs 0.0ms |

## Phase Breakdown

### update

| Phase | Busy Time |
|---|---:|
| fetch-formula-index | 69.7ms |
| persist-formula-index | 19.4ms |

### install-tree

| Phase | Busy Time |
|---|---:|
| resolve-install-list | 22.5ms |
| prefetch-payload | 21.4ms |
| store-bottle-blob | 11.5ms |
| download-bottle | 7.1ms |
| materialize-payload | 2.0ms |
| extract-bottle | 1.9ms |
| finalize-install | 0.2ms |
| post-install | 0.1ms |
| check-blob-store | 0.0ms |
| resolve-install-method | 0.0ms |
| fetch-post-install-source | 0.0ms |

### install-jq-wget

| Phase | Busy Time |
|---|---:|
| materialize-payload | 2844.3ms |
| prefetch-payload | 962.2ms |
| extract-bottle | 822.3ms |
| finalize-install | 241.9ms |
| download-bottle | 74.8ms |
| store-bottle-blob | 58.5ms |
| resolve-install-list | 23.6ms |
| post-install | 6.3ms |
| run-post-install | 3.2ms |
| fetch-post-install-source | 1.6ms |
| check-blob-store | 0.0ms |
| resolve-install-method | 0.0ms |

### upgrade-dry-run-jq

| Phase | Busy Time |
|---|---:|
| collect-upgrade-candidates | 23.6ms |
| fetch-formula-metadata | 22.6ms |
| discover-installed-kegs | 0.0ms |
| check-post-install-viability | 0.0ms |
