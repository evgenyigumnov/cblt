# Используем минимальный образ Alpine
FROM alpine:latest

# Устанавливаем необходимые пакеты, включая curl и зависимости для сборки Rust
RUN apk update && \
    apk add --no-cache \
    curl \
    build-base \
    pkgconfig \
    openssl-dev && \
    # Устанавливаем Rust
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && \
    # Добавляем Rust в PATH
    source $HOME/.cargo/env && \
    export PATH="$HOME/.cargo/bin:$PATH" && \
    # Создаем рабочую директорию
    mkdir -p /usr/src/app

WORKDIR /usr/src/app

# Копируем файлы проекта
COPY ./Cargo.toml .
COPY ./Cbltfile .
COPY ./src ./src
COPY ./assets ./assets

# Строим проект
RUN source $HOME/.cargo/env && \
    cargo build --release

# Копируем исполняемый файл и удаляем ненужные файлы
RUN cp /usr/src/app/target/release/cblt /usr/src/app/cblt && \
    rm -rf /usr/src/app/target && \
    rm -rf /usr/src/app/src && \
    rm -rf /usr/src/app/Cargo.toml && \
    # Удаляем Rust и все зависимости для сборки
    apk del \
    curl \
    build-base \
    pkgconfig \
    openssl-dev && \
    rm -rf $HOME/.cargo && \
    rm -rf /root/.rustup && \
    rm -rf /root/.cargo && \
    rm -rf /usr/local/cargo

# Указываем открытые порты
EXPOSE 80
EXPOSE 443

# Команда для запуска приложения
CMD ["./cblt"]