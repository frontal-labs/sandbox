// Frontend: the gRPC endpoint Bazel connects to on port 8980.
//
// Adapted from buildbarn/bb-deployments, docker-compose/config (Apache-2.0).
// It serves the remote cache directly. The scheduler is needed only for
// remote execution, which the `exec` compose profile starts.

local common = import 'common.libsonnet';

{
  grpcServers: [{
    listenAddresses: [':8980'],
    authenticationPolicy: { allow: {} },
  }],

  // Used only by Execute. The cache path never reaches it.
  schedulers: {
    '': {
      endpoint: {
        address: 'scheduler:8982',
        addMetadataJmespathExpression: {
          expression: |||
            {
              "build.bazel.remote.execution.v2.requestmetadata-bin": incomingGRPCMetadata."build.bazel.remote.execution.v2.requestmetadata-bin"
            }
          |||,
        },
      },
    },
  },

  maximumMessageSizeBytes: common.maximumMessageSizeBytes,
  global: common.global,

  contentAddressableStorage: {
    backend: common.blobstore.contentAddressableStorage,
    getAuthorizer: { allow: {} },
    putAuthorizer: { allow: {} },
    findMissingAuthorizer: { allow: {} },
  },

  actionCache: {
    backend: common.blobstore.actionCache,
    getAuthorizer: { allow: {} },
    putAuthorizer: { allow: {} },
  },

  fileSystemAccessCache: {
    backend: common.fileSystemAccessCache,
    getAuthorizer: { allow: {} },
    putAuthorizer: { allow: {} },
  },

  executeAuthorizer: { allow: {} },
}
