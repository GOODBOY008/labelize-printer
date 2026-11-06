FROM rust:1.99.1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake pkg-config python3 curl clang libclang-dev \
    libssl-dev libfontconfig1-dev libfreetype-dev libx11-dev libxcb1-dev \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libasound2-dev \
    libxrandr-dev libxi-dev libxcursor-dev libxinerama-dev libxcomposite-dev \
    libvulkan-dev libcups2-dev libavahi-client-dev zlib1g-dev libudev-dev \
    libdbus-1-dev libgtk-3-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
