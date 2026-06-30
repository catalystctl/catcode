# Multi-stage build for umans-harness.
# Stage 1 builds the Rust core + Go TUI in a full toolchain image.
# Stage 2 copies only the binaries + firejail into a slim runtime image.
# ponytail: no distroless, no scratch — a slim Debian keeps firejail + glibc
# working without heroic static-linking gymnastics.

# Base images are tag-pinned; pin each to a digest (@sha256:...) in CI for
# fully reproducible builds (e.g. rust:1.82-slim@sha256:<digest>). Keeping the
# digests in CI (not here) lets dependabot bump them in one place.
FROM rust:1.82-slim AS core-builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY core/ ./core/
RUN cd core && cargo build --release

FROM golang:1.24-slim AS tui-builder  # 1.24: tui/go.mod requires go 1.24.2 (hard min since Go 1.21); 1.23 silently pulled 1.24 via GOTOOLCHAIN (P0-6)
WORKDIR /build
COPY tui/ ./tui/
RUN cd tui && CGO_ENABLED=0 go build -o /tui .

FROM debian:bookworm-slim AS runtime
# firejail gives the --sandbox firejail core flag a real boundary; ca-certificates
# for TLS to the model API; util-linux for the unshare-based --no-network fallback.
# (curl removed: the entrypoint is an interactive TUI with no HTTP probe endpoint.)
RUN apt-get update && apt-get install -y --no-install-recommends \
    firejail ca-certificates util-linux && \
    rm -rf /var/lib/apt/lists/* && \
    useradd -m harness
WORKDIR /workspace
COPY --from=core-builder /build/core/target/release/core /usr/local/bin/umans-core
COPY --from=tui-builder /tui /usr/local/bin/umans-tui

# The core writes session/debug files here; the TUI reads settings here.
RUN mkdir -p /home/harness/.config/umans-harness/sessions && chown -R harness:harness /home/harness
USER harness
ENV UMANS_HARNESS_WORKSPACE=/workspace
# Production defaults wired through env (core/config.rs reads these).
#   SANDBOX=firejail  -> bash runs under a firejail profile (see --sandbox).
#   NO_NETWORK=0      -> network stays ON: the core must reach the model API.
#                        firejail isolates *bash* network separately; --no-network
#                        (unshare net) would also cut API access, so leave it off
#                        unless the API is reachable via a sidecar/proxy.
#   IDLE_TIMEOUT=120  -> seconds before an idle stream is treated as dead.
# NOTE on firejail in Docker (P1-20): firejail needs CAP_SYS_ADMIN + a user
# namespace + an unconfined AppArmor profile to create its sandbox. A plain
# `docker run` of this image as non-root (USER harness) CANNOT sandbox bash —
# firejail degrades/no-ops. To actually get isolation, run with:
#   docker run --cap-add SYS_ADMIN --security-opt apparmor=unconfined ...
# (or use gVisor / a VM). Without those flags, treat the container itself as the
# boundary and don't rely on --sandbox firejail inside it.
ENV UMANS_HARNESS_SANDBOX=firejail
ENV UMANS_HARNESS_NO_NETWORK=0
ENV UMANS_HARNESS_IDLE_TIMEOUT=120

# The TUI is the entry point; it spawns the core.
ENTRYPOINT ["/usr/local/bin/umans-tui"]
