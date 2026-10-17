// Runner: executes one action at a time inside the Ubuntu container. Used only
// by the `exec` compose profile.
//
// Adapted from buildbarn/bb-deployments, docker-compose/config (Apache-2.0).

local common = import 'common.libsonnet';

{
  buildDirectoryPath: '/worker/build',
  global: common.global,
  grpcServers: [{
    listenPaths: ['/worker/runner'],
    authenticationPolicy: { allow: {} },
  }],
}
