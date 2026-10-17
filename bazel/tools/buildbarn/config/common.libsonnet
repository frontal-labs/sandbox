// Shared settings for the local Buildbarn stack.
//
// Adapted from buildbarn/bb-deployments, docker-compose/config (Apache-2.0),
// reduced to one storage node. See bazel/tools/buildbarn/README.md.
//
// Every access policy here is `allow: {}`. The stack has no authentication,
// so publish it only on loopback or behind a gateway that authenticates.

local storageAddress = 'storage:8981';
local storage = { grpc: { client: { address: storageAddress } } };

{
  storageAddress: storageAddress,
  maximumMessageSizeBytes: 2 * 1024 * 1024,

  blobstore: {
    contentAddressableStorage: storage,
    actionCache: {
      completenessChecking: {
        backend: storage,
        maximumTotalTreeSizeBytes: 64 * 1024 * 1024,
      },
    },
  },

  fileSystemAccessCache: storage,

  global: {
    diagnosticsHttpServer: {
      httpServers: [{
        listenAddresses: [':80'],
        authenticationPolicy: { allow: {} },
      }],
      enablePrometheus: true,
      // pprof exposes process internals. Leave it off unless you are profiling.
      enablePprof: false,
      enableActiveSpans: true,
    },
  },
}
