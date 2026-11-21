# syntax=docker/dockerfile:1
FROM node:24-bookworm-slim AS web
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
COPY images/ /build/images/
RUN npm run build && npm test

FROM rust:1.100-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends zlib1g-dev pkg-config && rm -rf /var/lib/apt/lists/*
ARG BUILD_JOBS=4
ENV CC=gcc CXX=g++ CARGO_BUILD_JOBS=${BUILD_JOBS} RUST_TEST_THREADS=4
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY vendor ./vendor
COPY tests ./tests
COPY examples ./examples
COPY --from=web /build/web/dist ./web/dist
RUN cargo test --locked --all-targets && cargo build --locked --release

FROM debian:bookworm-slim AS runtime
ARG VERSION=0.1.0-beta.5
ARG REVISION=unknown
LABEL org.opencontainers.image.source="https://github.com/salnajjar/RustySoul" \
      org.opencontainers.image.title="RustySoul" \
      org.opencontainers.image.description="Soulseek daemon with a browser interface and API" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.licenses="GPL-3.0-only"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates zlib1g && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 rustysoul && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin rustysoul \
    && install -d -m 0700 -o rustysoul -g rustysoul /var/lib/rustysoul /downloads \
    && install -d -m 0755 -o rustysoul -g rustysoul /library
COPY --from=build /build/target/release/rustysoul /usr/local/bin/rustysoul
COPY deploy/container.toml /etc/rustysoul/rustysoul.toml
COPY LICENSE /usr/share/doc/rustysoul/LICENSE
COPY THIRD_PARTY_NOTICES.md /usr/share/doc/rustysoul/THIRD_PARTY_NOTICES.md
USER 10001:10001
ENV RUSTYSOUL_DEFAULT_USERNAME=admin RUSTYSOUL_DEFAULT_PASSWORD=change-me-now
WORKDIR /var/lib/rustysoul
EXPOSE 8484 2234
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 CMD ["rustysoul", "--config", "/etc/rustysoul/rustysoul.toml", "--healthcheck"]
ENTRYPOINT ["rustysoul"]
CMD ["--config", "/etc/rustysoul/rustysoul.toml"]
