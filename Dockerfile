FROM rust:1.82.0

WORKDIR /usr/src/app

COPY ./Cargo.toml .
COPY ./Cbltfile .
COPY ./src ./src
COPY ./assets ./assets

RUN cargo build --release

RUN cp /usr/src/app/target/release/cblt /usr/src/app/cblt

EXPOSE 80

CMD ["./cblt"]