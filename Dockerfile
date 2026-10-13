FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev gcc
WORKDIR /src

# Build dependencies alone first, against stub sources, so this layer stays
# cached until Cargo.toml/Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
COPY crates/duckdns-core/Cargo.toml crates/duckdns-core/
COPY crates/duckdns-daemon/Cargo.toml crates/duckdns-daemon/
COPY crates/duckdns-tray/Cargo.toml crates/duckdns-tray/
RUN mkdir -p crates/duckdns-core/src crates/duckdns-daemon/src crates/duckdns-tray/src \
 && touch crates/duckdns-core/src/lib.rs \
 && echo 'fn main() {}' > crates/duckdns-daemon/src/main.rs \
 && echo 'fn main() {}' > crates/duckdns-tray/src/main.rs \
 && cargo build --release --locked -p duckdns-daemon

# Real sources. touch: COPY keeps old mtimes, which cargo would take as
# "older than the stub build" and skip recompiling.
COPY crates crates
RUN touch crates/duckdns-core/src/lib.rs crates/duckdns-daemon/src/main.rs \
 && cargo build --release --locked -p duckdns-daemon

# ---- runtime: nothing but the binary (TLS roots are compiled in) ----
FROM scratch
COPY --from=build /src/target/release/duckdns-daemon /duckdns-daemon
USER 65534:65534
ENTRYPOINT ["/duckdns-daemon"]
