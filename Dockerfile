FROM node:22-bookworm-slim AS ui
WORKDIR /build/src/web/frontend
COPY src/web/frontend/package.json src/web/frontend/package-lock.json ./
RUN npm ci
COPY src/web/frontend/ .
RUN npm run build

FROM rust:1.87-bookworm AS rust-builder

WORKDIR /build
COPY src/Cargo.toml src/Cargo.lock ./src/
COPY src/crates ./src/crates/
COPY src/config ./src/config/
RUN cargo build --manifest-path src/Cargo.toml -p inferra-cli --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 1000 --create-home inferra
RUN mkdir -p /data && chown inferra:inferra /data

WORKDIR /app
COPY --from=rust-builder /build/src/target/release/inferra /app/inferra
COPY --from=ui /build/src/web/ui_dist /app/runtime-assets/ui_dist
COPY deploy/docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/inferra /app/docker-entrypoint.sh && ln -sf /app/inferra /usr/local/bin/inferra

USER inferra
WORKDIR /home/inferra

EXPOSE 7433

ENV INFERRA_CONFIG=/etc/inferra/inferra.toml

HEALTHCHECK --interval=30s --timeout=3s --start-period=20s --retries=3 CMD curl -fsS http://127.0.0.1:7433/healthz || exit 1

CMD ["/app/docker-entrypoint.sh"]
