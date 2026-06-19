# ── Stage 1: build ────────────────────────────────────────────────────────────
FROM rust:slim-bookworm AS builder

WORKDIR /build

# Workspace manifests first — lets Docker cache the (slow) dep-compilation
# layer independently of source changes.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY knowledge-server/Cargo.toml    knowledge-server/Cargo.toml
COPY knowledge-harvester/Cargo.toml knowledge-harvester/Cargo.toml
COPY agent/Cargo.toml               agent/Cargo.toml

# Minimal stubs so cargo can resolve and compile all external dependencies.
# knowledge-server is a lib + bin; the other two are pure binaries.
RUN mkdir -p knowledge-server/src knowledge-harvester/src agent/src \
 && echo '' > knowledge-server/src/lib.rs \
 && printf 'fn main(){}\n' | tee knowledge-server/src/main.rs \
                                  knowledge-harvester/src/main.rs \
                                  agent/src/main.rs > /dev/null

RUN cargo build --release -p knowledge-server

# Replace stubs with real source — only user code is recompiled from here.
RUN rm -f knowledge-server/src/lib.rs knowledge-server/src/main.rs
COPY knowledge-server/src    knowledge-server/src
COPY knowledge-server/skills knowledge-server/skills
RUN touch knowledge-server/src/lib.rs knowledge-server/src/main.rs \
 && cargo build --release -p knowledge-server

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

# ca-certificates: TLS to LLM APIs / OIDC providers.
# curl: used by the Docker healthcheck.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/knowledge-server /usr/local/bin/knowledge-server
COPY knowledge-server/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["docker-entrypoint.sh"]
