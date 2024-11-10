FROM rust:1.82.0

WORKDIR /usr/src/app

COPY ./Cargo.toml .
COPY ./Cbltfile .
COPY ./src ./src
COPY ./assets ./assets

RUN cargo build --release

RUN cp /usr/src/app/target/release/cblt /usr/src/app/cblt
RUN rm -rf /usr/src/app/target

EXPOSE 80
EXPOSE 443

CMD ["./cblt"]