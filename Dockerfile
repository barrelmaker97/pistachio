FROM rust:1.87-slim AS builder
WORKDIR /app

# Build dependencies with empty main()
COPY ./Cargo.toml .
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

# Copy in src, touch file to set modified time, then build
COPY ./src src
RUN touch src/main.rs
RUN cargo build --release

# Copy binary to release image
FROM debian:12.10-slim
WORKDIR /app
COPY --from=builder /app/target/release/pistachio .
CMD ["./pistachio"]
