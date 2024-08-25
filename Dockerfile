FROM rust:1.80-slim AS builder

WORKDIR /app

COPY ./Cargo.toml .
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

COPY ./src src
RUN touch src/main.rs
RUN cargo build --release
RUN strip target/release/nut-exporter

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/nut-exporter .
CMD ["./nut-exporter"]
