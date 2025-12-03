# Start with a rust alpine image
FROM rust:1-alpine3.21 AS builder
# This is important, see https://github.com/rust-lang/docker-rust/issues/85
ENV RUSTFLAGS="-C target-feature=-crt-static"
# if needed, add additional dependencies here
RUN apk add --no-cache musl-dev
# set the workdir and copy the source into it
WORKDIR /app
COPY ./Cargo.toml /app
COPY ./Cargo.lock /app
COPY rust-toolchain.toml /app
COPY ./src/cache_helper.rs /app/src/cache_helper.rs

RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release --bin cache

COPY ./src /app/src
# do a release build
RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release --bin server

# use a plain alpine image, the alpine version needs to match the builder
FROM alpine:3.21
# if needed, install additional dependencies here
RUN apk add --no-cache libgcc
# copy the binary into the final image
COPY --from=builder /app/target/release/server server
# set the binary as entrypoint
ENTRYPOINT ["/server"]

EXPOSE 9090
