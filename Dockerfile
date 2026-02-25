# Stage 1: Chef — compute dependency recipe
FROM rust:1.85-slim AS chef
RUN cargo install cargo-chef
WORKDIR /app

# Stage 2: Planner — generate recipe.json
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder — build dependencies then source
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies (cached unless Cargo.toml/lock changes)
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin discord-assist

# Stage 4: Runtime — minimal image
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as non-root
RUN useradd -m -u 1000 bot
USER bot
WORKDIR /app

COPY --from=builder /app/target/release/discord-assist /app/discord-assist

ENV CONFIG_PATH=/app/config.toml
ENV RUST_LOG=discord_assist=info

ENTRYPOINT ["/app/discord-assist"]
