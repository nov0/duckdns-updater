# ---- build: static musl binary ----
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev gcc
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p duckdns-daemon

# ---- runtime: nothing but the binary (TLS roots are compiled in) ----
FROM scratch
COPY --from=build /src/target/release/duckdns-daemon /duckdns-daemon
USER 65534:65534
ENTRYPOINT ["/duckdns-daemon"]
