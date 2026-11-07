# Project Avalanche master server in a container: built from this repository's source; its TLS files and its data
# (the logs) come from outside, never from the image. From the repository's root:
#   docker compose -f Engine/assets/master/deploy/compose.yaml build
#   docker compose -f Engine/assets/master/deploy/compose.yaml up -d
# Inside, the master listens on 8443 as an unprivileged user (no capability); compose.yaml publishes it on 443. Behind
# Coolify's proxy (Traefik or Caddy, which terminate TLS) the image runs as it is: master.toml without [tls], with
# [proxy] trusted, and no published port (server-procedure.md, "The master on Coolify"). The build context is cut to
# what the build reads by Dockerfile.dockerignore beside this file (BuildKit).
# Rust 1.99: the version this image is built and tested with (round net4c); the workspace's minimum is its
# `rust-version` (Engine/Cargo.toml), checked with `cargo check --workspace --all-targets` in a container
ARG BUILD_IMAGE=rust:1.99.0-slim-bookworm
ARG RUN_IMAGE=debian:bookworm-slim

FROM ${BUILD_IMAGE} AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked --manifest-path Engine/Cargo.toml -p avalanche-master

FROM ${RUN_IMAGE}
# uid and gid 10002 both: the host's deploy hook gives the TLS files this group (root owns them; the master reads them)
RUN groupadd --system --gid 10002 avalanche-master \
    && useradd --system --uid 10002 --gid 10002 --home-dir /srv/avalanche-master --shell /usr/sbin/nologin \
       avalanche-master \
    && install -d -o avalanche-master -g avalanche-master -m 0750 /srv/avalanche-master /srv/avalanche-master/data
COPY --from=build /src/Engine/target/release/avalanche-master /usr/local/bin/avalanche-master
COPY Engine/assets/master/THIRD_PARTY_NOTICES.txt /usr/share/doc/avalanche-master/THIRD_PARTY_NOTICES.txt
USER avalanche-master
WORKDIR /srv/avalanche-master
VOLUME ["/srv/avalanche-master/data"]
EXPOSE 8443/tcp
# the master asks itself on its port (`avalanche-master health`: GET /v1/health from loopback, HTTPS with [tls]); the
# image has no shell tool, so the check is the program. Coolify takes an image's HEALTHCHECK for a Dockerfile
# application, and its rolling update waits for it.
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --start-interval=2s --retries=3 \
    CMD ["/usr/local/bin/avalanche-master", "health", "--config", "/srv/avalanche-master/master.toml"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/avalanche-master"]
CMD ["run", "--config", "/srv/avalanche-master/master.toml", "--data", "/srv/avalanche-master/data"]
