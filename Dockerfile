# syntax=docker/dockerfile:1
FROM rust:1.87-alpine3.21 AS builder
RUN apk add --no-cache build-base git
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY fixtures/ fixtures/
COPY migrations/ migrations/
RUN cargo build --release --locked --bin csb \
    && strip target/release/csb

FROM alpine:3.21
RUN apk add --no-cache git ca-certificates
COPY --from=builder /build/target/release/csb /usr/local/bin/csb
RUN set -eux; \
    git init -q /tmp/smoke; \
    cd /tmp/smoke; \
    git config user.email "smoke@example.invalid"; \
    git config user.name "smoke"; \
    printf 'fn main() {}\n' > main.rs; \
    git add .; \
    git commit -qm one; \
    printf 'fn main() { println!("hi"); }\n' > main.rs; \
    git add .; \
    git commit -qm two; \
    csb review --repo /tmp/smoke \
      --base "$(git rev-parse HEAD~1)" \
      --head "$(git rev-parse HEAD)" \
      --out-dir /tmp/out --summary check; \
    csb verify --dir /tmp/out; \
    rm -rf /tmp/smoke /tmp/out
WORKDIR /repo
ENTRYPOINT ["csb"]
CMD ["help"]
