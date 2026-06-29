# Multi-stage build for umans-harness.
# Stage 1 builds the Rust core + Go TUI in a full toolchain image.
# Stage 2 copies only the binaries + firejail into a slim runtime image.
# ponytail: no distroless, no scratch — a slim Debian keeps firejail + glibc
# working without heroic static-linking gymnastics.

FROM rust:1.82-slim AS core-builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY core/ ./core/
RUN cd core && cargo build --release

FROM golang:1.23-slim AS tui-builder
WORKDIR /build
COPY tui/ ./tui/
RUN cd tui && CGO_ENABLED=0 go build -o /tui .

FROM debian:bookworm-slim AS runtime
# firejail gives the --sandbox firejail core flag a real boundary; ca-certificates
# for TLS to the model API; curl for a healthcheck.
RUN apt-get update && apt-get install -y --no-install-recommends \
    firejail ca-certificates curl util-linux && \
    rm -rf /var/lib/apt/lists/* && \
    useradd -m harness
WORKDIR /workspace
COPY --from=core-builder /build/core/target/release/core /usr/local/bin/umans-core
COPY --from=tui-builder /tui /usr/local/bin/umans-tui

# The core writes session/debug files here; the TUI reads settings here.
RUN mkdir -p /home/harness/.config/umans-harness/sessions && chown -R harness:harness /home/harness
USER harness
ENV UMANS_HARNESS_WORKSPACE=/workspace
# Sensible production defaults: sandbox bash, block network, higher idle timeout.
ENV UMANS_HARNESS_SANDBOX=firejail
ENV UMANS_HARNESS_NO_NETWORK=0
ENV UMANS_HARNESS_IDLE_TIMEOUT=120

# The TUI is the entry point; it spawns the core.
ENTRYPOINT ["/usr/local/bin/umans-tui"]
