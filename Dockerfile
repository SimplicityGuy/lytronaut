# syntax=docker/dockerfile:1
# Build independently inside each target Linux architecture (amd64 may use emulation).
FROM rust:1.99.0-bookworm@sha256:114c7a4425406451c2866b6aafe69fe29b1b298832db1277d411ac73c82d04d6 AS build
WORKDIR /src
ENV CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
RUN rustup target add wasm32-unknown-unknown && cargo install --locked wasm-bindgen-cli --version 0.2.129
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY assets ./assets
COPY .cargo ./.cargo
RUN cargo build --locked --release -p lytronaut-service && cargo build --locked --release -p lytronaut-client --target wasm32-unknown-unknown && wasm-bindgen --target web --out-dir /client target/wasm32-unknown-unknown/release/lytronaut_client.wasm
FROM scratch AS validation-artifacts
# Export precisely the tested build products without retaining compiler/cache layers.
COPY --from=build /src/target/release/lytronaut-service /native/lytronaut-service
COPY --from=build /src/target/wasm32-unknown-unknown/release/lytronaut_client.wasm /raw/lytronaut_client.wasm
COPY --from=build /client /client
COPY --from=build /src/Cargo.lock /Cargo.lock
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS runtime
ARG SOURCE_REVISION=unknown
LABEL org.opencontainers.image.source="https://github.com/SimplicityGuy/lytronaut" org.opencontainers.image.revision="${SOURCE_REVISION}"
# No interpreter, media process, browser, GPU driver or compiler is installed at runtime.
RUN groupadd --gid 10001 lytronaut && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin lytronaut && mkdir -p /data/catalog /data/originals /data/work /app/client && chown -R 10001:10001 /data && chmod 700 /data/catalog /data/originals /data/work
COPY --from=build /src/target/release/lytronaut-service /app/lytronaut-service
COPY --from=build /client /app/client
COPY LICENSE THIRD_PARTY_NOTICES.md DEPENDENCY_LICENSES.md Cargo.lock /app/notices/
USER 10001:10001
WORKDIR /app
ENV LYTRONAUT_LISTEN=0.0.0.0:3000 LYTRONAUT_ORIGIN=http://127.0.0.1:3000 LYTRONAUT_DATA=/data/catalog LYTRONAUT_ORIGINALS=/data/originals LYTRONAUT_STAGING=/data/work/staging LYTRONAUT_CLIENT=/app/client
EXPOSE 3000
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 CMD ["/app/lytronaut-service", "--healthcheck"]
ENTRYPOINT ["/app/lytronaut-service"]
