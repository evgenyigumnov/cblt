#!/bin/bash

VERSION=$(grep '^version =' Cargo.toml | sed -E 's/version = "(.+)"/\1/')

if [ -z "$VERSION" ]; then
    echo "Cant extract version from Cargo.toml"
    exit 1
fi

docker build -t ievkz/cblt:latest . && \
docker build -t ievkz/cblt:$VERSION . && \
docker push ievkz/cblt:latest && \
docker push ievkz/cblt:$VERSION

cargo publish -p cblt --allow-dirty

echo "Published: $VERSION"
