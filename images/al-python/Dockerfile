FROM builder-base AS builder-python

ARG PYTHON_VERSION=3.13.1

RUN mkdir -p /sandbox/runtimes/python && \
  wget "https://www.python.org/ftp/python/$PYTHON_VERSION/Python-$PYTHON_VERSION.tgz" && \
  tar -xvf "Python-$PYTHON_VERSION.tgz" && \
  cd "Python-$PYTHON_VERSION" && \
  ./configure --enable-optimizations --prefix=/sandbox/runtimes/python && \
  make -j "$(nproc)" && \
  make install && \
  cd .. && \
  rm -rf "Python-$PYTHON_VERSION" "Python-$PYTHON_VERSION.tgz"

ENV PATH="/opt/git/bin:/sandbox/runtimes/python/bin:/home/sandbox/.local/bin:/usr/local/bin:${PATH}"

RUN python3 -m pip install --upgrade pip && \
  pip install uv

RUN rm -rf \
  /sandbox/runtimes/python/lib/python3.13/test \
  /sandbox/runtimes/python/lib/python3.13/__pycache__ \
  /sandbox/runtimes/python/lib/python3.13/ensurepip \
  /sandbox/runtimes/python/lib/python3.13/idlelib \
  /sandbox/runtimes/python/lib/python3.13/tkinter \
  /sandbox/runtimes/python/lib/python3.13/distutils \
  /sandbox/runtimes/python/lib/pkgconfig \
  /sandbox/runtimes/python/share

# Convenience symlink so `python`, `python3` and `python3.13` all work.
RUN ln --relative --symbolic /sandbox/runtimes/python/bin/python3 /sandbox/runtimes/python/bin/python

FROM sandbox-base AS sandbox-python

# Copy compiled Python and git from builder stage
COPY --from=builder-python /sandbox/runtimes/python /sandbox/runtimes/python

ENV PATH="/sandbox/runtimes/python/bin:${PATH}"

USER sandbox

RUN python3 --version && \
  pip3 --version && \
  python3 -m venv -h && \
  uv --version