FROM ghcr.io/cross-rs/aarch64-unknown-linux-gnu:0.2.5
RUN dpkg --add-architecture arm64 && \
    apt-get update && \
    apt-get install -y libasound2-dev:arm64 libssl-dev:arm64 libusb-1.0-0-dev:arm64 libudev-dev:arm64
