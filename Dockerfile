FROM rust:1.91-bookworm AS builder
RUN apt-get update
RUN apt-get install -y pkg-config libssl-dev ca-certificates libasound2-dev
WORKDIR /build
COPY . .
RUN cargo build --release --locked

FROM rust:1.91-slim-bookworm
COPY --from=builder /build/target/release/turbofuro_worker /turbofuro_worker

RUN adduser --no-create-home --disabled-login turbofuro_user
RUN chown turbofuro_user:turbofuro_user /turbofuro_worker

USER turbofuro_user
EXPOSE 4000
CMD ["/turbofuro_worker"]