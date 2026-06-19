# ── Stage 1: build ────────────────────────────────────────────────────────────
FROM rust:slim-bookworm AS builder

# Raise cargo's per-request HTTP timeout (default 30 s is too short on slow
# or proxied networks when downloading the crates.io index).
ENV CARGO_HTTP_TIMEOUT=300

WORKDIR /build

# Workspace manifests first — lets Docker cache the (slow) dep-compilation
# layer independently of source changes.
COPY Cargo.toml Cargo.lock ./
# rust-toolchain.toml is intentionally omitted: the rust:slim-bookworm image
# ships the stable toolchain already. Copying it causes rustup to reach out to
# static.rust-lang.org to sync the channel manifest, which fails in restricted
# or slow network environments.
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

RUN cargo build --release -p knowledge-server \
 && cargo build --release -p agent

# Replace stubs with real source — only user code is recompiled from here.
RUN rm -f knowledge-server/src/lib.rs knowledge-server/src/main.rs \
          agent/src/main.rs
COPY knowledge-server/src    knowledge-server/src
COPY knowledge-server/skills knowledge-server/skills
COPY agent/src               agent/src
RUN touch knowledge-server/src/lib.rs knowledge-server/src/main.rs \
          agent/src/main.rs \
 && cargo build --release -p knowledge-server \
 && cargo build --release -p agent

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
FROM ubuntu:26.04

# ca-certificates: TLS to LLM APIs / OIDC providers.
# curl: used by the Docker healthcheck.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/knowledge-server /usr/local/bin/knowledge-server
COPY --from=builder /build/target/release/harvest-agent    /usr/local/bin/harvest-agent
COPY knowledge-server/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["docker-entrypoint.sh"]
