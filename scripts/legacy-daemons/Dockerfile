# One old Transmission daemon, from the distribution package of the base image.
# Built by scripts/legacy-daemons.sh with BASE_IMAGE set to debian:12, ubuntu:24.04 or debian:13.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
RUN apt-get update \
    && apt-get install --no-install-recommends --yes transmission-daemon transmission-cli \
    && rm -rf /var/lib/apt/lists/*
