# Buildkite CI/CD for Frontal Sandbox

This directory defines the Buildkite pipeline for the Frontal Sandbox
repository. It runs `lint → build → test` on every push and PR, then
deploys to staging on the default branch and to production after an
approval block.

## Layout

```
.buildkite/
├── pipeline.yml          # the pipeline definition
├── hooks/
│   └── pre-command       # runs once per build: installs toolchains
└── scripts/
    ├── setup.sh          # shared toolchain install
    ├── lint.sh           # rustfmt + clippy + biome + ruff + shellcheck
    ├── build.sh          # bazel build //... + release CLI + TS SDK + docker
    ├── test.sh           # cargo test + bazel test + TS + Python, JUnit XML
    ├── deploy-staging.sh # build, push, roll out, smoke test
    └── deploy-prod.sh    # same flow for production
```

Every script starts with `set -euo pipefail` and is runnable locally.

## Upload the pipeline

```bash
buildkite-agent pipeline upload .buildkite/pipeline.yml
```

Configure the pipeline in the Buildkite UI (or API) to:

- enable **GitHub webhooks** and **PR builds**,
- set **default branch** to `main`,
- enable **signed pipelines** for the production deploy step.

## Pipeline graph

```
push / PR                         main branch
─────────                         ────────────
lint ──► build ──► test (parallel)      │
      ┌── rust tests                  │
      ├── bazel tests                │
      ├── TypeScript SDK tests       │
      └── Python SDK tests           │
            │                       │
            ▼                       ▼
      annotate ──► deploy-staging ──► block ──► deploy-prod
```

- `lint`, `build`, and the four test jobs use `if_changed` so a change to
  `crates/**` does not re-run the TypeScript tests.
- Test jobs run in parallel and `depends_on: build`; the build step
  publishes artifacts (`target/release/frontal-sandbox`, `dist/**`,
  `bazel-out/**`) that downstream steps consume.
- `deploy-staging` and `deploy-prod` only run on the default branch.
- `deploy-prod` is gated by a `block` step and runs with
  `concurrency: 1` + `concurrency_group` so only one prod deploy
  executes at a time.

## Secrets and credentials

**No secrets are stored in the repository.** At runtime, credentials come
from one of:

- `buildkite-agent secret get` — the `buildkite-agent` CLI reads secrets
  from the Buildkite Secrets Vault and exports them as environment
  variables for the step.
- The **Buildkite OIDC plugin** (`buildkite-oidc`) — the agent mints a
  short-lived JWT from the build's identity; the cloud provider exchanges
  it for credentials. This is the preferred path for AWS, GCP, and Azure.

The scripts reference these environment variables; set them as secrets
on the pipeline (or in the agent's environment):

| Variable             | Where used      | Purpose                                  |
| -------------------- | --------------- | ---------------------------------------- |
| `STAGING_REGISTRY`   | deploy-staging  | container registry for the staging image |
| `STAGING_NAMESPACE`  | deploy-staging  | registry namespace / org                  |
| `STAGING_BASE_URL`   | deploy-staging  | staging endpoint for the smoke test       |
| `STAGING_API_KEY`    | deploy-staging  | API key for the staging smoke test         |
| `PROD_REGISTRY`      | deploy-prod     | container registry for the prod image     |
| `PROD_NAMESPACE`     | deploy-prod     | registry namespace / org                  |
| `PROD_BASE_URL`      | deploy-prod     | prod endpoint for the smoke test           |
| `PROD_API_KEY`       | deploy-prod     | API key for the prod smoke test            |

## Agents and queues

The pipeline declares two queues:

- `default` — CI steps (lint, build, test). Any agent tagged with
  `queue=default` picks these up.
- `deploy` — the production deploy step. Only agents tagged with
  `queue=deploy` run it; tag them with the extra trust level production
  needs (e.g. a private network, KVM access for Tier 2 microVMs).

## Caching

- The `pre-command` hook warms the Bazel disk cache
  (`BUILDKITE_CACHE_DIR=/var/cache/bazel`) so external repositories and
  the Rust toolchain are not re-fetched on every build.
- `cargo` and `npm` caches live on the agent's persistent storage; the
  `setup.sh` script installs the pinned toolchains once and reuses them.

## Verification checklist

- [ ] Push to a feature branch → `buildkite-agent pipeline upload` runs
      lint, build, and the parallel tests.
- [ ] Open a PR → same flow; failing tests fail the build.
- [ ] Push to `main` → staging deploy runs automatically.
- [ ] Approve the `block` step → production deploy runs, with only one
      prod deploy in flight at a time.