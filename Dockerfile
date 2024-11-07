FROM rust:1.82.0

WORKDIR /usr/src/app

COPY ./Cargo.toml .
COPY ./Cbltfile .
COPY ./src ./src
COPY ./logo.png ./logo.png
COPY ./logo_huge.png ./logo_huge.png

RUN cargo build --release

WORKDIR /usr/src/app

RUN cp /usr/src/app/target/release/cblt /usr/src/app/cblt

EXPOSE 80

CMD ["./cblt"]