# syntax=docker/dockerfile:1
#
# catcode-sandbox — the default Microsandbox image for Catalyst Code.
#
# Multi-arch (amd64 + arm64) polyglot developer image. CatCode boots this OCI
# image as the guest root filesystem when sandboxing is enabled, and runs
# agent-controlled workloads (bash, git, diagnostics, plugin scripts, …)
# inside the resulting microVM.
#
# Build / publish
# ---------------
#   docker buildx build \
#     --platform linux/amd64,linux/arm64 \
#     -t ghcr.io/catalystctl/catcode-sandbox:0.1 \
#     -f sandbox/Dockerfile .
#
# Tag immutably by digest for releases:
#   docker buildx build ... --output type=image,push=true,name=ghcr.io/catalystctl/catcode-sandbox:0.1
#   # then capture the resolved digest:
#   docker buildx imagetools inspect ghcr.io/catalystctl/catcode-sandbox:0.1
#
# Image update process
# --------------------
# 1. Bump the version tag below (CATCODE_SANDBOX_IMAGE_VERSION) and any pinned
#    toolchain versions as needed.
# 2. Rebuild for both platforms and push to GHCR.
# 3. Resolve and record the multi-arch digest:
#      docker buildx imagetools inspect ghcr.io/catalystctl/catcode-sandbox:<ver>
# 4. Update the default `sandbox_image` reference in core + docs to the pinned
#    tag (and, for production releases, the @sha256 digest).
# 5. Each CatCode release records the resolved image digest used by that
#    release in its release notes so builds are reproducible.
#
# License: this image bundles open-source components (Debian, rustup/ crates,
# NodeSource Node.js, Go, etc.). See THIRD_PARTY_NOTICES.md and the license
# texts inside the image's /usr/share/doc directories.

ARG DEBIAN_TAG=bookworm-20250113-slim

FROM debian:${DEBIAN_TAG} AS base
ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive
# Minimal apt: ca-certificates first so the toolchain installers can use TLS.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl xz-utils gnupg2 \
 && rm -rf /var/lib/apt/lists/*

# ---------------------------------------------------------------------------
# Stage: system packages — the tool list the sandbox contract guarantees.
# ---------------------------------------------------------------------------
FROM base AS system
ARG TARGETARCH

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash \
      coreutils \
      findutils \
      grep \
      sed \
      gawk \
      git \
      curl \
      ca-certificates \
      jq \
      ripgrep \
      tar \
      gzip \
      xz-utils \
      unzip \
      build-essential \
      pkg-config \
      libssl-dev \
      openssh-client \
      less \
      file \
      procps \
      locales \
 && rm -rf /var/lib/apt/lists/* \
 && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen && locale-gen
ENV LANG=en_US.UTF-8 \
    LC_ALL=en_US.UTF-8

# ---------------------------------------------------------------------------
# Stage: Rust toolchain (via rustup). Pinned to a stable channel date.
# Multi-arch: rustup auto-detects the target via TARGETARCH.
# ---------------------------------------------------------------------------
FROM system AS rust
ARG RUSTUP_VERSION=1.27.1
# Pin a known-good stable release. rustup installs the host's default toolchain.
ARG RUST_TOOLCHAIN=1.82.0

RUN case "$TARGETARCH" in \
      amd64)  RUSTUP_HOST=x86_64-unknown-linux-gnu  ;; \
      arm64)  RUSTUP_HOST=aarch64-unknown-linux-gnu ;; \
      *) echo "unsupported arch: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${RUSTUP_HOST}/rustup-init" \
      -o /tmp/rustup-init; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --profile minimal --default-toolchain "${RUST_TOOLCHAIN}" --default-host "${RUSTUP_HOST}"; \
    rm /tmp/rustup-init
ENV PATH=/root/.cargo/bin:${PATH}
# A writable CARGO_HOME for per-session package installs.
ENV CARGO_HOME=/root/.cargo \
    RUSTUP_HOME=/root/.rustup

# ---------------------------------------------------------------------------
# Stage: Node.js (via NodeSource). Pinned to an LTS line.
# ---------------------------------------------------------------------------
FROM rust AS node
ARG NODE_MAJOR=20

RUN curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && rm -rf /var/lib/apt/lists/* \
 && node --version && npm --version

# ---------------------------------------------------------------------------
# Stage: Python.
# ---------------------------------------------------------------------------
FROM node AS python
# bookworm ships Python 3.11. Install pip + venv so `diagnostics` (py_compile)
# and pip-based flows work.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      python3 python3-pip python3-venv python3-dev \
 && rm -rf /var/lib/apt/lists/* \
 && python3 --version

# ---------------------------------------------------------------------------
# Stage: Go toolchain. Pinned to a stable release.
# ---------------------------------------------------------------------------
FROM python AS go
ARG GO_VERSION=1.24.2

RUN case "$TARGETARCH" in \
      amd64) GOARCH=amd64 ;; \
      arm64) GOARCH=arm64 ;; \
      *) echo "unsupported arch: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-${GOARCH}.tar.gz" \
      -o /tmp/go.tgz; \
    tar -C /usr/local -xzf /tmp/go.tgz; \
    rm /tmp/go.tgz
ENV PATH=/usr/local/go/bin:${PATH}
ENV GOPATH=/root/go
ENV PATH=${GOPATH}/bin:${PATH}
RUN go version

# ---------------------------------------------------------------------------
# Final image.
# ---------------------------------------------------------------------------
FROM go AS final
ARG CATCODE_SANDBOX_IMAGE_VERSION=0.1
LABEL org.opencontainers.image.title="catcode-sandbox" \
      org.opencontainers.image.description="Default Microsandbox guest image for Catalyst Code (polyglot developer image)" \
      org.opencontainers.image.source="https://github.com/catalystctl/catcode" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${CATCODE_SANDBOX_IMAGE_VERSION}"

# The guest runs as a non-root user where the SDK supports it. /workspace is
# the mounted, writable host workspace; /home/catcode is a private guest home.
RUN useradd --create-home --shell /bin/bash catcode \
 && mkdir -p /workspace \
 && chown catcode:catcode /workspace
ENV HOME=/home/catcode

# Sanity-check that the contract tool set is present and on PATH.
RUN for bin in bash find grep sed awk git curl jq rg tar gzip xz unzip cargo rustc node npm python3 go; \
    do command -v "$bin" >/dev/null 2>&1 || { echo "missing: $bin" >&2; exit 1; }; done

USER catcode
WORKDIR /workspace
