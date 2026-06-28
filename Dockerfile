# syntax=docker/dockerfile:1

# Build stage: compile fully static musl binaries.
#
# Pinned to the crate's MSRV; combined with the committed Cargo.lock this makes
# the build reproducible (same compiler + same dependency versions).
#
# Targets x86_64 musl so the binaries are statically linked and depend on no
# system libraries — letting the final image be `scratch` (just the binaries).
FROM rust:1.85-slim-bookworm AS builder
WORKDIR /src

RUN rustup target add x86_64-unknown-linux-musl

# The whole workspace. Dependencies are pure-Rust, so no C toolchain is needed.
COPY . .

# Build with cached cargo registry and target directories for fast rebuilds. The
# target dir is a cache mount (not persisted in the layer), so copy the finished
# binaries out to /out where the final stage can pick them up.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --target x86_64-unknown-linux-musl -p rpt-cli -p rpt-to-xml \
    && mkdir -p /out \
    && cp target/x86_64-unknown-linux-musl/release/rpt \
          target/x86_64-unknown-linux-musl/release/rpt-to-xml /out/

# Runtime stage: nothing but the binaries.
FROM scratch AS runtime

# Static binaries, so no libc, shell, or package manager is needed.
COPY --from=builder /out/rpt /out/rpt-to-xml /usr/local/bin/

ENV PATH=/usr/local/bin
WORKDIR /data
USER 10001:10001

# `rpt` is the inspection CLI; `rpt-to-xml` is also on PATH. Override the command
# to run either, e.g.:
#   docker run --rm -v "$PWD:/data" IMAGE rpt inspect report.rpt
#   docker run --rm -v "$PWD:/data" IMAGE rpt-to-xml report.rpt out.xml
CMD ["rpt", "--help"]
