# Benchmark Artifacts

- Use [`tests/vm-benchmark.sh`](../tests/vm-benchmark.sh) for package-manager comparison tables.
- Use [`tests/vm-pipeline-baseline.sh`](../tests/vm-pipeline-baseline.sh) for brewdock-only phase baselines on `bd update`, `bd install tree`, `bd install jq wget`, and `bd upgrade --dry-run jq`. When pipeline baselines are noisy, prefer `--runs 3` first and compare `median` before looking at `mean`.
- Prefer committing the generated markdown summary when a Theme depends on a fixed baseline or bottleneck snapshot.
- The pipeline script depends on `BREWDOCK_BENCHMARK_FILE` tracing output. Keep phase names stable unless the benchmark contract changes.
