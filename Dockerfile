# GHOST daemon — multi-stage Rust build with musl for a fully static binary.
# Static binary has zero GLIBC dependency — runs on any Linux regardless of distro.

FROM rust:slim AS builder

# musl-tools provides musl-gcc for static linking.
RUN apt-get update && apt-get install -y \
    pkg-config \
    musl-tools \
    && rm -rf /var/lib/apt/lists/*

# Add the musl target.
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app
COPY rust/ ./rust/
WORKDIR /app/rust

# Use musl-gcc as the C compiler for the musl target (needed by onig_sys / syntect).
ENV CC_x86_64_unknown_linux_musl=musl-gcc
# Skip compile-time sqlx query checks — no DB at build time.
ENV SQLX_OFFLINE=true

RUN cargo build --release -p rusty-claude-cli --target x86_64-unknown-linux-musl

# ---------------------------------------------------------------------------
# Stage 2: minimal runtime — bookworm-slim is fine because the binary is static.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy static binary. Migrations are embedded at compile time (sqlx::migrate!).
COPY --from=builder /app/rust/target/x86_64-unknown-linux-musl/release/claw /usr/local/bin/claw
COPY ghost-context.txt /app/ghost-context.txt
EXPOSE 8080

# HOST=0.0.0.0 and PORT are read from env vars (Railway injects PORT automatically).
CMD ["claw", "daemon", "--allow-unsafe-prompt"]
