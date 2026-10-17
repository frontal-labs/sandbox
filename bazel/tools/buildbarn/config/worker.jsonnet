// Worker: takes actions from the scheduler and runs them in a Linux runner.
// Used only by the `exec` compose profile.
//
// Adapted from buildbarn/bb-deployments, docker-compose/config (Apache-2.0),
// the hardlinking variant. That variant needs no privileges. The FUSE variant
// in the reference is faster but needs a privileged container.
//
// The platform properties below declare Linux. The build in this repository
// targets x86_64-apple-darwin, so no action from it matches this worker. See
// bazel/README.md.

local common = import 'common.libsonnet';

{
  blobstore: common.blobstore,
  maximumMessageSizeBytes: common.maximumMessageSizeBytes,
  scheduler: { address: 'scheduler:8983' },
  global: common.global,

  buildDirectories: [{
    native: {
      buildDirectoryPath: '/worker/build',
      cacheDirectoryPath: '/worker/cache',
      maximumCacheFileCount: 10000,
      maximumCacheSizeBytes: 1024 * 1024 * 1024,
      cacheReplacementPolicy: 'LEAST_RECENTLY_USED',
    },
    runners: [{
      endpoint: { address: 'unix:///worker/runner' },
      concurrency: 4,
      instanceNamePrefix: 'local',
      platform: {
        properties: [
          { name: 'OSFamily', value: 'linux' },
          { name: 'container-image', value: 'docker://ghcr.io/catthehacker/ubuntu:act-22.04@sha256:dd7654ffb01d5b7b54b23b9ce928a1f7f2d08c7b3d7e320b6574b55d7ccde78b' },
        ],
      },
      workerId: {
        datacenter: 'local',
        rack: '0',
        slot: '0',
        hostname: 'buildbarn-worker',
      },
    }],
  }],

  inputDownloadConcurrency: 10,
  outputUploadConcurrency: 11,

  directoryCache: {
    maximumCount: 1000,
    maximumSizeBytes: 1000 * 1024,
    cacheReplacementPolicy: 'LEAST_RECENTLY_USED',
  },
}
