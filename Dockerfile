# GHOST daemon — multi-stage Rust build.
# Stage 1: compile with the full Rust toolchain.
# Stage 2: copy the single binary into a slim Debian image.

FROM rust:1.85-slim AS builder

# System deps needed to link sqlx with rustls-tls (no OpenSSL required for rustls).
RUN apt-get update && apt-get install -y \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the entire rust workspace.
COPY rust/ ./rust/

WORKDIR /app/rust

# sqlx offline mode: skip compile-time query checking since we have no DB at build time.
# Migrations run at daemon startup via sqlx::migrate::Migrator.
ENV SQLX_OFFLINE=true

RUN cargo build --release -p rusty-claude-cli

# ---------------------------------------------------------------------------
# Stage 2: minimal runtime image.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary. Migrations are embedded at compile time (sqlx::migrate!).
COPY --from=builder /app/rust/target/release/claw /usr/local/bin/claw

# Railway injects PORT; daemon reads it from the PORT env var.
# Expose a representative default for documentation — actual port is dynamic.
EXPOSE 8080

# Start the daemon. HOST=0.0.0.0 binds all interfaces (required on Railway).
# --allow-unsafe-prompt enables POST /prompt; auth is enforced via GHOST_DAEMON_KEY.
CMD ["claw", "daemon", "--allow-unsafe-prompt"]
