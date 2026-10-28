FROM sandbox-base AS sandbox-node

ARG NODE_ARCH=x64
ARG NODE_MAJOR
ARG NODE_VERSION

RUN mkdir -p /sandbox/runtimes/node${NODE_MAJOR} && curl https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-$NODE_ARCH.tar.gz | tar -zx --strip-components=1 -C /sandbox/runtimes/node${NODE_MAJOR} && /sandbox/runtimes/node${NODE_MAJOR}/bin/node --version

ENV PATH="/sandbox/runtimes/node${NODE_MAJOR}/bin:${PATH}"

RUN npm install -g --ignore-scripts pnpm@10.34.9

USER sandbox

RUN mkdir -p ~/.global/{npm,pnpm} && \
  npm config set prefix ~/.global/npm && \
  pnpm config set global-dir ~/.global/pnpm && \
  pnpm config set global-bin-dir ~/.global/pnpm/bin

ENV PATH="/sandbox/runtimes/node${NODE_MAJOR}/bin:/home/sandbox/.global/pnpm/bin:/home/sandbox/.global/npm/bin:${PATH}"