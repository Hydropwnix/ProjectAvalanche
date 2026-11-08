# Project Avalanche master server in a container, built from this repository's source. Configured for Coolify as it
# is: plain HTTP on 8443 behind Coolify's proxy, which holds the certificate and is trusted by its container's name
# (coolify-proxy); the configuration is Engine/assets/master/deploy/master.coolify.toml.example, copied in below. On
# Coolify, a resource from this repository:
#   build pack "Docker Compose" (docker-compose.yaml beside this file): the domain on the service master, Deploy;
#   build pack "Dockerfile" (this file): Ports Exposes 8443, the domain, a Persistent Storage volume at
#   /srv/avalanche-master/data, Deploy.
# Elsewhere a master.toml mounted at /srv/avalanche-master/master.toml replaces the built-in one (direct TLS on a plain
# host: Engine/assets/master/deploy/compose.yaml). The build context is cut to what the build reads by
# Dockerfile.dockerignore beside this file (BuildKit).
# Rust 1.99: the version this image is built and tested with; the workspace's minimum is its
# `rust-version` (Engine/Cargo.toml), checked with `cargo check --workspace --all-targets` in a container
ARG BUILD_IMAGE=rust:1.99.0-slim-bookworm
ARG RUN_IMAGE=debian:bookworm-slim

FROM ${BUILD_IMAGE} AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked --manifest-path Engine/Cargo.toml -p avalanche-master

FROM ${RUN_IMAGE}
# uid and gid 10002 both: the host's deploy hook gives the TLS files this group (root owns them; the master reads them).
# The master's folder is root's (the master's user writes only data/, never its own configuration)
RUN groupadd --system --gid 10002 avalanche-master \
    && useradd --system --uid 10002 --gid 10002 --home-dir /srv/avalanche-master --shell /usr/sbin/nologin \
       avalanche-master \
    && install -d -o root -g avalanche-master -m 0750 /srv/avalanche-master \
    && install -d -o avalanche-master -g avalanche-master -m 0750 /srv/avalanche-master/data
COPY --from=build /src/Engine/target/release/avalanche-master /usr/local/bin/avalanche-master
COPY Engine/assets/master/THIRD_PARTY_NOTICES.txt /usr/share/doc/avalanche-master/THIRD_PARTY_NOTICES.txt
# the built-in configuration: root's in a root folder, so the master's user cannot change its own trust
COPY Engine/assets/master/deploy/master.coolify.toml.example /srv/avalanche-master/master.toml
USER avalanche-master
WORKDIR /srv/avalanche-master
# no VOLUME: Coolify removes a replaced container with `docker rm -f`, which keeps an anonymous volume,
# so each deployment would leave one behind; the data (the logs) is a named volume in docker-compose.yaml, and a
# Persistent Storage volume at /srv/avalanche-master/data with the Dockerfile build pack
EXPOSE 8443/tcp
# the master asks itself on its port (`avalanche-master health`: GET /v1/health from loopback, HTTPS with [tls]); the
# image has no shell tool, so the check is the program. Coolify takes an image's HEALTHCHECK for a Dockerfile
# application (its times read from this line), and its rolling update waits for it.
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --start-interval=2s --retries=3 \
    CMD ["/usr/local/bin/avalanche-master", "health", "--config", "/srv/avalanche-master/master.toml"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/avalanche-master"]
CMD ["run", "--config", "/srv/avalanche-master/master.toml", "--data", "/srv/avalanche-master/data"]
