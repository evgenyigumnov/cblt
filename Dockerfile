# Use minimal Alpine image
FROM alpine:latest

# Install necessary packages, including curl, build dependencies, and OpenSSL static libraries
RUN apk update && \
    apk add --no-cache \
        curl \
        build-base \
        pkgconfig \
        openssl-dev \
        openssl-libs-static && \
    # Install Rust
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && \
    # Add Rust to PATH
    source $HOME/.cargo/env && \
    export PATH="$HOME/.cargo/bin:$PATH" && \
    # Create working directory
    mkdir -p /usr/src/app

WORKDIR /usr/src/app

# Copy project files
COPY ./Cargo.toml .
COPY ./Cbltfile .
COPY ./src ./src
COPY ./assets ./assets

# Build the project
RUN source $HOME/.cargo/env && \
    cargo build --release

# Copy the executable and remove unnecessary files
RUN cp /usr/src/app/target/release/cblt /usr/src/app/cblt && \
    rm -rf /usr/src/app/target && \
    rm -rf /usr/src/app/src && \
    rm -rf /usr/src/app/Cargo.toml && \
    # Remove Rust and all build dependencies
    apk del \
        curl \
        build-base \
        pkgconfig \
        openssl-dev \
        openssl-libs-static && \
    rm -rf $HOME/.cargo && \
    rm -rf /root/.rustup && \
    rm -rf /root/.cargo && \
    rm -rf /usr/local/cargo

# Expose ports
EXPOSE 80
EXPOSE 443

# Command to run the application
CMD ["./cblt"]
