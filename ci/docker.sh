#!/bin/sh
. ./ci/preamble.sh
image=ghcr.io/igankevich/random-dir-ci-2:latest
docker build --tag $image - <ci/Dockerfile
docker push $image
