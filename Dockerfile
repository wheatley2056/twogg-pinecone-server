# TwoGG Pinecone Server: Admin-Oberfläche (React-Admin) + Rust-Server in einem Image.
# Bauen:  docker build -t pinecone-server .

FROM node:22-bookworm-slim AS admin
WORKDIR /src/admin
COPY admin/package.json admin/package-lock.json ./
RUN npm ci
COPY admin/ ./
RUN npm run build

FROM rust:1.97-bookworm AS server
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/pinecone-server /usr/local/bin/pinecone-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data pinecone \
    && mkdir -p /data && chown pinecone /data
COPY --from=server /usr/local/bin/pinecone-server /usr/local/bin/pinecone-server
COPY --from=admin /src/admin/dist /app/admin
USER pinecone
ENV PINECONE_ADDR=0.0.0.0:8080 \
    PINECONE_DATA_DIR=/data \
    PINECONE_ADMIN_DIR=/app/admin \
    RUST_LOG=info
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["pinecone-server", "health"]
ENTRYPOINT ["pinecone-server"]
