// Storage node: holds the content-addressable store, the action cache, and the
// file-system access cache on local block devices.
//
// Adapted from buildbarn/bb-deployments, docker-compose/config (Apache-2.0).
// Sizes are for a developer machine. Raise sizeBytes for a shared cache.

local common = import 'common.libsonnet';

// One local store backed by a block-device file under `dir`.
// newBlocks must be 1 for the action cache and the file-system access cache.
// The CAS accepts more, so each caller sets it.
local localStore(dir, blockBytes, keyMapBytes, newBlocks) = {
  backend: {
    'local': {
      keyLocationMapOnBlockDevice: {
        file: {
          path: dir + '/key_location_map',
          sizeBytes: keyMapBytes,
        },
      },
      keyLocationMapMaximumGetAttempts: 16,
      keyLocationMapMaximumPutAttempts: 64,
      oldBlocks: 8,
      currentBlocks: 24,
      newBlocks: newBlocks,
      blocksOnBlockDevice: {
        source: {
          file: {
            path: dir + '/blocks',
            sizeBytes: blockBytes,
          },
        },
        spareBlocks: 3,
      },
      persistent: {
        stateDirectoryPath: dir + '/persistent_state',
        minimumEpochInterval: '300s',
      },
    },
  },
  getAuthorizer: { allow: {} },
  putAuthorizer: { allow: {} },
};

{
  grpcServers: [{
    listenAddresses: [':8981'],
    authenticationPolicy: { allow: {} },
  }],
  maximumMessageSizeBytes: common.maximumMessageSizeBytes,
  global: common.global,

  // Only the CAS has a findMissing authorizer. The other two stores do not.
  contentAddressableStorage: localStore('/storage-cas', 8 * 1024 * 1024 * 1024, 400 * 1024 * 1024, 3) + {
    findMissingAuthorizer: { allow: {} },
  },

  actionCache: localStore('/storage-ac', 256 * 1024 * 1024, 1024 * 1024, 1),

  fileSystemAccessCache: localStore('/storage-fsac', 20 * 1024 * 1024, 1024 * 1024, 1),
}
