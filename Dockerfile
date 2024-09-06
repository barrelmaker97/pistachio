FROM rust:1.81-slim AS builder

WORKDIR /app

COPY ./Cargo.toml .
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

COPY ./src src
RUN touch src/main.rs
RUN cargo build --release
RUN strip target/release/pistachio

FROM debian:12.7-slim
WORKDIR /app
COPY --from=builder /app/target/release/pistachio .
CMD ["./pistachio"]
