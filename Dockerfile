FROM node:22-bookworm-slim AS web-build
WORKDIR /build/web
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1.98-bookworm AS rust-build
WORKDIR /build
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY migrations/ ./migrations/
COPY src/ ./src/
COPY --from=web-build /build/web/dist ./web/dist
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --create-home metered-usage \
    && install --directory --owner=10001 --group=10001 /data
COPY --from=rust-build /build/target/release/metered-usage /usr/local/bin/metered-usage
USER 10001
ENV DATABASE_URL=sqlite:/data/metered-usage.db
ENV BIND_ADDRESS=0.0.0.0:3000
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/metered-usage"]
