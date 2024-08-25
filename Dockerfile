FROM rust:1.80-slim AS build

COPY ./Cargo.lock /tmp/Cargo.lock
COPY ./Cargo.toml /tmp/Cargo.toml
COPY ./src /tmp/src
WORKDIR /tmp

RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=build /tmp/target/release/nut-exporter .
CMD ["./nut-exporter"]
