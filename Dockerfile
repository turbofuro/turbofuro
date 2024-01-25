FROM rust:1.75-slim-buster as builder
WORKDIR /build
COPY . .
RUN cargo build --release --locked

FROM rust:1.75-slim-buster
RUN apt-get update
RUN apt-get install -y openssl ca-certificates
COPY --from=builder /build/target/release/turbofuro_worker /turbofuro_worker

RUN adduser --no-create-home --disabled-login turbofuro_user
RUN chown turbofuro_user:turbofuro_user /turbofuro_worker

USER turbofuro_user
EXPOSE 4000
CMD ["/turbofuro_worker"]