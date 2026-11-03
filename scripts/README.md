# Scripts

Utility scripts for development, release, and benchmarking.

## Contents

| Script | Purpose |
|--------|---------|
| `scripts/release/publish-sdk.sh` | Publishes SDK artifacts to GHCR for all languages |
| `scripts/sync-sdk-versions.mjs` | Synchronizes version numbers across SDKs and the workspace |
| `scripts/bench/update_baselines.sh` | Updates criterion benchmark baselines |
| `scripts/bench/criterion_to_json.py` | Converts criterion output to JSON for CI |

## Usage

Run scripts from the repository root:

```bash
# Publish SDKs (requires oras, GHCR auth)
./scripts/release/publish-sdk.sh 0.1.0

# Sync versions after a release
./scripts/sync-sdk-versions.mjs 0.1.1

# Update benchmarks
./scripts/bench/update_baselines.sh
```

## License

Apache-2.0