FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget tzdata sqlite3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY solo-new-api-rust /app/solo-new-api-rust
COPY web-dist /app/web/dist

ENV PORT=3000 \
    DB_PATH=/data/one-api.db \
    JWT_SECRET_PATH=/data/.rust_jwt_secret \
    WEB_DIST=/app/web/dist \
    TRUST_PROXY_HEADERS=true \
    TZ=Asia/Shanghai

EXPOSE 3000
ENTRYPOINT ["/app/solo-new-api-rust"]
