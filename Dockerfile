# The `failscope` binary (ingest, serve, alert). Off-chain workspace only.
#   docker build -t failscope .
FROM rust:1.94.0-bookworm AS build
# Use the image's toolchain as-is instead of letting rustup fetch the
# rustfmt/clippy components rust-toolchain.toml lists.
ENV RUSTUP_TOOLCHAIN=1.94.0
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p failscope-api --bin failscope \
    && cp target/release/failscope /usr/local/bin/failscope

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home failscope
COPY --from=build /usr/local/bin/failscope /usr/local/bin/failscope
USER failscope
ENTRYPOINT ["failscope"]
