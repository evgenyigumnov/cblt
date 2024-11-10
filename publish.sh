#!/bin/bash
docker build -t ievkz/cblt:latest . && \
docker build -t ievkz/cblt:0.0.4 . && \
docker push ievkz/cblt:latest && \
docker push ievkz/cblt:0.0.4