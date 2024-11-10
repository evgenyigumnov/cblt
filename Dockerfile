FROM rust:alpine  as builder

RUN apk update && \
    apk add --no-cache \
        curl \
        build-base \
        pkgconfig \
        openssl-dev \
        openssl-libs-static

WORKDIR /usr/src/app

COPY ./Cargo.toml .
COPY ./src ./src

RUN cargo build --release

CMD ["./cblt"]

FROM alpine:latest

RUN apk add --no-cache openssl

RUN mkdir /cblt
RUN mkdir /cblt/etc
RUN mkdir /cblt/assets
COPY --from=builder /usr/src/app/target/release/cblt /cblt/cblt

WORKDIR /cblt

COPY ./assets ./assets
COPY ./Cbltfile ./etc/Cbltfile

EXPOSE 80
EXPOSE 443

CMD ["./cblt", "--cfg", "./etc/Cbltfile"]
