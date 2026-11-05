# syntax=docker/dockerfile:1
# Builds the Hub (Rust) with its Haskell acceptor library and the end-to-end
# Producer, then ships them in slim runtime images. Debian 13 (trixie) is the
# base because its libcrypto is 3.5; the eventchain libraries need >= 3.2.
# Nothing here is architecture-specific: ghcup, rustup and apt pick the build
# machine's architecture.

FROM debian:trixie-slim AS builder
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git xz-utils pkg-config patchelf \
        libgmp-dev libffi-dev libncurses-dev libnuma-dev zlib1g-dev libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# GHC 9.12.4 and cabal 3.16 through ghcup.
ENV GHCUP_INSTALL_BASE_PREFIX=/opt/ghcup
ENV PATH=/opt/ghcup/.ghcup/bin:/root/.cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://get-ghcup.haskell.org -o /tmp/ghcup-install.sh \
    && BOOTSTRAP_HASKELL_NONINTERACTIVE=1 BOOTSTRAP_HASKELL_MINIMAL=1 sh /tmp/ghcup-install.sh
RUN ghcup install ghc 9.12.4 --set && ghcup install cabal 3.16.1.0 --set \
    && rm -rf /opt/ghcup/.ghcup/cache /opt/ghcup/.ghcup/tmp \
    && ghc --version && cabal --version

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustc --version

WORKDIR /src
RUN cabal update

# Haskell dependencies: cached until a .cabal or cabal.project file changes.
# The Producer and the acceptor pin the same eventchain commit and share one
# cabal store, so building the Producer's dependencies builds the acceptor's.
# (The acceptor's own foreign-library depends on its local library, so cabal
# cannot build its dependencies without its sources.)
COPY tools/producer/cabal.project tools/producer/hub-producer.cabal tools/producer/
RUN cd tools/producer && cabal build --only-dependencies exe:producer

# Rust dependencies: cached until Cargo.toml or Cargo.lock changes. A stub
# crate with a no-op build script stands in for the Hub.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && echo 'fn main() {}' > build.rs \
    && cargo build --release && rm -rf src build.rs target/release/hub target/release/deps/hub-* target/release/.fingerprint/hub-*

# The sources.
COPY acceptor/ acceptor/
RUN cd acceptor && cabal build eventchain_acceptor
COPY tools/producer/ tools/producer/
RUN cd tools/producer && cabal build exe:producer
COPY build.rs ./
COPY src/ src/
RUN cargo build --release

# One directory holds everything the binaries load that the OS does not
# provide, and every ELF object finds its neighbours through $ORIGIN, so the
# runtime stage needs no knowledge of where cabal or ghcup put things.
RUN set -eu; mkdir -p /out/bin /out/lib; \
    STORE_LIBS=$(find /root/.local/state/cabal/store -name 'libHS*.so' -exec dirname {} \; | sort -u | tr '\n' ':'); \
    export LD_LIBRARY_PATH="$STORE_LIBS"; \
    ACC=$(ls acceptor/dist-newstyle/build/*/ghc-*/eventchain-acceptor-*/f/eventchain_acceptor/build/eventchain_acceptor/libeventchain_acceptor.so); \
    cp "$ACC" /out/lib/; \
    cp target/release/hub /out/bin/hub; \
    cp "$(cd tools/producer && cabal list-bin producer)" /out/bin/producer; \
    for round in 1 2 3; do \
      for f in /out/lib/*.so /out/bin/*; do \
        ldd "$f" | awk '/=> \//{print $3}' | while read -r dep; do \
          case "$dep" in /lib/*|/usr/lib/*) ;; /opt/ghcup/*|/root/.local/state/cabal/*|/src/*) cp -n "$dep" /out/lib/ ;; esac; \
        done; \
      done; \
    done; \
    patchelf --force-rpath --set-rpath '$ORIGIN' /out/lib/*.so; \
    patchelf --force-rpath --set-rpath '$ORIGIN/../lib' /out/bin/hub /out/bin/producer; \
    unset LD_LIBRARY_PATH; \
    for f in /out/lib/*.so /out/bin/*; do \
      if ldd "$f" | grep -q 'not found'; then echo "unresolved libraries in $f" >&2; ldd "$f" >&2; exit 1; fi; \
    done; \
    ls /out/lib

FROM debian:trixie-slim AS runtime-base
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        libgmp10 libffi8 libnuma1 libncurses6 zlib1g libssl3t64 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && dpkg-query -W -f='libssl3t64 ${Version}\n' libssl3t64
COPY --from=builder /out/lib /opt/hub/lib
COPY --from=builder /out/bin /opt/hub/bin
RUN ldd /opt/hub/bin/hub /opt/hub/bin/producer | grep -c 'not found' | grep -qx 0

# The Hub.
FROM runtime-base AS hub
RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data hub && mkdir /data && chown hub /data
ENV HUB_DATA=/data HUB_ADDR=0.0.0.0:3717
USER hub
EXPOSE 3717
ENTRYPOINT ["/opt/hub/bin/hub"]

# The demo's client: the Producer, plus curl, python3 and the openssl CLI for
# the statement-signature check.
FROM runtime-base AS tools
RUN apt-get update && apt-get install -y --no-install-recommends curl python3 openssl && rm -rf /var/lib/apt/lists/*
COPY tools/producer/verify-signature.py /opt/hub/bin/verify-signature.py
ENV PATH=/opt/hub/bin:$PATH
WORKDIR /work
