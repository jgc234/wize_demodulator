FROM rust:1.96-trixie AS builder

RUN rustup target add x86_64-unknown-linux-musl
RUN apt-get update && apt-get install -y musl-tools

WORKDIR /app

COPY . .

RUN cargo build \
    --release \
    --target x86_64-unknown-linux-musl

FROM scratch

COPY --from=builder \
    /app/target/x86_64-unknown-linux-musl/release/rtl_pipeline \
    /rtl_pipeline

ENTRYPOINT ["/rtl_pipeline"]