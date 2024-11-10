#!/bin/bash
docker build -t ievkz/cblt:latest . && \
docker build -t ievkz/cblt:0.0.8 . && \
docker push ievkz/cblt:latest && \
docker push ievkz/cblt:0.0.8
cargo publish -p cblt --allow-dirty
