# Project Avalanche dedicated server in a container: built from this repository's source, the data (the rules subset of
# your own disc, the server's keys, its logs) in a volume, never in the image. From the repository's root:
#   docker compose -f Engine/assets/server/deploy/compose.yaml build
#   docker compose -f Engine/assets/server/deploy/compose.yaml run --rm \
#     -v /path/to/your/ssx.iso:/iso/ssx.iso:ro avalanche-server setup /iso/ssx.iso --out /srv/avalanche/data
#   docker compose -f Engine/assets/server/deploy/compose.yaml up -d
# LAN discovery and port mapping see the container's network, not your LAN: with them on, use the host's network
# (compose.yaml says how). The build context is cut to what the build reads by Dockerfile.dockerignore beside this file
# (BuildKit).
# Rust 1.99: the version this image is built and tested with (round net4c); the workspace's minimum is its
# `rust-version` (Engine/Cargo.toml), checked with `cargo check --workspace --all-targets` in a container
ARG BUILD_IMAGE=rust:1.99.0-slim-bookworm
ARG RUN_IMAGE=debian:bookworm-slim

FROM ${BUILD_IMAGE} AS build
# a C++ compiler: the server's script host builds Luau from source (mlua-sys through luau0-src, cc-rs); the slim image
# carries gcc alone, and without g++ the build stops at mlua-sys ("failed to find tool \"c++\"", round net4c)
RUN apt-get update \
    && apt-get install -y --no-install-recommends g++ \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked --manifest-path Engine/Cargo.toml -p avalanche-server

FROM ${RUN_IMAGE}
# the system's trusted roots: the heartbeat to an https:// master reads them (rustls-native-certs); without them every
# heartbeat to the project's master fails (round net4c)
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --home-dir /srv/avalanche --shell /usr/sbin/nologin avalanche \
    && install -d -o avalanche -g avalanche -m 0750 /srv/avalanche /srv/avalanche/data
COPY --from=build /src/Engine/target/release/avalanche-server /usr/local/bin/avalanche-server
COPY Engine/assets/server/THIRD_PARTY_NOTICES.txt /usr/share/doc/avalanche-server/THIRD_PARTY_NOTICES.txt
USER avalanche
WORKDIR /srv/avalanche
VOLUME ["/srv/avalanche/data"]
EXPOSE 27450/udp 27450/tcp 27451/udp
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/avalanche-server"]
CMD ["run", "--config", "/srv/avalanche/server.toml", "--data", "/srv/avalanche/data"]
