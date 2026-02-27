FROM rust:1.85-bookworm

ARG BUN_VERSION=1.2.9

RUN apt-get update && apt-get install -y --no-install-recommends \
  ca-certificates \
  curl \
  git \
  pkg-config \
  libssl-dev \
  libusb-1.0-0-dev \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  librsvg2-bin \
  xz-utils \
  make \
  gcc \
  g++ \
  mingw-w64 \
  gcc-mingw-w64-x86-64 \
  g++-mingw-w64-x86-64 \
  && rm -rf /var/lib/apt/lists/*

RUN rustup target add \
  aarch64-unknown-linux-gnu \
  x86_64-pc-windows-gnu

ENV BUN_INSTALL=/usr/local/bun
RUN curl -fsSL https://bun.sh/install | bash -s -- bun-v${BUN_VERSION}

ENV PATH="${BUN_INSTALL}/bin:${PATH}"
WORKDIR /workspace
