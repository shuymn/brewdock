# Pipeline Baseline

| Scenario | Wall | Top Phases |
|---|---:|---|
| update | 2.04s | fetch-formula-index 63.9ms, persist-formula-index 15.3ms |
| install-tree | 1.20s | resolve-install-list 22.2ms, install-payload 18.3ms, store-bottle-blob 7.3ms |
| install-jq-wget | 11.00s | install-payload 3313.2ms, relocate-keg 1715.9ms, extract-bottle 838.9ms |
| upgrade-dry-run-jq | 0.07s | collect-upgrade-candidates 24.1ms, fetch-formula-metadata 22.4ms, discover-installed-kegs 0.0ms |

## Phase Breakdown

### update

| Phase | Busy Time |
|---|---:|
| fetch-formula-index | 63.9ms |
| persist-formula-index | 15.3ms |

### install-tree

| Phase | Busy Time |
|---|---:|
| resolve-install-list | 22.2ms |
| install-payload | 18.3ms |
| store-bottle-blob | 7.3ms |
| download-bottle | 7.2ms |
| extract-bottle | 1.6ms |
| relocate-keg | 0.7ms |
| materialize-keg | 0.7ms |
| finalize-install | 0.2ms |
| post-install | 0.1ms |
| resolve-install-method | 0.0ms |
| fetch-post-install-source | 0.0ms |

### install-jq-wget

| Phase | Busy Time |
|---|---:|
| install-payload | 3313.2ms |
| relocate-keg | 1715.9ms |
| extract-bottle | 838.9ms |
| materialize-keg | 602.9ms |
| finalize-install | 262.0ms |
| store-bottle-blob | 86.7ms |
| download-bottle | 58.1ms |
| resolve-install-list | 22.8ms |
| post-install | 10.0ms |
| run-post-install | 3.0ms |
| fetch-post-install-source | 1.8ms |
| resolve-install-method | 0.0ms |

### upgrade-dry-run-jq

| Phase | Busy Time |
|---|---:|
| collect-upgrade-candidates | 24.1ms |
| fetch-formula-metadata | 22.4ms |
| discover-installed-kegs | 0.0ms |
| check-post-install-viability | 0.0ms |
