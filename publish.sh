#!/bin/bash
docker build -t ievkz/cblt:latest . && \
docker build -t ievkz/cblt:0.0.6 . && \
docker push ievkz/cblt:latest && \
docker push ievkz/cblt:0.0.6
cargo publish -p cblt --allow-dirty
