# syntax=docker/dockerfile:1

# Build stage: compile fully static musl binaries.
#
# Pinned to the crate's MSRV; combined with the committed Cargo.lock this makes
# the build reproducible (same compiler + same dependency versions).
#
# Targets x86_64 musl so the binaries are statically linked and depend on no
# system libraries — letting the final image be `scratch` (just the binaries).
FROM rust:1.92-slim-bookworm AS builder
WORKDIR /src

RUN rustup target add x86_64-unknown-linux-musl

# The SQLite driver bundles C, so a musl C toolchain is required despite the rest
# of the tree being pure Rust. `cc` looks for the target-prefixed name first.
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# Build with cached cargo registry and target directories for fast rebuilds. The
# target dir is a cache mount (not persisted in the layer), so copy the finished
# binaries out to /out where the final stage can pick them up.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --target x86_64-unknown-linux-musl -p rpt-cli -p rpt-render-cli \
    && mkdir -p /out \
    && cp target/x86_64-unknown-linux-musl/release/rpt \
          target/x86_64-unknown-linux-musl/release/rpt-render /out/

# Runtime stage: nothing but the binaries.
FROM scratch AS runtime

# Static binaries, so no libc, shell, or package manager is needed.
COPY --from=builder /out/rpt /out/rpt-render /usr/local/bin/

# The binaries link every dependency and embed the bundled fonts, so the image redistributes them
# and owes their notices. `scratch` has no shell to read these with — they are here so that whoever
# redistributes the image can extract them (`docker cp`) and satisfy the same conditions.
COPY --from=builder /src/LICENSE /src/THIRD-PARTY-NOTICES.md /usr/local/share/licenses/rpt-rs/

# MPL-2.0 §3.2(a) asks that a recipient of the executable form be told how to obtain the source. The
# image carries no README to say so, so the label says it.
LABEL org.opencontainers.image.source="https://github.com/MrSrsen/rpt-rs" \
      org.opencontainers.image.licenses="MPL-2.0"

ENV PATH=/usr/local/bin
WORKDIR /data
USER 10001:10001

# `rpt` is the inspection/export CLI; `rpt-render` renders a report. Override the
# command to run either, e.g.:
#   docker run --rm -v "$PWD:/data" IMAGE rpt inspect report.rpt
#   docker run --rm -v "$PWD:/data" IMAGE rpt json-dump report.rpt out.json
#   docker run --rm -v "$PWD:/data" IMAGE rpt-render report.rpt -o out.pdf
CMD ["rpt", "--help"]
